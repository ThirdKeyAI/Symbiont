#!/usr/bin/env python3
"""Installed-CLI outbound workflow with synthetic credentials and local services.

Exercises real manifest loading, policy, terminal approval, credential injection,
signed journals, cancellation/retry inspection and refusal paths. It makes no
external provider calls and does not claim to qualify a worker sandbox.
"""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import pty
import re
import runpy
import select
import signal
import subprocess
import tempfile
import termios
import threading
import time
import uuid


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    require(__debug__, "Run without Python optimization; independent verifier requires assertions")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    require(not args.report.exists(), "Preserve previous reports; choose a new path")
    binary = args.binary.resolve(strict=True)
    source = Path(__file__).resolve().parents[1]
    verifier = runpy.run_path(str(source / "scripts/test-direct-inference.py"))["verify_journal"]
    source_identity = runpy.run_path(str(source / "scripts/check-security-controls.py"))["source_identity"]
    report = dict(schema_version=1, passed=False, timestamp=datetime.now(timezone.utc).isoformat(),
                  source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip(),
                  source_digest=source_identity(source), binary_sha256=digest(binary),
                  driver_sha256=digest(__file__), verifier_sha256=digest(source / "scripts/test-direct-inference.py"),
                  platform=dict(os=platform.system(), release=platform.release(), architecture=platform.machine()),
                  scope="governed ToolClad HTTP through installed CLI; no worker isolation qualification",
                  routes=["cli_toolclad_http"], backends=["runtime_http", "protected_journal"],
                  control_ids=["OUT-001", "OUT-002", "OUT-003", "AUD-001"],
                  checks=[], errors=[], configurations={}, requests=[], journals=[], timings_seconds={})
    secret = "synthetic-outbound-" + uuid.uuid4().hex
    inference_secret = "synthetic-inference-" + uuid.uuid4().hex
    mode = "read"
    fixture_errors = []
    provider_calls = []
    receipts = []
    accepted = threading.Event()
    release = threading.Event()
    children = []

    class API(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self):
            try:
                size = int(self.headers.get("Content-Length", "0"))
                require(size <= 4096, "oversized fixture request")
                body = self.rfile.read(size)
                require(self.headers.get("Authorization") == "Bearer " + secret, "wrong injected credential")
                expected_path = "/records/7%3Bv%3D1/admin" if mode == "path_parameter_read" else "/records/7"
                require(self.path == expected_path, "unexpected API destination")
                receipts.append(dict(method=self.command, path=self.path, body=body.decode(), credential_matched=True))
                status = 200
                result = dict(record=7, status="reviewed" if self.command == "POST" else "pending")
                if mode == "reflection":
                    result = dict(reflected=secret)
                if mode == "redirect":
                    status = 302
                    result = dict(error="upstream detail must not reach the agent")
                if mode == "interrupt":
                    accepted.set()
                    require(release.wait(20), "interruption fixture release timeout")
                payload = json.dumps(result).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                if status == 302:
                    self.send_header("Location", "/forbidden")
                if mode == "header_reflection":
                    self.send_header("X-Reflected", secret)
                self.end_headers()
                try:
                    self.wfile.write(payload)
                except (BrokenPipeError, ConnectionResetError):
                    pass
            except Exception as error:
                fixture_errors.append(str(error))
                self.send_error(500)

        do_GET = reply
        do_POST = reply

    class Provider(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                require(self.path == "/v1/chat/completions", "unexpected inference route")
                size = int(self.headers.get("Content-Length", "0"))
                require(0 < size <= 1024 * 1024, "invalid inference size")
                raw = self.rfile.read(size)
                require(secret.encode() not in raw, "credential reached model input")
                request = json.loads(raw)
                provider_calls.append(request)
                tool_result = any(message["role"] == "tool" for message in request["messages"])
                message = dict(role="assistant", content="CASE_WORKFLOW_DONE")
                finish = "stop"
                if not tool_result:
                    name = "case_write" if mode in {"write", "denied_write"} else "case_read"
                    record = {"bad_input": "bad/path", "path_parameter_read": "7;v=1",
                              "path_parameter_traversal": "..;"}.get(mode, "7")
                    arguments = dict(record=record)
                    if name == "case_write":
                        arguments["status"] = "reviewed"
                    calls = [dict(id="case-operation", type="function", function=dict(name=name, arguments=json.dumps(arguments)))]
                    if mode == "budget":
                        calls.append(dict(id="second-operation", type="function", function=dict(name=name, arguments=json.dumps(arguments))))
                    message = dict(role="assistant", content=None, tool_calls=calls)
                    finish = "tool_calls"
                payload = json.dumps(dict(id="fixture", model="fixture", object="chat.completion",
                                          choices=[dict(index=0, message=message, finish_reason=finish)],
                                          usage=dict(prompt_tokens=10, completion_tokens=5, total_tokens=15))).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            except Exception as error:
                fixture_errors.append(str(error))
                self.send_error(500)

    api = ThreadingHTTPServer(("127.0.0.1", 0), API)
    provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    api.daemon_threads = provider.daemon_threads = True
    for server in [api, provider]:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="symbi-outbound-e2e-") as directory:
            root = Path(directory)
            for name in ["agents", "tools", "policies", "observer", ".symbiont/governed", "home"]:
                (root / name).mkdir(parents=True, mode=0o700, exist_ok=True)
            governed, observer = root / ".symbiont/governed", root / "observer"
            (root / ".env").write_text("")
            (root / "agents/case_review.symbi").write_text("agent case_review {}\n")
            (root / "policies/cases.cedar").write_text("permit(principal, action, resource);\n")
            seed = os.urandom(32)
            key = governed / "audit-signing.key"
            key.write_bytes(seed)
            key.chmod(0o600)
            public_der = subprocess.check_output(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"],
                                                input=bytes.fromhex("302e020100300506032b657004220420") + seed,
                                                stderr=subprocess.PIPE, timeout=5)
            public = observer / "public.der"
            public.write_bytes(public_der)
            env = dict(PATH="/usr/bin:/bin", HOME=str(root / "home"), LANG="C.UTF-8", RUST_LOG="error",
                       SYMBIONT_ENV="production", SYMBIONT_MASTER_KEY="0" * 64, EMBEDDING_PROVIDER="disabled",
                       DBUS_SESSION_BUS_ADDRESS="unix:path=/tmp/no-outbound-test-keyring",
                       OPENAI_API_KEY=inference_secret, CHAT_MODEL="fixture",
                       OPENAI_BASE_URL=f"http://127.0.0.1:{provider.server_port}/v1",
                       CASE_SERVICE_KEY=secret, TOOLCLAD_SECRET_CASES="synthetic-forbidden-fallback")

            def configure(kind="normal"):
                credential = 'source="environment"\nvariable="CASE_SERVICE_KEY"\nversion="fixture-v1"'
                env["CASE_SERVICE_KEY"] = secret
                if kind in {"store_missing", "store_read"}:
                    env["CASE_SERVICE_KEY"] = "synthetic-forbidden-fallback"
                    (root / "store.json").write_text(json.dumps({"cases": secret} if kind == "store_read" else {}))
                    (root / "store.json").chmod(0o600)
                    (root / "store.toml").write_text('type="file"\nformat="json"\npath=' + json.dumps(str(root / "store.json")) + '\n')
                    credential = 'source="secret_store"\nconfig_file=' + json.dumps(str(root / "store.toml")) + '\nkey="cases"\nversion="fixture-v1"'
                configuration = '[outbound]\nschema_version=' + ('99' if kind == "version" else '1') + '\ntenant="fixture"\nworkspace="cases"\n'
                configuration += '[outbound.credentials.cases]\n' + credential + '\n'
                for name, method in [("case_read", "GET"), ("case_write", "POST")]:
                    configuration += f'[outbound.capabilities.{name}]\ntool="{name}"\norigin="http://127.0.0.1:{api.server_port}"\nmethod="{method}"\npath_prefix="/records/"\nprivate_origin=true\ncredential="cases"\nmax_requests=1\n'
                    manifest = f'[tool]\nname="{name}"\nversion="1"\ndescription="Review a synthetic case"\nhuman_approval={str(name == "case_write").lower()}\n[args.record]\ntype="integer"\nrequired=true\nmin=1\nmax=100\n'
                    if kind == "path_parameters":
                        configuration += "allow_path_parameters=true\n"
                        manifest = manifest.replace('type="integer"\nrequired=true\nmin=1\nmax=100',
                                                    'type="literal_text"\nrequired=true')
                    if name == "case_write":
                        manifest += '[args.status]\ntype="enum"\nallowed=["reviewed"]\nrequired=true\n'
                    manifest += f'[http]\ncapability="{name}"\nmethod="{method}"\nurl="http://127.0.0.1:{api.server_port}/' + ('other' if kind == "path" else 'records') + '/{record}"\n'
                    if kind == "path_parameters":
                        manifest = manifest.replace('/{record}"', '/{record}/admin"')
                    if name == "case_write":
                        manifest += 'body_template=\'{"status":"{status}"}\'\n[http.headers]\ncontent-type="application/json"\n'
                    manifest += '[output]\nformat="json"\nmax_bytes=4096\n'
                    (root / f"tools/{name}.clad.toml").write_text(manifest)
                (root / "symbiont.toml").write_text(configuration)
                configuration_files = [root / "symbiont.toml", root / "agents/case_review.symbi",
                                       root / "policies/cases.cedar", *sorted((root / "tools").glob("*.toml"))]
                if kind in {"store_missing", "store_read"}:
                    configuration_files.append(root / "store.toml")
                snapshots = {str(path.relative_to(root)): digest(path) for path in configuration_files}
                report["configurations"][kind] = snapshots

            def cli(words):
                return subprocess.run([str(binary), *words], cwd=root, env=env, capture_output=True, text=True, timeout=30)

            def inspect_new(before):
                epochs = []
                for path in sorted(set(governed.glob("*.jsonl")) - before):
                    principal, run_id = path.stem.split(".")
                    entries = verifier(path, public, observer, principal, run_id)
                    require(secret not in path.read_text(), "credential leaked into signed journal")
                    events = [entry["event"].get("ToolEffect", {}).get("effect", {}) for entry in entries]
                    outbound = [event["NetworkRequestStarted"]["outbound"] for event in events
                                if "NetworkRequestStarted" in event and event["NetworkRequestStarted"].get("outbound")]
                    epochs.extend(item["assignment_epoch"] for item in outbound)
                    if mode == "header_reflection":
                        outcomes = [event["NetworkRequestFinished"] for event in events
                                    if "NetworkRequestFinished" in event]
                        require(len(outcomes) == 1, "missing reflected-header outcome")
                        require(outcomes[0].get("response_hash") is None and
                                outcomes[0].get("response_headers_hash") is None,
                                "reflected credential contributed a journal hash")
                    result = cli(["audit", "inspect", str(path), "--run-id", run_id, "--public-key", public_der[-32:].hex()])
                    require(result.returncode in {0, 2}, "runtime rejected independently verified journal")
                    report["journals"].append(dict(mode=mode, sha256=digest(path), records=len(entries),
                                                    independent_verification=True, inspection_exit=result.returncode,
                                                    outbound_capabilities=[item["capability"] for item in outbound]))
                return epochs

            def run_case(case, expect_requests, approve=None):
                nonlocal mode
                mode = case
                before = set(governed.glob("*.jsonl"))
                received = len(receipts)
                start = time.monotonic()
                words = ["run", "case_review", "--input", case]
                if approve is None:
                    result = cli(words)
                    output, code = result.stdout + result.stderr, result.returncode
                else:
                    master, slave = pty.openpty()
                    def terminal():
                        os.setsid()
                        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
                    child = subprocess.Popen([str(binary), *words, "--approval-terminal", "--approval-timeout", "10"],
                                             cwd=root, env=env, stdin=slave, stdout=slave, stderr=slave,
                                             preexec_fn=terminal, close_fds=True)
                    children.append(child)
                    os.close(slave)
                    chunks, answered = bytearray(), False
                    try:
                        while time.monotonic() - start < 30:
                            ready, _, _ = select.select([master], [], [], 0.1)
                            if ready:
                                try:
                                    block = os.read(master, 65536)
                                except OSError:
                                    break
                                if not block:
                                    break
                                chunks.extend(block)
                                prompt = re.search(rb"Type approve ([^ ]+) to approve; any other answer denies:\r?\n> ", chunks)
                                if prompt and not answered:
                                    require(b'case_write' in chunks and b'reviewed' in chunks, "approval did not show exact operation")
                                    os.write(master, (b"approve " + prompt[1] if approve else b"deny") + b"\n")
                                    answered = True
                            if child.poll() is not None and not ready:
                                break
                        code = child.wait(timeout=3)
                        require(answered, "approval prompt was not exercised")
                        output = chunks.decode(errors="replace")
                    finally:
                        os.close(master)
                report["timings_seconds"][case] = round(time.monotonic() - start, 3)
                require(secret not in output, "credential leaked to CLI output")
                require(secret not in json.dumps(provider_calls), "credential leaked to the model")
                require("upstream detail must not reach the agent" not in output + json.dumps(provider_calls), "upstream error detail leaked")
                if case in {"budget", "redirect", "reflection", "header_reflection", "store_missing"}:
                    require(code == 2, f"{case}: uncertain effect was reported as complete")
                tool_results = [message.get("content") for message in (provider_calls[-1]["messages"] if provider_calls else []) if message["role"] == "tool"]
                require(len(receipts) - received == expect_requests, f"{case}: wrong outbound request count; exit={code}, tools={tool_results}, output={output[-1000:]}")
                if case in {"read", "read_again", "write", "store_read", "path_parameter_read"}:
                    require(code == 0 and "CASE_WORKFLOW_DONE" in output, f"{case}: workflow did not complete: {output[-3000:]}")
                if case == "path_parameter_traversal":
                    require("outside the outbound capability" in output + json.dumps(tool_results),
                            "encoded traversal did not exercise the capability refusal")
                epochs = inspect_new(before)
                report["checks"].append(dict(name=case, passed=True, requests=expect_requests, cli_exit=code))
                return epochs

            configure()
            checked = cli(["tools", "validate"])
            require(checked.returncode == 0, checked.stderr)
            first = run_case("read", 1)
            second = run_case("read_again", 1)
            require(first and second and set(first).isdisjoint(second), "assignments were reused between executions")
            run_case("write", 1, approve=True)
            require(json.loads(receipts[-1]["body"]) == {"status": "reviewed"}, "approved body changed")
            run_case("denied_write", 0, approve=False)
            run_case("bad_input", 0)
            run_case("budget", 1)
            run_case("redirect", 1)
            run_case("reflection", 1)
            run_case("header_reflection", 1)
            configure("path_parameters")
            checked = cli(["tools", "validate"])
            require(checked.returncode == 0, checked.stderr)
            run_case("path_parameter_read", 1)
            run_case("path_parameter_traversal", 0)
            configure("store_read")
            run_case("store_read", 1)
            configure("store_missing")
            run_case("store_missing", 0)
            for kind in ["version", "path"]:
                configure(kind)
                prior = len(provider_calls)
                checked = cli(["tools", "validate"])
                require(checked.returncode != 0, f"{kind}: invalid configuration accepted")
                failed = cli(["run", "case_review", "--input", "read"])
                require(failed.returncode != 0 and len(provider_calls) == prior, "invalid configuration reached inference")
                report["checks"].append(dict(name="preflight_" + kind, passed=True, requests=0))

            configure()
            mode = "interrupt"
            invocation = str(uuid.uuid4())
            before = set(governed.glob("*.jsonl"))
            words = ["run", "case_review", "--input", "interrupt", "--invocation-id", invocation]
            with (observer / "interrupt.log").open("w") as log:
                child = subprocess.Popen([str(binary), *words], cwd=root, env=env, stdout=log, stderr=log, start_new_session=True)
                children.append(child)
                require(accepted.wait(15), "interrupted request never reached the fixture")
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)
            release.set()
            calls = len(receipts)
            retry = cli(words)
            require(retry.returncode != 0 and len(receipts) == calls, "uncertain invocation was replayed")
            inspect_new(before)
            require(report["journals"][-1]["inspection_exit"] == 2, "interrupted request was reported complete")
            report["checks"].append(dict(name="interrupted_effect_refuses_replay", passed=True, requests=1))
            require(not fixture_errors, str(fixture_errors))
            report["requests"] = receipts
            report["configuration_digest"] = hashlib.sha256(json.dumps(report["configurations"], sort_keys=True, separators=(",", ":")).encode()).hexdigest()
            report["passed"] = True
    except Exception as error:
        report["errors"].append(str(error))
    finally:
        release.set()
        for child in children:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)
        api.shutdown()
        provider.shutdown()
        report["errors"].extend(fixture_errors)
        report["passed"] = report["passed"] and not report["errors"]
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(passed=report["passed"], checks=len(report["checks"]), errors=report["errors"])))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

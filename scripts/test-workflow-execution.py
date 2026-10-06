#!/usr/bin/env python3
"""Exercise workflow admission, sandboxed effects and failure with shipping symbi up.

Uses synthetic loopback inference, a cached Docker image and temporary state.
Requires OpenSSL and the escape harness's journal verifier. Scoped-key
authority is covered separately by the runtime workflow_execution HTTP tests.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def eventually(predicate, timeout=40):
    deadline = time.monotonic() + timeout
    while True:
        result = predicate()
        if result:
            return result
        if time.monotonic() >= deadline:
            raise TimeoutError("workflow fixture did not reach the expected state")
        time.sleep(.05)


def run(options, audit):
    root = Path(tempfile.mkdtemp(prefix="symbiont-workflow-e2e-"))
    record = {"fixture": str(root), "passed": False}
    for directory in ("home", "effects", "agents", "tools", "policies", "docker-client"):
        (root / directory).mkdir()
    (root / "effects").chmod(0o777)
    (root / "host-canary").write_text("synthetic-host-canary")
    (root / "policies/fixture.cedar").write_text("permit(principal, action, resource);")
    docker_env = {key: value for key, value in os.environ.items() if not key.startswith("DOCKER_")}
    docker_env.update(DOCKER_HOST="unix:///var/run/docker.sock", DOCKER_CONFIG=str(root / "docker-client"))
    image = subprocess.check_output(["docker", "image", "inspect", options.image,
                                     "--format", "{{.Id}}"], env=docker_env, text=True, timeout=10).strip()
    (root / "symbiont.toml").write_text(
        '[sandbox]\ntier="docker"\n[sandbox.docker]\nimage=' + json.dumps(image) +
        '\nvolumes=[' + json.dumps(str(root / "effects") + ':/workspace:rw') + ']\n')
    code = ('from pathlib import Path; import os,sys,json; '
            'value={"token":sys.argv[1],"uid":os.getuid(),'
            '"credential_visible":"OPENAI_API_KEY" in os.environ,'
            '"host_visible":Path(' + json.dumps(str(root / "host-canary")) + ').exists()}; '
            'text=json.dumps(value); Path("/workspace/result.json").write_text(text); print(text)')
    (root / "tools/record_payload.clad.toml").write_text('''[tool]
name="record_payload"
version="1"
description="Record a workflow input"
binary="python3"
timeout_seconds=15
[args.token]
position=1
required=true
type="string"
[command]
template=\'\'\'python3 -c '%s' {token}\'\'\'
[output]
format="text"
[filesystem]
create=["result.json"]
''' % code)
    requests, errors = [], []

    class Provider(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                size = int(self.headers.get("Content-Length", "0"))
                assert self.path == "/v1/chat/completions" and 0 < size <= 1024 * 1024
                assert len(requests) < 4, "unexpected inference count"
                body = json.loads(self.rfile.read(size))
                requests.append(body)
                if any(m["role"] == "assistant" for m in body["messages"]):
                    message = {"role": "assistant", "content": body["messages"][-1]["content"]}
                    finish = "stop"
                else:
                    assert "agent chosen()" in body["messages"][0]["content"]
                    assert "agent sibling()" not in body["messages"][0]["content"]
                    payload = json.loads(next(m["content"] for m in body["messages"] if m["role"] == "user"))
                    assert payload == {"token": "workflow-effect"}, payload
                    message = {"role": "assistant", "content": None, "tool_calls": [{
                        "id": "workflow-effect", "type": "function",
                        "function": {"name": "record_payload", "arguments": json.dumps(payload)}}]}
                    finish = "tool_calls"
                response = json.dumps({"id": "fixture", "object": "chat.completion", "created": 0,
                    "model": "local-fixture", "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
                    "choices": [{"index": 0, "finish_reason": finish, "message": message}]}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(response)))
                self.end_headers()
                self.wfile.write(response)
            except Exception as error:
                errors.append(str(error))
                self.send_error(400)

    provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    provider.daemon_threads = True
    thread = threading.Thread(target=provider.serve_forever, daemon=True)
    thread.start()
    api, webhook = free_port(), free_port()
    token = "synthetic-admin-" + uuid.uuid4().hex
    env = {"PATH": "/usr/bin:/bin", "HOME": str(root / "home"), "LANG": "C.UTF-8",
           "SYMBIONT_ENV": "production", "SYMBIONT_API_TOKEN": token,
           "SYMBIONT_MASTER_KEY": "0" * 64,
           "DBUS_SESSION_BUS_ADDRESS": "unix:path=/tmp/symbiont-no-test-keyring",
           "OPENAI_API_KEY": "synthetic-fixture-key", "CHAT_MODEL": "local-fixture",
           "OPENAI_BASE_URL": f"http://127.0.0.1:{provider.server_port}/v1"}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def call(path, data=None):
        request = urllib.request.Request(f"http://127.0.0.1:{api}/api/v1" + path,
            data=None if data is None else json.dumps(data).encode(),
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json", "Idempotency-Key": str(uuid.uuid4())})
        with opener.open(request, timeout=10) as response:
            assert response.status == 200
            return json.load(response)

    process = None
    try:
        with (root / "server.log").open("w") as log:
            process = subprocess.Popen([str(options.binary), "up", "--port", str(api),
                "--http-port", str(webhook), "--http-bind", "127.0.0.1", "--http.token", "synthetic-webhook"],
                cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)

            def ready():
                assert process.poll() is None, "runtime exited before readiness"
                try:
                    return call("/health")
                except (OSError, urllib.error.URLError):
                    return None

            eventually(ready, 60)
            source = ('metadata { name = "chosen" } agent sibling() {} '
                      'agent chosen() { with sandbox = "docker", timeout = 20.seconds {} }')
            admission = call("/workflows/execute", {"workflow_id": source, "parameters": {"token": "workflow-effect"}})
            assert admission["status"] == "queued" and "execution_started" not in admission
            agent, execution = admission["agent_id"], admission["execution_id"]
            assert uuid.UUID(agent) != uuid.UUID(execution)

            def terminal(run_id, status):
                time.sleep(0.5)
                entries = call(f"/agents/{agent}/history")["history"]
                matching = [entry for entry in entries if entry["execution_id"] == run_id]
                if any(entry["status"] == status for entry in matching):
                    assert sorted(entry["status"] for entry in matching) == sorted(["queued", status])
                    return matching
                return None

            record["completed_history"] = eventually(lambda: terminal(execution, "Completed"))
            effect = json.loads((root / "effects/result.json").read_text())
            assert effect == {"token": "workflow-effect", "uid": 65534, "credential_visible": False, "host_visible": False}
            journal_root = root / ".symbiont/governed"
            # This fresh fixture owns its journal key; only the public half is retained.
            seed = (journal_root / "audit-signing.key").read_bytes()
            assert len(seed) == 32
            public_der = subprocess.run(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"],
                input=bytes.fromhex("302e020100300506032b657004220420") + seed,
                capture_output=True, check=True, timeout=5).stdout
            assert len(public_der) == 44 and public_der[:12].hex() == "302a300506032b6570032100"
            public_hex = public_der[12:].hex()
            journal = journal_root / f"{agent}.{execution}.jsonl"
            entries = audit.verify_journal(journal, public_hex, run_id=execution)
            assert all(entry["agent_id"] == agent for entry in entries)
            assert entries[-1]["event"]["Terminated"]["reason"] == "Completed"
            source_policy = entries[0]["event"]["Started"]["execution_context"]["source_policy"]
            assert source_policy["agent_name"] == "chosen"
            assert source_policy["source_hash"] == "sha256:" + hashlib.sha256(json.dumps(source).encode()).hexdigest()
            approved = [(entry["sequence"], call) for entry in entries
                        for call in entry["event"].get("PolicyEvaluated", {}).get("approved_calls", [])
                        if call["action"].get("ToolCall", {}).get("name") == "record_payload"]
            assert len(approved) == 1
            approved_sequence, authorized = approved[0]
            assert authorized["arguments"] == {"token": "workflow-effect"}
            assert authorized["source_policy"] == source_policy
            assert authorized["resolved"]["argv"] == ["python3", "-c", code, "workflow-effect"]
            boundary = authorized["resolved"]["command_boundary"]
            assert boundary["tier"] == "docker" and boundary["container"]["image"] == image
            assert boundary["container"]["network_mode"] == "none"
            assert boundary["container"]["mounts"] == []
            assert boundary["filesystem"]["read"] == []
            assert boundary["filesystem"]["create"] == ["result.json"]
            observations = [(entry["sequence"], observation) for entry in entries
                            for observation in entry["event"].get("ToolBatchCompleted", {}).get("observations", [])
                            if observation["call_id"] == "workflow-effect"]
            assert len(observations) == 1
            outcome_sequence, observation = observations[0]
            assert outcome_sequence > approved_sequence and not observation["is_error"]
            assert observation["metadata"]["call_fingerprint"] == authorized["fingerprint"]
            output = json.loads(observation["content"])
            assert output["status"] == "success" and output["results"]["exit_code"] == 0
            assert json.loads(output["results"]["raw_output"]) == effect
            assert output["created_files"][0]["sha256"] == digest(root / "effects/result.json")
            assert len(requests) == 2 and not errors
            refused = call("/workflows/execute", {"workflow_id": 'agent unavailable() { with sandbox = "firecracker" {} }',
                           "parameters": {"token": "must-not-execute"}, "agent_id": agent})
            assert refused["status"] == "queued" and "execution_started" not in refused
            assert refused["execution_id"] != execution and refused["agent_id"] == agent
            record["unresolved_history"] = eventually(lambda: terminal(refused["execution_id"], "Unresolved"))
            refused_audit = Path(refused["audit"]["path"])
            assert refused_audit.read_bytes() == b"", "setup refusal must precede journal startup"
            assert refused["audit"]["run_id"] == refused["execution_id"]
            assert len(requests) == 2 and not errors
            assert json.loads((root / "effects/result.json").read_text()) == effect
            assert (root / "host-canary").read_text() == "synthetic-host-canary"
            record.update(passed=True, admission=admission, refused=refused, effect=effect,
                          journal_sha256=digest(journal), audit_public_key=public_hex, image=image)
    except Exception as error:
        record.update(passed=False, error=f"{type(error).__name__}: {error}")
    finally:
        if process is not None and process.poll() is None:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=40)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
                record.update(passed=False, cleanup_error="runtime shutdown timed out")
        provider.shutdown()
        provider.server_close()
        thread.join(timeout=5)
        try:
            leases = list((root / "home/.symbiont/sandbox-leases").glob("*.json"))
            ids = subprocess.check_output(["docker", "ps", "-aq", "--no-trunc"], env=docker_env, text=True, timeout=10).split()
            containers = json.loads(subprocess.check_output(["docker", "inspect", *ids], env=docker_env, text=True, timeout=10)) if ids else []
            owned = [c["Id"] for c in containers if any(Path(m.get("Source", "/")).is_relative_to(root) for m in c.get("Mounts", []))]
            record.update(remaining_leases=[str(p) for p in leases], remaining_containers=owned)
            assert not leases and not owned, "workflow workers remain after shutdown"
        except Exception as error:
            record.update(passed=False, cleanup_error=str(error))
    record.update(inference_requests=len(requests), provider_errors=errors)
    return record


def main():
    if not __debug__:
        raise RuntimeError("Run this assertion-based regression without Python optimization")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--harness-scripts", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--image", default="python:3.12-slim")
    options = parser.parse_args()
    options.binary = options.binary.resolve()
    sys.path.insert(0, str(options.harness_scripts.resolve()))
    import verify_runtime_managed_cli as audit
    files = [Path(__file__), Path(audit.__file__), Path(audit.common.__file__)]
    before = {str(path): digest(path) for path in files}
    binary_hash = digest(options.binary)
    record = run(options, audit)
    record.update(binary_sha256=binary_hash, driver_hashes=before,
                  binary_unchanged=digest(options.binary) == binary_hash,
                  drivers_unchanged={str(path): digest(path) for path in files} == before)
    record["passed"] &= record["binary_unchanged"] and record["drivers_unchanged"]
    options.report.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record))
    return 0 if record["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

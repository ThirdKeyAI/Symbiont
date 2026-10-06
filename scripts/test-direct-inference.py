#!/usr/bin/env python3
"""Test shipping REPL inference against a local HTTP fixture and pinned audit key.

Requires Python 3, OpenSSL with Ed25519 support, and a built repl-cli binary.
No external provider is contacted. Private fixture keys never leave temporary storage.
"""

import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import select
import subprocess
import tempfile
import threading


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_journal(path, public_key, observer, principal, expected_run=None, legacy=False):
    previous = "0" * 64
    entries = []
    run_id = expected_run
    for line in path.read_bytes().splitlines(keepends=True):
        assert line.endswith(b"\n") and line.startswith(b'{"payload":'), "Invalid signed frame"
        text = line.decode("utf-8")
        start = len('{"payload":')
        payload, end = json.JSONDecoder().raw_decode(text, start)
        record = json.loads(text)
        with tempfile.TemporaryDirectory(dir=observer) as work:
            work = Path(work)
            (work / "payload").write_bytes(text[start:end].encode())
            (work / "signature").write_bytes(base64.b64decode(record["signature"], validate=True))
            result = subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
                                     "-inkey", str(public_key), "-rawin", "-in", str(work / "payload"),
                                     "-sigfile", str(work / "signature")], capture_output=True, timeout=5)
            assert result.returncode == 0, "Invalid journal signature"
        assert payload["previous_hash"] == previous
        if legacy:
            assert payload["version"] == 1 and "run_id" not in payload
        else:
            assert payload["version"] == 2
            run_id = run_id or payload["run_id"]
            assert payload["run_id"] == run_id, "Wrong invocation"
        entry = payload["entry"]
        assert entry["sequence"] == len(entries) and entry["agent_id"] == principal
        entries.append(entry)
        previous = hashlib.sha256(line).hexdigest()
    assert entries, "Empty journal"
    return entries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    options = parser.parse_args()
    binary = options.binary.resolve(strict=True)
    planned = ["signed_records_before_http_and_complete_response", "unsafe_storage_prevents_http",
               "tampered_journal_rejected", "abrupt_exit_leaves_incomplete_signed_run"]
    report = dict(passed=False, containment_claim=False, planned=planned, checks=[], exchanges=[],
                  binary=str(binary), binary_sha256=sha256(binary), driver_sha256=sha256(Path(__file__)),
                  http_requests=[], fixture_errors=[])
    try:
        with tempfile.TemporaryDirectory(prefix="symbi-inference-e2e-") as directory:
            root = Path(directory)
            project = root / "project"
            observer = root / "observer"
            project.mkdir(mode=0o700)
            observer.mkdir(mode=0o700)
            (project / ".symbiont").mkdir(mode=0o700)
            governed = project / ".symbiont/governed"
            governed.mkdir(mode=0o700)
            seed = os.urandom(32)
            key_file = governed / "audit-signing.key"
            key_file.write_bytes(seed)
            key_file.chmod(0o600)
            private_der = bytes.fromhex("302e020100300506032b657004220420") + seed
            public_der = subprocess.check_output(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"], input=private_der, stderr=subprocess.PIPE, timeout=5)
            public_key = observer / "public.der"
            public_key.write_bytes(public_der)
            pinned_hex = public_der[-32:].hex()
            release = threading.Event()
            waiting = threading.Event()
            principal = None
            fixture_token = "synthetic-fixture-key"

            class Handler(BaseHTTPRequestHandler):
                def log_message(self, *_args):
                    pass

                def do_POST(self):
                    try:
                        assert self.path == "/v1/chat/completions"
                        assert self.headers.get("Authorization") == f"Bearer {fixture_token}"
                        count = int(self.headers["Content-Length"])
                        assert 0 < count <= 1024 * 1024
                        request = json.loads(self.rfile.read(count))
                        assert request["model"] == "fixture" and request["max_tokens"] == 4096
                        candidates = []
                        for path in governed.glob("*.jsonl"):
                            entries = verify_journal(path, public_key, observer, principal)
                            if "DirectInferenceRequested" in entries[-1]["event"]:
                                candidates.append(path)
                        assert len(candidates) == 1, "Missing pre-effect signed request"
                        report["http_requests"].append(request)
                        if request["messages"][-1]["content"] == "wait-fixture":
                            waiting.set()
                            assert release.wait(15), "Fixture release deadline"
                        body = json.dumps(dict(id="fixture", model="fixture", choices=[dict(index=0,
                            message=dict(role="assistant", content="shipping fixture response"), finish_reason="stop")],
                            usage=dict(prompt_tokens=2, completion_tokens=3, total_tokens=5))).encode()
                        self.send_response(200)
                        self.send_header("Content-Type", "application/json")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        try:
                            self.wfile.write(body)
                        except (BrokenPipeError, ConnectionResetError):
                            pass  # The abrupt-exit case deliberately closes this connection.
                    except Exception as error:
                        report["fixture_errors"].append(str(error))
                        self.send_error(500)

            http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            http.daemon_threads = True
            thread = threading.Thread(target=http.serve_forever, daemon=True)
            thread.start()
            env = dict(PATH="/usr/bin:/bin", HOME=str(project), LANG="C.UTF-8",
                       OPENAI_API_KEY=fixture_token, CHAT_MODEL="fixture",
                       OPENAI_BASE_URL=f"http://127.0.0.1:{http.server_port}/v1",
                       SYMBIONT_MASTER_KEY="0" * 64,
                       DBUS_SESSION_BUS_ADDRESS="unix:path=/tmp/symbi-no-test-keyring")
            with (observer / "stderr").open("w") as stderr:
                child = subprocess.Popen([str(binary), "--stdio"], cwd=project, env=env,
                                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True)

                def rpc(code, await_response=True):
                    request = dict(id=len(report["exchanges"]) + 1, method="evaluate", params=dict(code=code))
                    child.stdin.write(json.dumps(request) + "\n")
                    child.stdin.flush()
                    row = dict(request=request)
                    report["exchanges"].append(row)
                    if not await_response:
                        return None
                    assert select.select([child.stdout], [], [], 15)[0], "RPC deadline"
                    response = json.loads(child.stdout.readline())
                    assert response["id"] == request["id"]
                    row["response"] = response
                    return response

                try:
                    agent = rpc("agent InferenceWorker {}")["result"]["output"]
                    principal = re.search(r"Agent\(id: ([0-9a-f-]{36}),", agent)[1]
                    rpc("behavior Inferred { steps { return llm_call(args) } }")
                    rpc(f":agent start {principal}")
                    result = rpc(f":agent execute {principal} Inferred shipping-request")
                    assert result["result"]["output"].endswith('"shipping fixture response"'), result
                    references = json.loads(rpc(":audit")["result"]["output"])
                    assert references["omitted"] == 0 and len(references["entries"]) == 1
                    reference = references["entries"][0]
                    assert reference["agent_id"] == principal and reference["operation"] == "llm_call"
                    audit = reference["audit"]
                    assert audit["public_key"] == pinned_hex, "Unexpected audit signer"
                    path = Path(audit["path"])
                    assert path.parent == governed and path.stat().st_mode & 0o777 == 0o600
                    entries = verify_journal(path, public_key, observer, principal, audit["run_id"])
                    assert [next(iter(entry["event"])) for entry in entries] == [
                        "Started", "DirectInferenceRequested", "DirectInferenceResponseReceived",
                        "DirectInferenceFinished", "Terminated"]
                    assert entries[-1]["event"]["Terminated"]["reason"] == "Completed"
                    assert report["http_requests"][0]["messages"] == [dict(role="user", content="shipping-request")]
                    report["checks"].append(planned[0])

                    governed.chmod(0o777)
                    denied = rpc(f":agent execute {principal} Inferred blocked-request")
                    assert "Required DSL audit initialization failed" in denied["error"]["message"]
                    assert len(report["http_requests"]) == 1
                    governed.chmod(0o700)
                    report["checks"].append(planned[1])

                    tampered = observer / "tampered.jsonl"
                    original = path.read_bytes()
                    changed = original.replace(b'"llm_call"', b'"llm_fail"', 1)
                    assert changed != original
                    tampered.write_bytes(changed)
                    try:
                        verify_journal(tampered, public_key, observer, principal, audit["run_id"])
                    except AssertionError:
                        pass
                    else:
                        raise AssertionError("Tampered journal verified")
                    report["checks"].append(planned[2])

                    before = set(governed.glob("*.jsonl"))
                    rpc(f":agent execute {principal} Inferred wait-fixture", await_response=False)
                    assert waiting.wait(10), report["fixture_errors"]
                    child.terminate()
                    child.wait(timeout=10)
                    created = set(governed.glob("*.jsonl")) - before
                    assert len(created) == 1
                    interrupted = verify_journal(created.pop(), public_key, observer, principal)
                    assert len(interrupted) == 2 and "DirectInferenceRequested" in interrupted[-1]["event"]
                    assert not any("Terminated" in entry["event"] for entry in interrupted)
                    report["checks"].append(planned[3])
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.wait()
                    child.stdin.close()
                    child.stdout.close()
                    release.set()
                    http.shutdown()
                    http.server_close()
                    thread.join(timeout=5)
                    assert not thread.is_alive()
            assert not report["fixture_errors"], report["fixture_errors"]
        assert report["checks"] == planned
        assert sha256(binary) == report["binary_sha256"]
        assert sha256(Path(__file__)) == report["driver_sha256"]
        report["passed"] = True
    finally:
        options.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(passed=True, checks=len(report["checks"]), report=str(options.report))))


if __name__ == "__main__":
    main()

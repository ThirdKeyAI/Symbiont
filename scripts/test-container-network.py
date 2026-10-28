#!/usr/bin/env python3
"""Shipping CLI network-profile regression with synthetic local effects only.

Uses a cached Docker image, a scripted loopback model and a host TCP receiver.
Production profiles must refuse bridge/host networking before inference. An
isolated worker must still complete useful work; development bridge remains
an explicit unrestricted capability. This is deterministic regression evidence.
"""
import argparse
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import socketserver
import subprocess
import tempfile
import threading
import time
import uuid


CASES = {
    "production_bridge": ("production", "docker", "bridge", "bridge networking is forbidden"),
    "production_alias_gvisor": (" PROD ", "gvisor", "bridge", "bridge networking is forbidden"),
    "invalid_environment": ("production-like", "docker", "bridge", "unknown value"),
    "invalid_environment_bytes": (b"\xff", "docker", "bridge", "unknown value"),
    "production_host": ("production", "docker", "host", "host and shared namespaces"),
    "production_isolated": ("production", "docker", "none", None),
    "development_bridge": ("development", "docker", "bridge", None),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def docker(docker_env, *args):
    return subprocess.run(["docker", *args], capture_output=True, text=True,
                          timeout=10, check=True, env=docker_env).stdout.strip()


@contextmanager
def fixture_directory(row):
    root = Path(tempfile.mkdtemp(prefix="symbi-net-"))
    try:
        yield root
    finally:
        if row.get("cleanup_confirmed"):
            shutil.rmtree(root)
        else:
            # Pending creation/cleanup leases need their original durable paths.
            row["retained_fixture"] = str(root)


def run_case(name, binary, image, gateway, docker_env):
    environment, tier, network, refusal = CASES[name]
    row = {"case": name, "passed": False, "environment": repr(environment) if isinstance(environment, bytes) else environment,
           "tier": tier, "network": network, "requests": [], "connections": [], "errors": []}
    label = "symbi.network-regression=" + uuid.uuid4().hex

    class Receiver(socketserver.BaseRequestHandler):
        def handle(self):
            self.request.settimeout(2)
            payload = self.request.recv(64)
            row["connections"].append(payload.decode("ascii"))
            if payload == b"synthetic-network-probe":
                self.request.sendall(b"ack")

    class Provider(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                assert self.path == "/v1/chat/completions"
                length = int(self.headers["Content-Length"])
                assert 0 < length <= 1024 * 1024
                row["requests"].append(json.loads(self.rfile.read(length)))
                assert len(row["requests"]) <= 2
                if len(row["requests"]) == 1:
                    message = {"role": "assistant", "content": None, "tool_calls": [{
                        "id": "network-probe", "type": "function",
                        "function": {"name": "network_probe", "arguments": "{}"}}]}
                    finish = "tool_calls"
                else:
                    message = {"role": "assistant", "content": "fixture complete"}
                    finish = "stop"
                body = json.dumps({"id": "fixture", "object": "chat.completion", "created": 0,
                    "model": "scripted-fixture", "choices": [{"index": 0, "message": message,
                    "finish_reason": finish}], "usage": {"prompt_tokens": 1,
                    "completion_tokens": 1, "total_tokens": 2}}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except Exception as error:
                row["errors"].append(str(error))
                self.send_error(400)

    with fixture_directory(row) as root:
        output = root / "output"
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        for child in ("agents", "tools", "policies/run", "home"):
            (root / child).mkdir(parents=True)
        receiver = socketserver.TCPServer((gateway, 0), Receiver)
        provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        threads = [threading.Thread(target=server.serve_forever, daemon=True)
                   for server in (receiver, provider)]
        for thread in threads:
            thread.start()
        try:
            payload = root / "probe.py"
            payload.write_text(f'''import json, os, pathlib, socket
assert os.getuid() != 0
assert 'OPENAI_API_KEY' not in os.environ
result = {{'sum': sum([2, 3, 5]), 'connected': False}}
try:
    with socket.create_connection(({gateway!r}, {receiver.server_address[1]}), 1) as connection:
        connection.sendall(b'synthetic-network-probe')
        result['connected'] = connection.recv(3) == b'ack'
except OSError:
    pass
pathlib.Path('/workspace/result.json').write_text(json.dumps(result))
print(json.dumps(result))
''')
            payload.chmod(0o444)
            (root / "agents/fixture.symbi").write_text('agent fixture() {}\n')
            table = "sandbox.docker" if tier == "docker" else "sandbox.gvisor.docker"
            (root / "symbiont.toml").write_text(
                f'[sandbox]\ntier={json.dumps(tier)}\n[{table}]\nimage={json.dumps(image)}\n'
                f'network_mode={json.dumps(network)}\n'
                f'volumes={json.dumps([f"{output}:/workspace:rw", f"{payload}:/workspace/probe.py:ro"])}\n'
                f'extra_flags={json.dumps(["--label=" + label])}\n')
            (root / "tools/network_probe.clad.toml").write_text('''[tool]
name = "network_probe"
version = "1"
binary = "/usr/local/bin/python3"
description = "Synthetic network probe and useful arithmetic"
[command]
template = "/usr/local/bin/python3 probe.py"
[output]
format = "json"
[filesystem]
read = ["probe.py"]
create = ["result.json"]
''')
            (root / "policies/run/fixture.cedar").write_text('permit(principal, action, resource);\n')
            row["fixture_hashes"] = {str(p.relative_to(root)): digest(p)
                for p in root.rglob("*") if p.is_file()}
            process = subprocess.run([str(binary), "run", "fixture", "--input", "Run network_probe",
                "--max-iterations", "3"], cwd=root, capture_output=True, text=True, timeout=35,
                env={**docker_env, "HOME": str(root / "home"), "LANG": "C.UTF-8",
                     "SYMBIONT_ENV": environment, "SYMBIONT_SANDBOX_STATE_DIR": str(root / "leases"),
                     "OPENAI_API_KEY": "synthetic-fixture-key", "CHAT_MODEL": "scripted-fixture",
                     "OPENAI_BASE_URL": f"http://127.0.0.1:{provider.server_port}/v1"})
            row.update(exit_code=process.returncode, stdout=process.stdout, stderr=process.stderr)
            result_file = output / "result.json"
            if refusal:
                assert process.returncode != 0 and refusal in process.stderr, process.stderr
                assert not row["requests"] and not row["connections"] and not result_file.exists()
            else:
                assert process.returncode == 0 and "Completed" in process.stderr, process.stderr
                assert len(row["requests"]) == 2 and not row["errors"]
                row["effect"] = json.loads(result_file.read_text())
                assert row["effect"] == {"sum": 10, "connected": network == "bridge"}
                assert row["connections"] == (["synthetic-network-probe"] if network == "bridge" else [])
                tool = [m for m in row["requests"][-1]["messages"] if m["role"] == "tool"]
                assert len(tool) == 1 and tool[0]["tool_call_id"] == "network-probe"
                assert json.loads(tool[0]["content"])["status"] == "success"
            row["passed"] = True
        except Exception as error:
            row["errors"].append(f"{type(error).__name__}: {error}")
        finally:
            for server in (provider, receiver):
                server.shutdown()
                server.server_close()
            for thread in threads:
                thread.join(timeout=5)
            deadline = time.monotonic() + 10
            while True:
                workers = docker(docker_env, "ps", "-aq", "--filter", "label=" + label).splitlines()
                if not workers or time.monotonic() >= deadline:
                    break
                time.sleep(.1)
            row["leftover_workers"] = workers
            row["leftover_leases"] = [p.name for p in (root / "leases").glob("*.json")]
            row["cleanup_confirmed"] = not workers and not row["leftover_leases"]
            if workers or row["leftover_leases"]:
                row["passed"] = False
                row["errors"].append("unacknowledged worker cleanup")
                # Remove only this fixture's workers; keep the failure in evidence.
                for worker in workers:
                    docker(docker_env, "rm", "-f", "-v", worker)
    return row


def main():
    if not __debug__:
        raise RuntimeError("Run this assertion-based regression without Python optimization")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--image", default="python:3.12-slim")
    parser.add_argument("--cases", nargs="+", choices=CASES, default=list(CASES))
    options = parser.parse_args()
    binary = options.binary.resolve()
    before = digest(binary)
    rows = []
    with tempfile.TemporaryDirectory(prefix="symbi-net-client-") as client_dir:
        # Both inspection/cleanup and the runtime use this one local daemon,
        # independent of ambient Docker contexts, hosts, TLS or credentials.
        docker_env = {"PATH": "/usr/bin:/bin", "DOCKER_HOST": "unix:///var/run/docker.sock",
                      "DOCKER_CONFIG": client_dir}
        image = docker(docker_env, "image", "inspect", options.image, "--format", "{{.Id}}")
        gateway = docker(docker_env, "network", "inspect", "bridge", "--format", "{{(index .IPAM.Config 0).Gateway}}")
        for case in options.cases:
            try:
                rows.append(run_case(case, binary, image, gateway, docker_env))
            except Exception as error:
                rows.append({"case": case, "passed": False, "error": f"{type(error).__name__}: {error}"})
    report = {"binary_sha256": before, "binary_unchanged": digest(binary) == before,
              "driver_sha256": digest(Path(__file__)), "image": image,
              "planned_cases": options.cases, "cases": rows}
    report["passed"] = report["binary_unchanged"] and all(row["passed"] for row in rows)
    options.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "cases": {r["case"]: r["passed"] for r in rows}}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

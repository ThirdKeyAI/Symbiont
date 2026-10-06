#!/usr/bin/env python3
"""Exercise shipping Landlock MCP dispatch with a signed local fixture.

Requires Linux with Landlock ABI 6+, delegated systemd user services, Python 3,
OpenSSL, and a built symbi.
Uses synthetic data and local inference only. This checks filesystem, socket and signal
enforcement, required audit and cleanup after runtime SIGKILL. Adaptive escape
resistance is not established by these deterministic fixtures.
"""
import argparse
import base64
from contextlib import ExitStack
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import runpy
import signal
import select
import socket
import subprocess
import tempfile
import threading
import traceback
import time
from delegated_service_fixture import AutomaticDelegatedService, DelegatedService, eventually


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


SERVER = r'''
import errno, json, os, pathlib, signal, socket, sys, time
schema = json.loads(os.environ['SCHEMA'])
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':request['params']['protocolVersion'], 'capabilities':{'tools':{}}, 'serverInfo':{'name':'probe','version':'1'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'probe','inputSchema':schema}]}
    elif method == 'tools/call':
        if os.environ.get('HOLD_FOR_CANCELLATION') == '1':
            if os.fork() == 0:
                os.setsid()
                while True: time.sleep(1)
            time.sleep(60)
        value = {'sum':sum([2,3,5]), 'system_read':bool(pathlib.Path(sys.executable).read_bytes()), 'inherited_secret':os.environ.get('PRIVATE_FIXTURE_SECRET')}
        for label, mode in [('read_denied','r'), ('write_denied','w')]:
            try:
                with open(os.environ['SENTINEL'], mode) as file:
                    if mode == 'r': file.read()
                    else: file.write('changed')
            except OSError as error: value[label] = error.errno == errno.EACCES
            else: value[label] = False
        for label, family, kind, address in [
            ('tcp_denied',socket.AF_INET,socket.SOCK_STREAM,('127.0.0.1',int(os.environ['PORT']))),
            ('udp_denied',socket.AF_INET,socket.SOCK_DGRAM,('127.0.0.1',int(os.environ['UDP_PORT']))),
            ('unix_denied',socket.AF_UNIX,socket.SOCK_STREAM,os.environ['UNIX_PATH'])]:
            try:
                with socket.socket(family,kind) as connection:
                    connection.settimeout(2)
                    connection.connect(address)
                    if label != 'tcp_denied': connection.sendall(b'fixture-probe')
            except OSError as error: value[label] = error.errno in (errno.EACCES,errno.EPERM)
            else: value[label] = False
        try: os.kill(int(os.environ['OBSERVER_PID']),signal.SIGUSR1)
        except OSError as error: value['signal_denied'] = error.errno in (errno.EACCES,errno.EPERM)
        else: value['signal_denied'] = False
        left, right = socket.socketpair()
        with left, right:
            left.sendall(b'local')
            value['local_socketpair'] = right.recv(5) == b'local'
        try:
            left, right = socket.socketpair(socket.AF_UNIX, socket.SOCK_DGRAM)
            with left, right: left.sendto(b'fixture-probe', os.environ['UNIX_DATAGRAM_PATH'])
        except OSError as error: value['datagram_pair_denied'] = error.errno == errno.EACCES
        else: value['datagram_pair_denied'] = False
        result = {'content':[{'type':'text','text':json.dumps(value)}]}
    else: result = {}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--automatic-supervisor", action="store_true",
                        help="bootstrap through shipping doctor without a manually created unit")
    args = parser.parse_args()
    assert __debug__, "Python assertions must be enabled"
    binary = args.binary.resolve(strict=True)
    report = dict(passed=False, automatic_supervisor=args.automatic_supervisor,
                  binary_sha256=digest(binary), driver_sha256=digest(__file__),
                  checks=[], requests=[], provider_errors=[])
    provider = None
    service = None
    observers = []
    runtime = None
    pidfds = []
    prior_signal = signal.signal(signal.SIGUSR1, lambda number, _: report.setdefault('received_signals', []).append(number))
    try:
        with tempfile.TemporaryDirectory(prefix="symbi-landlock-") as directory, ExitStack() as cleanup:
            root = Path(directory)
            project, observer = root / "project", root / "observer"
            for path in [observer, project / "home", project / "tools", project / "agents",
                         project / "policies", project / ".symbiont/governed"]:
                path.mkdir(mode=0o700, parents=True, exist_ok=True)
            service_type = AutomaticDelegatedService if args.automatic_supervisor else DelegatedService
            service = cleanup.enter_context(service_type(binary, root / "state"))
            sentinel = observer / "private"
            sentinel.write_text("protected synthetic data")
            unix = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            observers.append(unix)
            unix_path = str(observer / "control.sock")
            unix.bind(unix_path)
            unix.listen(1)
            unix.setblocking(False)
            unix_datagram = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
            observers.append(unix_datagram)
            unix_datagram_path = str(observer / "control.dgram")
            unix_datagram.bind(unix_datagram_path)
            unix_datagram.setblocking(False)
            udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            observers.append(udp)
            udp.bind(("127.0.0.1", 0))
            udp.setblocking(False)
            (project / ".env").write_text("")
            (project / "agents/fixture.symbi").write_text("agent fixture() {}\n")
            (project / "policies/fixture.cedar").write_text("permit(principal, action, resource);\n")
            (project / "tools/probe.clad.toml").write_text(
                '[tool]\nname="probe"\nversion="1"\ndescription="Check the worker boundary"\n'
                'timeout_seconds=10\n[output]\nformat="json"\n[mcp]\nserver="probe"\ntool="probe"\n')
            governed = project / ".symbiont/governed"
            seed = os.urandom(32)
            key = governed / "audit-signing.key"
            key.write_bytes(seed)
            key.chmod(0o600)
            public = subprocess.run(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"],
                                    input=bytes.fromhex("302e020100300506032b657004220420") + seed,
                                    capture_output=True, check=True, timeout=5).stdout
            public_path = observer / "public.der"
            public_path.write_bytes(public)
            report["public_key"] = public[-32:].hex()
            helper = Path(__file__).with_name("test-direct-inference.py")
            verifier = runpy.run_path(str(helper))["verify_journal"]
            report["verifier_sha256"] = digest(helper)
            signing_key = observer / "schema.pem"
            signing_key.write_bytes(subprocess.run(
                ["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"],
                capture_output=True, check=True, timeout=5).stdout)
            signing_key.chmod(0o600)
            schema_public = subprocess.run(["openssl", "pkey", "-in", str(signing_key), "-pubout"],
                                           capture_output=True, check=True, timeout=5).stdout.decode()
            schema = dict(type="object", properties={})
            signature = subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(signing_key)],
                                       input=json.dumps(schema, sort_keys=True, separators=(",", ":")).encode(),
                                       capture_output=True, check=True, timeout=5).stdout
            schema["signature"] = base64.b64encode(signature).decode()

            class Provider(BaseHTTPRequestHandler):
                def log_message(self, *_):
                    pass

                def do_POST(self):
                    try:
                        size = int(self.headers.get("Content-Length", "0"))
                        assert self.path == "/v1/chat/completions" and 0 < size <= 1024 * 1024
                        request = json.loads(self.rfile.read(size))
                        report["requests"].append(request)
                        assert len(report["requests"]) <= 3
                        if len(report["requests"]) in (1, 3):
                            message = dict(role="assistant", content=None, tool_calls=[dict(
                                id="probe", type="function", function=dict(name="probe", arguments="{}"))])
                            finish = "tool_calls"
                        else:
                            message, finish = dict(role="assistant", content="fixture complete"), "stop"
                        body = json.dumps(dict(id="fixture", model="fixture", choices=[dict(
                            index=0, message=message, finish_reason=finish)],
                            usage=dict(prompt_tokens=1, completion_tokens=1, total_tokens=2))).encode()
                        self.send_response(200)
                        self.send_header("Content-Type", "application/json")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                    except Exception as error:
                        report["provider_errors"].append(repr(error))
                        self.send_error(500)

            provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
            provider.daemon_threads = True
            threading.Thread(target=provider.serve_forever, daemon=True).start()
            (project / "mcp-config.toml").write_text(
                '[servers.probe]\ncommand="/usr/bin/python3"\nargs=' + json.dumps(["-u", "-c", SERVER])
                + '\npublic_key_pem=' + json.dumps(schema_public)
                + '\n[servers.probe.env]\nSCHEMA=' + json.dumps(json.dumps(schema))
                + '\nSENTINEL=' + json.dumps(str(sentinel))
                + '\nUNIX_PATH=' + json.dumps(unix_path)
                + '\nUNIX_DATAGRAM_PATH=' + json.dumps(unix_datagram_path)
                + '\nUDP_PORT=' + json.dumps(str(udp.getsockname()[1]))
                + '\nOBSERVER_PID=' + json.dumps(str(os.getpid()))
                + '\nPORT=' + json.dumps(str(provider.server_port)) + '\n')
            config = project / "symbiont.toml"
            config.write_text('[sandbox]\ntier="landlock"\n[sandbox.landlock]\nabi_floor=6\nrequire_network=true\n')
            env = dict(PATH="/usr/bin:/bin", HOME=str(project / "home"), LANG="C.UTF-8",
                       SYMBIONT_ENV="production", OPENAI_API_KEY="synthetic-key", CHAT_MODEL="fixture",
                       OPENAI_BASE_URL=f"http://127.0.0.1:{provider.server_port}/v1",
                       PRIVATE_FIXTURE_SECRET="synthetic-inherited-secret",
                       SYMBIONT_SANDBOX_STATE_DIR=str(service.state))
            command = [str(binary), "run", "fixture", "--input", "Run probe once", "--max-iterations", "3"]
            result = subprocess.run(command, cwd=project, env=env, capture_output=True, text=True, timeout=30)
            report["execution"] = dict(exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr)
            assert result.returncode == 0, report["execution"]
            journals = list(governed.glob("*.jsonl"))
            assert len(journals) == 1 and len(report["requests"]) == 2 and not report["provider_errors"]
            journal = journals[0]
            principal, run_id = journal.stem.split(".")
            entries = verifier(journal, public_path, observer, principal, run_id)
            report["journal"] = dict(sha256=digest(journal), entries=entries)
            assert entries[-1]["event"]["Terminated"]["reason"] == "Completed"
            approvals = [call for entry in entries
                         for call in entry['event'].get('PolicyEvaluated', {}).get('approved_calls', [])
                         if call['action'].get('ToolCall', {}).get('name') == 'probe']
            assert len(approvals) == 1
            boundary = approvals[0]['resolved']['command_boundary']['landlock']
            assert boundary['boundary_version'] == 3 and boundary['abi_required'] == 6
            assert boundary['supervision'] == 'delegated_cgroup_v1'
            assert service.capacity()['reserved']['workers'] == 0
            assert boundary['scopes'] == ['signal', 'abstract_unix_socket']
            assert boundary['socket_policy'] == 'private_unix_stream_pair_only'
            assert boundary['inherited_descriptors'] == 'stdio_only' and boundary['io_uring'] == 'denied'
            outcomes = [o for e in entries for o in e["event"].get("ToolBatchCompleted", {}).get("observations", [])]
            assert len(outcomes) == 1
            envelope = json.loads(outcomes[0]["content"])
            assert envelope["status"] == "executed", envelope
            value = json.loads(envelope["results"][0]["text"])
            report["probe"] = value
            try:
                connection, _ = unix.accept()
                with connection:
                    connection.settimeout(1)
                    report['unix_received'] = connection.recv(128).decode()
            except BlockingIOError: report['unix_received'] = None
            try: report['udp_received'] = udp.recv(128).decode()
            except BlockingIOError: report['udp_received'] = None
            try: report['unix_datagram_received'] = unix_datagram.recv(128).decode()
            except BlockingIOError: report['unix_datagram_received'] = None
            assert value == dict(sum=10, system_read=True, inherited_secret=None,
                                 read_denied=True, write_denied=True, tcp_denied=True,
                                 udp_denied=True, unix_denied=True, signal_denied=True,
                                 local_socketpair=True, datagram_pair_denied=True), value
            assert not report.get('received_signals') and report['unix_received'] is None and report['udp_received'] is None
            assert report['unix_datagram_received'] is None
            assert sentinel.read_text() == "protected synthetic data"
            report["checks"].append("signed_mcp_call_returns_useful_result_with_filesystem_socket_and_signal_denials")
            report["checks"].append("required_run_journal_verifies_and_parent_environment_is_not_inherited")

            config.write_text(config.read_text().replace("abi_floor=6", "abi_floor=99"))
            result = subprocess.run(command, cwd=project, env=env, capture_output=True, text=True, timeout=15)
            report["unavailable_kernel"] = dict(exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr)
            assert result.returncode != 0 and "99" in result.stdout + result.stderr
            assert len(report["requests"]) == 2 and sentinel.read_text() == "protected synthetic data"
            report["checks"].append("unsupported_kernel_is_refused_before_inference_without_host_fallback")
            config.write_text(config.read_text().replace("abi_floor=99", "abi_floor=6"))
            with (project / "mcp-config.toml").open('a') as stream:
                stream.write('HOLD_FOR_CANCELLATION="1"\n')
            with (observer/'cancel.stdout').open('w') as stdout, (observer/'cancel.stderr').open('w') as stderr:
                runtime = subprocess.Popen(command, cwd=project, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
            def detached_worker():
                assert runtime.poll() is None, (observer/'cancel.stderr').read_text()
                for path in service.state.glob('*.json'):
                    try: record = json.loads(path.read_text())
                    except FileNotFoundError: continue
                    if record['state']['phase'] != 'host_created' or record.get('origin') is None: continue
                    group = Path(record['state']['cgroup']['path'])
                    try: pids = [int(pid) for pid in (group/'cgroup.procs').read_text().split()]
                    except FileNotFoundError: continue
                    if len(pids) >= 2 and any(os.getsid(pid) == pid for pid in pids):
                        return path, record, group, pids
            lease_path, record, group, pids = eventually(detached_worker, 15)
            report['cancelled_worker'] = dict(record=record, pids=pids)
            pidfds.extend(os.pidfd_open(pid) for pid in pids)
            os.killpg(runtime.pid, signal.SIGKILL)
            runtime.wait(timeout=5)
            eventually(lambda: not group.exists() and not lease_path.exists(), 12)
            assert all(select.select([fd], [], [], 0)[0] for fd in pidfds)
            assert service.capacity()['reserved']['workers'] == 0
            assert len(report['requests']) == 3
            report['checks'].append('shipping_runtime_SIGKILL_closes_lease_and_reaps_detached_MCP_descendant_without_runtime_cleanup')
            report["passed"] = True
    except Exception as error:
        report.update(error=f"{type(error).__name__}: {error}", traceback=traceback.format_exc())
    finally:
        if provider is not None:
            provider.shutdown()
            provider.server_close()
        if runtime is not None and runtime.poll() is None:
            os.killpg(runtime.pid, signal.SIGKILL)
            runtime.wait(timeout=5)
        for fd in pidfds: os.close(fd)
        for observer in observers: observer.close()
        signal.signal(signal.SIGUSR1, prior_signal)
        if digest(binary) != report["binary_sha256"]:
            report.update(passed=False, error="binary changed during test")
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(passed=report["passed"], report=str(args.report))))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Exercise shipping shell turns, local inference, Gate review and brokered files.

Requires the built shell and supervisor, Docker for cleanup observations, tmux and
OpenSSL. All projects, keys, provider replies and file effects are synthetic.
"""
import argparse
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading
import time
import uuid

IMAGE = "sha256:7bf61a5ec2a4631b240bc8cf83404e2dd37fb06a4b4bbd34b2c36906a5fb4aee"

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def wait(predicate, description, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.05)
    raise AssertionError("Timed out: " + description)

class Terminal:
    def __init__(self, binary, project, observer, env, tmux):
        self.tmux, self.project, self.env = tmux, project, env
        self.socket = observer / "tmux.socket"
        self.frames, self.pids = [], []
        try:
            self.call("new-session", "-d", "-s", "fixture", "-x", "160", "-y", "45",
                      "-c", str(project), str(binary), "--yes")
            self.pids = [int(self.call("display-message", "-p", field).stdout.strip())
                         for field in ["#{pid}", "#{pane_pid}"]]
        except BaseException:
            self.close()
            raise

    def call(self, *args, check=True):
        return subprocess.run([self.tmux, "-S", str(self.socket), "-f", "/dev/null", *args],
                              env=self.env, cwd=self.project, capture_output=True,
                              text=True, check=check, timeout=5)

    def send(self, text):
        self.call("send-keys", "-t", "fixture:0.0", "-l", "--", text)

    def frame(self):
        frame = self.call("capture-pane", "-t", "fixture:0.0", "-p").stdout
        if not self.frames or self.frames[-1] != frame:
            self.frames.append(frame)
        assert sum(len(frame) for frame in self.frames) <= 2 * 1024 * 1024
        return frame

    def shown(self, text, timeout=15):
        wait(lambda: text in self.frame(), "terminal display " + text, timeout)

    def review(self, markers):
        for _ in range(40):
            frame = self.frame()
            markers = [marker for marker in markers if marker not in frame]
            if not markers:
                return
            self.send("\x1b[6~")
            time.sleep(.12)
        raise AssertionError("Review missing " + repr(markers))

    def close(self):
        self.call("kill-server", check=False)
        def alive(pid):
            path = Path(f"/proc/{pid}/stat")
            try:
                return path.read_text().split(")", 1)[1].split()[0] != "Z"
            except FileNotFoundError:
                return False
        wait(lambda: not any(alive(pid) for pid in self.pids), "terminal cleanup", 5)
        assert self.call("has-session", "-t", "fixture", check=False).returncode != 0


@contextmanager
def fixture_directory(row):
    path = tempfile.mkdtemp(prefix="symbi-shell-governance-")
    try:
        yield path
    finally:
        if row.get("leftover_workers") or row.get("leftover_leases") or row.get("cleanup_wait_error"):
            # Retain the supervisor's actual paths until recovery completes.
            row["retained_fixture"] = path
        else:
            shutil.rmtree(path)

def run_case(case, options, verifier):
    row = dict(case=case, passed=False, requests=[], frames=[], errors=[])
    canonical = case.startswith("canonical_")
    outcome = case.removeprefix("canonical_")
    with fixture_directory(row) as directory:
        root = Path(directory)
        project, observer = root / "project", root / "observer"
        project.mkdir(mode=0o700); observer.mkdir(mode=0o700)
        data = project / "data"; data.mkdir()
        (project / ".symbiont").mkdir(mode=0o700)
        governed = project / ".symbiont/governed"; governed.mkdir(mode=0o700)
        seed = os.urandom(32)
        key_file = governed / "audit-signing.key"
        key_file.write_bytes(seed); key_file.chmod(0o600)
        public = subprocess.check_output(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"],
            input=bytes.fromhex("302e020100300506032b657004220420") + seed, stderr=subprocess.PIPE, timeout=5)
        public_file = observer / "public.der"; public_file.write_bytes(public)
        row["pinned_public_key"] = public[-32:].hex()
        policy = project / "policies/shell"; policy.mkdir(parents=True)
        (policy / "orchestrator.cedar").write_text(json.dumps([
            dict(name="fixture", active=True, source="permit(principal, action, resource);")]))
        label = "symbi.shell-governance=" + uuid.uuid4().hex
        (project / "symbiont.toml").write_text(
            '[sandbox]\ntier="docker"\n[sandbox.docker]\n' +
            'image=' + json.dumps(IMAGE) + '\nuser=' + json.dumps(f"{os.getuid()}:{os.getgid()}") + '\n' +
            'volumes=[' + json.dumps(f"{data}:/workspace:rw") + ']\n' +
            'extra_flags=[' + json.dumps("--label=" + label) + ']\n')
        if options.landlock:
            (project / "symbiont.toml").write_text('[sandbox]\ntier="landlock"\n[sandbox.roots]\nsource_roots=' + json.dumps([str(data)+':/workspace:ro']) + '\noutput_roots=' + json.dumps([str(data)+':/workspace:rw']) + '\n')
        if outcome == "unsafe_audit": governed.chmod(0o777)
        if outcome == "unavailable_boundary": (project / "symbiont.toml").write_text('[sandbox]\ntier="firecracker"\n')
        if outcome == "invalid_constraints":
            (project / ".symbi").mkdir()
            (project / ".symbi/constraints.toml").write_text("[malformed")
        canonical_source = None
        if canonical:
            selected = "firecracker" if outcome == "unavailable_boundary" else "docker"
            inline_rule = 'deny: "edit_file"' if outcome == "policy_denied" else 'allow: "edit_file" if invocation.arguments.path == "result.txt"'
            canonical_source = ('metadata { description = "shipping canonical fixture", executor = "orga" }\n'
                'agent writer() { capabilities = ["write"] with sandbox = "' + selected + '", timeout = 20.seconds {} policy files { ' + inline_rule + ' } }\n')
            (project / "agents").mkdir()
            (project / "agents/writer.symbi").write_text(canonical_source)
            if outcome != "unavailable_boundary":
                configuration = (project / "symbiont.toml").read_text().replace('tier="docker"', 'tier="firecracker"')
                (project / "symbiont.toml").write_text(configuration)
            row["canonical_source"] = canonical_source
            row["source_hash"] = "sha256:" + hashlib.sha256(json.dumps(canonical_source, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
        row["configuration_sha256"] = digest(project / "symbiont.toml")
        row["policy_sha256"] = digest(policy / "orchestrator.cedar")
        content = "\nLiteral $(printf unchanged); {fixture} & bytes\n"
        token = "synthetic-fixture-key"
        principal = None

        def verify(path):
            nonlocal principal
            first = json.loads(path.read_text().splitlines()[0])["payload"]
            principal = principal or first["entry"]["agent_id"]
            return verifier.verify_journal(path, public_file, observer, principal, first["run_id"])

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args): pass
            def do_POST(self):
                try:
                    assert self.path == "/v1/chat/completions"
                    assert self.headers.get("Authorization") == "Bearer " + token
                    size = int(self.headers["Content-Length"])
                    assert 0 < size < 1024 * 1024
                    request = json.loads(self.rfile.read(size))
                    journals = list(governed.glob("*.jsonl"))
                    assert len(journals) == 1
                    entries = verify(journals[0])
                    assert "Started" in entries[0]["event"]
                    if canonical:
                        binding = entries[0]["event"]["Started"]["execution_context"]["agent_definition"]
                        assert binding["name"] == "writer" and binding["source_hash"] == row["source_hash"]
                        assert binding["mode"] == "canonical_orga_conversation"
                    row["requests"].append(request)
                    assert len(row["requests"]) <= 2, "unexpected whole-turn retry"
                    if len(row["requests"]) == 1:
                        assert not (data / "result.txt").exists()
                        names = [tool["function"]["name"] for tool in request["tools"]]
                        assert "edit_file" in names and "shell" not in names
                        message = dict(role="assistant", content=None, tool_calls=[dict(id="fixture-edit", type="function",
                            function=dict(name="edit_file", arguments=json.dumps(dict(path="./result.txt", content=content))))])
                        reason = "tool_calls"
                    else:
                        tool_messages = [message for message in request["messages"] if message["role"] == "tool"]
                        assert len(tool_messages) == 1 and tool_messages[0]["tool_call_id"] == "fixture-edit"
                        if outcome == "approved":
                            assert (data / "result.txt").read_text() == content
                            batches = [entry["event"]["ToolBatchCompleted"] for entry in entries if "ToolBatchCompleted" in entry["event"]]
                            assert len(batches) == 1 and not batches[0]["observations"][0]["is_error"]
                        else:
                            assert outcome in ("denied", "policy_denied") and not (data / "result.txt").exists()
                            assert "denied" in tool_messages[0]["content"].lower()
                        message = dict(role="assistant", content="shipping shell fixture complete")
                        reason = "stop"
                    response = json.dumps(dict(id="fixture", model="fixture", choices=[dict(index=0, message=message, finish_reason=reason)],
                        usage=dict(prompt_tokens=2, completion_tokens=3, total_tokens=5))).encode()
                    self.send_response(200); self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(response))); self.end_headers()
                    self.wfile.write(response)
                except Exception as error:
                    row["errors"].append(repr(error)); self.send_error(500)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler); server.daemon_threads = True
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        state = observer / "leases"; state.mkdir(mode=0o700)
        env = dict(PATH="/usr/bin:/bin", HOME=str(project), LANG="C.UTF-8", TERM="xterm-256color", SHELL="/bin/sh",
            OPENAI_API_KEY=token, OPENAI_BASE_URL=f"http://127.0.0.1:{server.server_port}/v1", CHAT_MODEL="fixture",
            SYMBIONT_ENV="production", SYMBIONT_SANDBOX_STATE_DIR=str(state),
            SYMBIONT_SANDBOX_SUPERVISOR=str(options.supervisor), SYMBIONT_MASTER_KEY="0" * 64,
            DBUS_SESSION_BUS_ADDRESS="unix:path=/tmp/symbi-no-test-keyring")
        terminal = None
        try:
            if outcome == "invalid_constraints":
                result = subprocess.run([str(options.binary)], cwd=project, env=env, capture_output=True, text=True, timeout=5)
                assert result.returncode != 0 and "Invalid project constraints" in result.stderr
                assert not row["requests"] and not (data / "result.txt").exists()
                row["startup_error"] = result.stderr
                row["terminal_cleaned"] = True
            else:
                terminal = Terminal(options.binary, project, observer, env, options.tmux)
                terminal.shown("symbi")
                terminal.send(("@writer " if canonical else "") + "fixture request\r")
                if outcome in ("approved", "denied", "cancelled", "policy_denied"):
                    if outcome == "policy_denied":
                        terminal.shown("shipping shell fixture complete", timeout=45)
                        assert len(row["requests"]) == 2 and not row["errors"]
                        terminal.send("\x07")
                        terminal.shown("Gate · 0 held")
                        terminal.send("\x1b")
                    else:
                        wait(lambda: len(row["requests"]) == 1, "initial fixture inference")
                        terminal.send("\x07")
                        terminal.shown("Gate · 1 held")
                        terminal.send("a")
                        terminal.shown("Press Enter to review")
                        assert not (data / "result.txt").exists(), "--yes or an unreviewed shortcut bypassed approval"
                        terminal.send("\r")
                        terminal.shown("Review ")
                        review_id = re.search(r"Review ([a-f0-9]+)", terminal.frame())[1]
                        row["approval_id"] = review_id
                        terminal.review(["result.txt", "Literal", "worker_program_hash", "landlock" if options.landlock else "docker", principal])
                        if canonical:
                            terminal.review(["agent_definition", row["source_hash"]])
                            # Pending authority must retain the source/configuration snapshot.
                            (project / "agents/writer.symbi").write_text('agent writer() { with sandbox = "firecracker" {} }')
                            (project / "symbiont.toml").write_text('[sandbox]\ntier="firecracker"\n')
                        assert len(row["requests"]) == 1 and not (data / "result.txt").exists()
                        if outcome == "cancelled":
                            terminal.send("\x03")
                            terminal.shown("Cancellation requested")
                            wait(lambda: any("Terminated" in path.read_text() for path in governed.glob("*.jsonl")), "cancelled terminal audit")
                            assert len(row["requests"]) == 1 and not (data / "result.txt").exists()
                        else:
                            terminal.send("a" if outcome == "approved" else "d")
                            terminal.shown(("Approved " if outcome == "approved" else "Denied ") + review_id)
                            terminal.send("\x1b")
                            terminal.shown("shipping shell fixture complete", timeout=45)
                            assert len(row["requests"]) == 2 and not row["errors"]
                    path, = governed.glob("*.jsonl")
                    entries = verify(path)
                    assert "Terminated" in entries[-1]["event"]
                    decisions = [entry["event"]["PolicyEvaluated"] for entry in entries if "PolicyEvaluated" in entry["event"]]
                    if outcome == "approved":
                        invocation = decisions[0]["approved_calls"][0]
                        if canonical:
                            assert invocation["resolved"]["agent_definition"]["source_hash"] == row["source_hash"]
                        assert invocation["arguments"]["path"] == "result.txt"
                        assert invocation["arguments"]["content"] == content
                        resolved = invocation["resolved"]
                        assert resolved["execution_transport"] == "fixed_workspace_file_broker"
                        if options.landlock:
                            assert resolved["command_boundary"]["landlock"]["broker_source_roots"] == resolved["command_boundary"]["landlock"]["broker_output_roots"] == []
                        else:
                            assert resolved["command_boundary"]["container"]["mounts"] == []
                        assert resolved["file_access"]["operation"] == "edit_file"
                        assert resolved["command_boundary"]["filesystem"] == resolved["file_access"]
                        assert resolved["file_access"]["write"]["path"] == "result.txt"
                        assert resolved["file_access"]["write"]["mode"] == "create_no_replace"
                        assert resolved["file_access"]["write"]["sha256"] == hashlib.sha256(content.encode()).hexdigest()
                        assert (data / "result.txt").read_text() == content
                    elif outcome in ("denied", "policy_denied"):
                        assert decisions[0]["denied_count"] == 1 and not (data / "result.txt").exists()
                        if outcome == "policy_denied":
                            assert "inline policy files" in json.dumps(decisions[0])
                    else:
                        assert entries[-1]["event"]["Terminated"]["reason"]["Error"]["message"] == "agent execution cancelled"
                    row["journal_records"] = entries
                    row["journal_sha256"] = digest(path)
                    row["run_id"] = json.loads(path.read_text().splitlines()[0])["payload"]["run_id"]
                    terminal.send("/audit\r")
                    # References may extend into scrollback; capture the full rendered history.
                    wait(lambda: "Protected run references:" in terminal.call("capture-pane", "-t", "fixture:0.0", "-p", "-S", "-").stdout,
                         "public audit reference")
                    history = terminal.call("capture-pane", "-t", "fixture:0.0", "-p", "-S", "-").stdout
                    assert row["run_id"] in history and row["pinned_public_key"] in history, "Missing public audit reference in rendered history"
                    row["audit_display"] = history[-100000:]
                else:
                    terminal.shown("Agent 'writer' error:" if canonical else "Orchestrator error:")
                    assert not row["requests"] and not (data / "result.txt").exists()
            row["passed"] = True
        except Exception as error:
            row["errors"].append(repr(error)); row["passed"] = False
        finally:
            if terminal is not None and any("Terminated" not in path.read_text() for path in governed.glob("*.jsonl")):
                try:
                    terminal.send("\x03")
                    wait(lambda: all("Terminated" in path.read_text() for path in governed.glob("*.jsonl")),
                         "cancelled turn cleanup before terminal shutdown", timeout=45)
                except Exception as error:
                    row["cleanup_wait_error"] = repr(error)
                    row["errors"].append(repr(error))
            row["observed_journals"] = {path.name: path.read_text() for path in governed.glob("*.jsonl")}
            if terminal is not None:
                row["final_scrollback"] = terminal.call("capture-pane", "-t", "fixture:0.0", "-p", "-S", "-", check=False).stdout[-100000:]
                row["frames"] = terminal.frames
                try: terminal.close(); row["terminal_cleaned"] = True
                except Exception as error: row["errors"].append(repr(error)); row["terminal_cleaned"] = False
            server.shutdown(); server.server_close(); thread.join(timeout=5)
            leftovers = [] if options.landlock else subprocess.check_output(["docker", "ps", "-a", "--filter", "label=" + label, "--format", "{{.ID}}"], text=True, timeout=5).splitlines()
            row["leftover_workers"] = leftovers
            row["leftover_leases"] = [path.name for path in state.glob("*.json")]
            row["passed"] &= row.get("terminal_cleaned", False) and not leftovers and not row["leftover_leases"] and not row["errors"]
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--landlock", action="store_true")
    parser.add_argument("--case", action="append")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--tmux", default=shutil.which("tmux"))
    options = parser.parse_args()
    options.binary = options.binary.resolve(strict=True); options.supervisor = options.supervisor.resolve(strict=True)
    assert options.tmux and os.getuid() != 0
    companion = Path(__file__).with_name("test-direct-inference.py")
    spec = importlib.util.spec_from_file_location("audit_fixture", companion)
    verifier = importlib.util.module_from_spec(spec); spec.loader.exec_module(verifier)
    before = {str(path): digest(path) for path in (options.binary, options.supervisor, Path(__file__), companion, Path(options.tmux))}
    planned = ["approved", "denied", "cancelled", "unsafe_audit", "unavailable_boundary", "invalid_constraints", "canonical_approved", "canonical_denied", "canonical_unavailable_boundary", "canonical_policy_denied"]
    if options.case:
        assert set(options.case).issubset(planned)
        planned = [case for case in planned if case in options.case]
    if options.landlock:
        assert not any(case.startswith('canonical_') for case in planned), 'native cases inherit project configuration'
    report = dict(passed=False, containment_claim=False, identities=before, planned=planned, cases=[])
    for case in planned:
        row = run_case(case, options, verifier); report["cases"].append(row)
        options.report.write_text(json.dumps(report, indent=2) + "\n")
        print(case + ": " + ("passed" if row["passed"] else "FAILED"), flush=True)
        if not row["passed"]: raise SystemExit(1)
    report["passed"] = len(report["cases"]) == len(planned) and all(row["passed"] for row in report["cases"])
    assert before == {path: digest(Path(path)) for path in before}
    options.report.write_text(json.dumps(report, indent=2) + "\n")

if __name__ == "__main__": main()

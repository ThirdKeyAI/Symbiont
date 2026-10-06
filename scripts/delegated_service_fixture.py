"""Disposable systemd user service for local delegated-worker E2E tests."""
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time
import uuid


def eventually(check, timeout=15):
    deadline = time.monotonic() + timeout
    while True:
        result = check()
        if result:
            return result
        assert time.monotonic() < deadline, 'fixture condition timed out'
        time.sleep(0.02)


def protocol_identity():
    root = Path(__file__).resolve().parents[1] / 'crates/sandbox-supervisor'
    digest = hashlib.sha256()
    def visit(relative):
        path = root / relative
        if path.is_dir():
            for child in sorted(path.iterdir()):
                visit(child.relative_to(root))
        else:
            digest.update(str(relative).encode())
            digest.update(path.read_bytes())
    for relative in ['src', 'Cargo.toml', 'build.rs']:
        visit(Path(relative))
    version = int(re.search(r'pub const VERSION: u32 = (\d+);', (root/'src/protocol.rs').read_text())[1])
    return dict(version=version, implementation=digest.hexdigest())


class Connection:
    def __init__(self, state, request):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(20)
        self.socket.connect(str(state/'supervisor.sock'))
        self.reader = self.socket.makefile('rb')
        self.socket.sendall(json.dumps(request).encode() + b'\n')

    def receive(self):
        frame = self.reader.readline(1024*1024+1)
        assert frame and len(frame) <= 1024*1024, 'missing or oversized supervisor response'
        return json.loads(frame)

    def cancel(self):
        self.socket.shutdown(socket.SHUT_WR)

    def close(self):
        self.reader.close()
        self.socket.close()


class DelegatedService:
    def __init__(self, binary, state):
        self.binary, self.state = Path(binary), Path(state)
        self.unit = 'symbi-delegated-e2e-' + uuid.uuid4().hex + '.service'
        self.protocol = protocol_identity()
        self.cgroup = None

    def command(self, *arguments, check=True):
        return subprocess.run(['systemctl', '--user', *arguments], capture_output=True,
                              text=True, check=check, timeout=20)

    def start(self):
        self.state.mkdir(mode=0o700, parents=True, exist_ok=True)
        result = subprocess.run([
            'systemd-run', '--user', '--collect', '--unit='+self.unit,
            '--property=Type=notify', '--property=WatchdogSec=5s',
            '--property=TimeoutStopSec=5s', '--property=KillMode=control-group',
            '--property=SendSIGKILL=yes', '--property=Delegate=cpu memory pids',
            '--property=DelegateSubgroup=manager', '--property=RuntimeMaxSec=180',
            str(self.binary), '__sandbox_supervisor', '--state-dir', str(self.state), '--delegated-workers',
        ], capture_output=True, text=True, timeout=25)
        if result.returncode:
            logs = subprocess.run(['journalctl', '--user', '-u', self.unit, '-n', '20', '--no-pager'],
                                  capture_output=True, text=True, timeout=10)
            self.stop()
            raise AssertionError(result.stderr + logs.stdout)
        relative = self.command('show', self.unit, '--property=ControlGroup', '--value').stdout.strip()
        self.cgroup = Path('/sys/fs/cgroup') / relative.lstrip('/')
        assert self.cgroup.name == self.unit
        assert self.request(dict(operation='ping', **self.protocol))['delegated_workers'] is True
        return self

    def request(self, request):
        connection = Connection(self.state, request)
        try:
            return connection.receive()
        finally:
            connection.close()

    def capacity(self):
        reply = self.request(dict(operation='inspect', lease=None, **self.protocol))
        assert reply['status'] == 'capacity', reply
        return reply['snapshot']

    def pid(self):
        pid = int(self.command('show', self.unit, '--property=MainPID', '--value').stdout)
        assert pid > 1
        return pid

    def stop(self):
        result = self.command('stop', self.unit, check=False)
        state = self.command('show', self.unit, '--property=ActiveState', '--value', check=False).stdout.strip()
        assert state in ('inactive', 'failed', ''), (result.stderr, state)
        if self.cgroup and self.cgroup.exists():
            assert 'populated 0' in (self.cgroup/'cgroup.events').read_text()
        self.command('reset-failed', self.unit, check=False)

    def __enter__(self):
        return self.start()

    def __exit__(self, *_):
        self.stop()


class AutomaticDelegatedService(DelegatedService):
    """Use shipping doctor to bootstrap the service, without writing a unit."""

    def start(self, project=None):
        self.unit = 'symbi-workers-' + hashlib.sha256(os.fsencode(self.state)).hexdigest() + '.service'
        if project is None:
            project = self.state.parent/'automatic-service-check'
            project.mkdir(mode=0o700)
            (project/'symbiont.toml').write_text('[sandbox]\ntier="landlock"\n')
        environment = {key: value for key, value in os.environ.items()
                       if key in ('HOME', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS')}
        environment.update(PATH='/usr/bin:/bin', SYMBIONT_SANDBOX_STATE_DIR=str(self.state))
        try:
            result = subprocess.run([str(self.binary), 'doctor'], cwd=project, env=environment,
                                    capture_output=True, text=True, timeout=35)
            self.doctor_output = result.stdout + result.stderr
            assert result.returncode == 0, result.stdout + result.stderr
            relative = self.command('show', self.unit, '--property=ControlGroup', '--value').stdout.strip()
            self.cgroup = Path('/sys/fs/cgroup') / relative.lstrip('/')
            assert self.cgroup.name == self.unit
            assert self.request(dict(operation='ping', **self.protocol))['delegated_workers'] is True
            return self
        except Exception:
            self.stop()
            raise

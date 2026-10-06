#!/usr/bin/env python3
"""Shipping init/doctor E2E on a real Linux desktop; no manually installed unit.

Requires ABI 6+, systemd user delegation, and the built binary. Missing host
capabilities fail this test; they are never counted as skipped or successful.
Only disposable synthetic projects and supervisor pools are used.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading
import time
import traceback

from delegated_service_fixture import Connection, protocol_identity, eventually


def main():
    if not __debug__:
        raise SystemExit('Run this E2E with Python assertions enabled')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--managed-executable', type=Path, help='Also diagnose this installed CLI without model calls')
    parser.add_argument('--development', action='store_true', help='Also verify a declared-source coding task')
    parser.add_argument('--install-dir', type=Path, help='Install a copy into a new directory before testing')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.install_dir is not None:
        assert not args.install_dir.exists(), 'Choose a new installation directory'
        args.install_dir.mkdir(parents=True)
        installed = args.install_dir/'symbi'
        shutil.copy2(binary, installed)
        binary = installed.resolve(strict=True)
    with binary.open('rb') as stream:
        binary_hash = hashlib.file_digest(stream, 'sha256').hexdigest()
    report = {'binary': str(binary), 'sha256': binary_hash,
              'kernel': platform.release(), 'machine': platform.machine(),
              'checks': [], 'success': False}
    units = []

    def systemctl(*arguments, check=True):
        return subprocess.run(['/usr/bin/systemctl', '--user', *arguments],
                              capture_output=True, text=True, timeout=20, check=check)

    try:
        with ExitStack() as stack:
            temporary = stack.enter_context(tempfile.TemporaryDirectory(prefix='symbi-onboarding-'))
            root = Path(temporary)
            project = root / 'project'
            # Literal spaces and expansion characters must survive service launch.
            state = root / 'pool $literal%'
            unit = 'symbi-workers-' + hashlib.sha256(os.fsencode(state)).hexdigest() + '.service'
            units.append(unit)
            stack.callback(systemctl, 'stop', unit, check=False)
            env = {key: value for key, value in os.environ.items()
                   if key in ('PATH', 'HOME', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS')}
            env['SYMBIONT_SANDBOX_STATE_DIR'] = str(state)
            env['SYMBIONT_ENV'] = 'development'
            binaries = root/'bin'
            binaries.mkdir()
            docker_marker = root/'docker-called'
            docker = binaries/'docker'
            docker.write_text('#!/bin/sh\nprintf called > "$DOCKER_TEST_MARKER"\nexit 77\n')
            docker.chmod(0o700)
            env['PATH'] = str(binaries) + ':/usr/bin:/bin'
            env['DOCKER_TEST_MARKER'] = str(docker_marker)

            def run(*arguments, cwd=project, success=True):
                result = subprocess.run([str(binary), *arguments], cwd=cwd, env=env,
                                        capture_output=True, text=True, timeout=40)
                assert (result.returncode == 0) == success, result.stdout + result.stderr
                return result

            run('init', '--sandbox', 'landlock', '--dir', str(project), cwd=root)
            assert not (project/'docker-compose.yml').exists()
            config = project/'symbiont.toml'
            original = config.read_text()
            assert 'tier = "landlock"' in original
            source = (project/'agents/assistant.symbi').read_text()
            assert 'sandbox =' not in source
            run('dsl', '--check', '-f', 'agents/assistant.symbi')
            report['checks'].append('fresh scaffold selects Landlock and inherits it without Docker Compose')

            # An explicit external-service owner is authoritative: doctor cannot
            # create a replacement, even when that service is absent.
            config.write_text(original + f'\n[sandbox.landlock.supervisor]\nservice_uid = {os.geteuid()}\n')
            denied = run('doctor', success=False)
            assert 'managed sandbox supervisor unavailable' in denied.stdout + denied.stderr
            assert not (state/'supervisor.sock').exists()
            report['checks'].append('missing explicitly managed service refuses automatic replacement')
            config.write_text(original)

            with ThreadPoolExecutor(max_workers=3) as executor:
                results = list(executor.map(lambda _: run('doctor'), range(3)))
            report['doctor'] = results[0].stdout
            assert all('cleanup confirmed' in result.stdout for result in results)
            assert all('Checking Docker' not in result.stdout for result in results)
            assert all('Private native workspace is writable' in result.stdout for result in results)
            assert all('Private loopback and both inherited connections work' in result.stdout for result in results)
            assert all('Managed CLI startup not checked: no [managed_cli]' in result.stdout for result in results)
            control_group = systemctl('show', unit, '--property=ControlGroup', '--value').stdout.strip()
            group = Path('/sys/fs/cgroup') / control_group.lstrip('/')
            assert group.name == unit and (group/'manager').is_dir()
            assert not list(group.glob('worker-*')), 'doctor left worker cgroups behind'
            connection = Connection(state, dict(operation='inspect', lease=None, **protocol_identity()))
            try:
                capacity = connection.receive()
            finally:
                connection.close()
            assert capacity['status'] == 'capacity' and capacity['snapshot']['reserved']['workers'] == 0
            report['checks'].append('concurrent first launches share one automatic delegated unit and clean every worker')

            config.write_text(original + '\n[sandbox.landlock]\nabi_floor = 99\n')
            denied = run('doctor', success=False)
            assert '99' in denied.stdout + denied.stderr
            report['checks'].append('unsupported kernel requirement fails without another backend')
            config.write_text(original)

            systemctl('stop', unit)
            assert not group.exists(), 'stopped service cgroup survives'
            run('doctor')
            report['checks'].append('a later invocation recreates the stopped automatic service')

            # Deny real namespace syscalls in the shipping setup helper. The
            # basic Landlock probe still succeeds; doctor must name the later
            # workspace or network failure and clean the admitted worker.
            wrapper = binaries/'restricted-helper'
            syscall = {'x86_64': 272, 'aarch64': 97}[platform.machine()]
            for mask, stage in [(0x10000000, 'Native workspace'), (0x40000000, 'Managed CLI transport')]:
                wrapper.write_text("#!/usr/bin/python3\n" + f"""
import ctypes, os, sys
class Filter(ctypes.Structure):
    _fields_=[('code',ctypes.c_ushort),('jt',ctypes.c_ubyte),('jf',ctypes.c_ubyte),('k',ctypes.c_uint)]
class Program(ctypes.Structure):
    _fields_=[('length',ctypes.c_ushort),('filter',ctypes.POINTER(Filter))]
filters=(Filter*6)(Filter(0x20,0,0,0),Filter(0x15,0,3,{syscall}),Filter(0x20,0,0,16),Filter(0x45,0,1,{mask}),Filter(6,0,0,0x50000|13),Filter(6,0,0,0x7fff0000))
program=Program(6,filters)
libc=ctypes.CDLL(None,use_errno=True)
assert libc.prctl(38,1,0,0,0)==0
assert libc.prctl(22,2,ctypes.byref(program),0,0)==0
os.execv({str(binary)!r},[{str(binary)!r},*sys.argv[1:]])
""")
                wrapper.chmod(0o700)
                env['SYMBIONT_LANDLOCK_WORKER'] = str(wrapper)
                denied = run('doctor', success=False)
                assert 'Restricted worker executed' in denied.stdout
                assert stage+':' in denied.stderr and 'namespace' in denied.stderr
                if stage == 'Managed CLI transport':
                    assert 'Private native workspace is writable' in denied.stdout
                eventually(lambda: not list(state.glob('*.json')) and not list((state/'staging').glob('*')))
                report['checks'].append(stage+' denial is diagnosed without a fallback or retained worker')
            del env['SYMBIONT_LANDLOCK_WORKER']

            # The exact executable grant must survive a user-local installation,
            # without granting host source, credentials or broker descriptors.
            args.report.parent.mkdir(parents=True, exist_ok=True)
            cli_directory = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix='doctor-cli-', dir=args.report.resolve().parent)))
            assert not cli_directory.is_relative_to('/tmp'), 'Store this report outside /tmp for the executable grant test'
            executable = cli_directory/'cli'
            canary = cli_directory/'host-canary'; canary.write_text('private host data')
            env['DOCTOR_SECRET_CANARY'] = 'must not enter the CLI'
            executable.write_text("#!/usr/bin/python3\n" + f"""
import os, pathlib, sys
assert sys.argv[1:] == ['--version']
assert os.getcwd() == '/tmp/symbi-workspace'
assert os.environ['HOME'] == '/tmp/symbi-home'
assert 'DOCTOR_SECRET_CANARY' not in os.environ
assert 'SYMBI_TOOLS_FD' not in os.environ and 'SYMBI_INFERENCE_FD' not in os.environ
assert pathlib.Path('/proc/self/maps').read_text()
assert pathlib.Path('/proc/self/stat').read_text().startswith(str(os.getpid())+' ')
try: pathlib.Path({str(canary)!r}).read_text()
except OSError: pass
else: raise AssertionError('host sibling was exposed')
print('doctor-fixture 1.0' + chr(27) + '[2J')
""")
            executable.chmod(0o700)
            config.write_text(original+'\n[managed_cli]\nexecutable='+json.dumps(str(executable))+'\n')
            result = run('doctor')
            assert 'Managed CLI --version:' in result.stdout and 'doctor-fixture 1.0' in result.stdout
            assert '\x1b' not in result.stdout and r'\u{1b}' in result.stdout
            report['checks'].append('configured CLI starts through private adapters with scoped metadata and escaped version output')
            executable.write_text('#!/bin/sh\necho incompatible-toolchain >&2\nexit 23\n')
            denied = run('doctor', success=False)
            assert 'Managed CLI startup:' in denied.stderr and 'incompatible-toolchain' in denied.stderr
            assert 'executable selects a compatible standalone CLI' in denied.stderr
            report['checks'].append('CLI startup failures include the toolchain remedy')
            executable.write_text('#!/bin/sh\nsleep 60 &\nwait\n')
            started = time.monotonic()
            denied = run('doctor', success=False)
            elapsed = time.monotonic() - started
            report['hung_cli'] = dict(stdout=denied.stdout, stderr=denied.stderr, elapsed_seconds=elapsed)
            assert 'Managed CLI startup:' in denied.stderr and any(reason in denied.stderr for reason in ('timed out', 'terminated', 'lifetime expired')), denied.stdout + denied.stderr
            assert elapsed < 20, elapsed
            eventually(lambda: not list(state.glob('*.json')) and not list((state/'staging').glob('*')))
            assert not list(group.glob('worker-*'))
            report['checks'].append('hung CLI and descendants are bounded and cleaned before doctor fails')
            config.write_text(original+'\n[managed_cli]\nexecutable="/missing/doctor-cli"\n')
            denied = run('doctor', success=False)
            assert 'Managed CLI configuration:' in denied.stderr and 'Set [managed_cli] executable' in denied.stderr
            report['checks'].append('invalid configured executable fails with a specific configuration remedy')
            if args.managed_executable:
                installed_cli = args.managed_executable.resolve(strict=True)
                config.write_text(original+'\n[managed_cli]\nexecutable='+json.dumps(str(installed_cli))+'\n')
                result = run('doctor')
                assert 'Managed CLI --version:' in result.stdout
                with installed_cli.open('rb') as stream:
                    report['managed_cli'] = dict(executable=str(installed_cli), sha256=hashlib.file_digest(stream, 'sha256').hexdigest(), diagnostic=result.stdout)
                report['checks'].append('installed real CLI passes the contained startup check without provider configuration')
            config.write_text(original)

            development = {'active': False}
            class Provider(BaseHTTPRequestHandler):
                def log_message(self, *_):
                    pass

                def do_POST(self):
                    length = int(self.headers.get('Content-Length', '0'))
                    assert self.path == '/v1/chat/completions' and 0 < length < 1024 * 1024
                    request = json.loads(self.rfile.read(length))
                    assert request['model'] == 'onboarding-fixture'
                    report['provider_requests'] = report.get('provider_requests', 0) + 1
                    message, finish = dict(role='assistant', content='Fresh project is ready.'), 'stop'
                    if development['active']:
                        observations = [m for m in request['messages'] if m['role'] == 'tool']
                        if not observations:
                            message = dict(role='assistant', content=None, tool_calls=[dict(id='verify', type='function', function=dict(name='verify_module', arguments=json.dumps(dict(input='calculator.py', output='test-result.json'))))])
                            finish = 'tool_calls'
                        else:
                            result = json.loads(observations[0]['content'])
                            assert result['status'] == 'success' and result['results']['tests_passed'] == 2
                            message = dict(role='assistant', content='Coding example verified.')
                    body = json.dumps(dict(id='fixture', model='onboarding-fixture', choices=[dict(
                        index=0, message=message, finish_reason=finish)], usage=dict(prompt_tokens=1,
                        completion_tokens=1,total_tokens=2))).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

            provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
            stack.callback(provider.server_close)
            stack.callback(provider.shutdown)
            threading.Thread(target=provider.serve_forever, daemon=True).start()
            env.update(OPENAI_API_KEY='synthetic-key', CHAT_MODEL='onboarding-fixture',
                       OPENAI_BASE_URL=f'http://127.0.0.1:{provider.server_port}/v1')
            response = run('run', 'assistant', '--input', 'Confirm this fresh project is ready.',
                           '--max-iterations', '2')
            assert 'Fresh project is ready.' in response.stdout
            assert report['provider_requests'] == 1
            journals = list((project/'.symbiont/governed').glob('*.jsonl'))
            assert len(journals) == 1
            entries = [json.loads(line)['payload']['entry'] for line in journals[0].read_text().splitlines()]
            assert entries[-1]['event']['Terminated']['reason'] == 'Completed'
            public_key = re.search(r'Audit public key: ([0-9a-f]{64})', response.stderr)[1]
            run_id = journals[0].stem.split('.')[1]
            run('audit', 'inspect', str(journals[0]), '--run-id', run_id, '--public-key', public_key)
            report['checks'].append('generated agent completes a locally served run with a verified signed terminal journal')
            if args.development:
                code = root/'code'; code.mkdir()
                (project/'tools').mkdir(exist_ok=True)
                program = "import json,pathlib,sys\ndef total(values): return sum(values)\nassert total([2,3,5]) == 10\nassert total([]) == 0\nresult={'tests_passed':2}\npathlib.Path(sys.argv[1]).write_text(json.dumps(result))\nprint(json.dumps(result))\n"
                (code/'calculator.py').write_text(program)
                config.write_text(original + '\n[sandbox.roots]\nsource_roots=' + json.dumps([str(code)+':/workspace:ro']) + '\noutput_roots=' + json.dumps([str(code)+':/workspace:rw']) + '\n')
                (project/'agents/coding.symbi').write_text('agent coding() {}\n')
                (project/'policies/coding.cedar').write_text('permit(principal,action,resource);\n')
                (project/'tools/verify_module.clad.toml').write_text('[tool]\nname="verify_module"\nversion="1"\nbinary="python3"\ndescription="Test the declared calculator source"\n[args.input]\nposition=1\nrequired=true\ntype="path"\n[args.output]\nposition=2\nrequired=true\ntype="path"\n[command]\ntemplate="python3 -I {input} {output}"\n[filesystem]\nread=["{input}"]\ncreate=["{output}"]\nmax_file_bytes=4096\n[output]\nformat="json"\n')
                development['active'] = True
                previous = set(journals)
                response = run('run','coding','--input','Verify the calculator source and publish its test result.','--max-iterations','3')
                assert 'Coding example verified.' in response.stdout
                assert json.loads((code/'test-result.json').read_text()) == {'tests_passed':2}
                assert (code/'calculator.py').read_text() == program
                journals = set((project/'.symbiont/governed').glob('*.jsonl')) - previous
                assert len(journals) == 1 and report['provider_requests'] == 3
                journal = journals.pop()
                run_id = journal.stem.split('.')[1]
                public_key = re.search(r'Audit public key: ([0-9a-f]{64})', response.stderr)[1]
                run('audit','inspect',str(journal),'--run-id',run_id,'--public-key',public_key)
                eventually(lambda: not list(state.glob('*.json')) and not list((state/'staging').glob('*')))
                report['coding'] = dict(source_sha256=hashlib.sha256(program.encode()).hexdigest(),result=json.loads((code/'test-result.json').read_text()),journal_sha256=hashlib.sha256(journal.read_bytes()).hexdigest())
                report['checks'].append('fresh installed project tests declared Python source and publishes its result after supervised cleanup with a verified journal')
            assert not docker_marker.exists(), 'Landlock onboarding invoked Docker'
            report['success'] = True
    except Exception:
        report['error'] = traceback.format_exc()
    finally:
        for unit in units:
            try:
                systemctl('stop', unit, check=False)
                systemctl('reset-failed', unit, check=False)
            except Exception:
                report['success'] = False
                report['cleanup_error'] = traceback.format_exc()
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'success': report['success'], 'checks': report['checks'], 'report': str(args.report)}))
    if not report['success']:
        print(report.get('error', report.get('cleanup_error', 'validation failed')))
        raise SystemExit(1)


if __name__ == '__main__':
    main()

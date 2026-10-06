#!/usr/bin/env python3
"""Shipping Git queries against synthetic repositories and protected observations.

Requires a cached image containing Git and Python. The observer records actual
Docker mounts and snapshot hashes before payload start; it never enters a worker.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import runpy
import shutil
import shlex
from file_grant_backend import Backend, arguments as backend_arguments
from worker_origin_observer import verify_origin
import signal
import subprocess
import tempfile
import threading
import time
import traceback
import uuid


def digest(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def main():
    assert __debug__
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--image', default='symbi-managed-real-e2e:local')
    backend_arguments(parser)
    parser.add_argument('--case', action='append', help='Run only selected case names')
    args = parser.parse_args()
    vm = args.firecracker_binary is not None
    native = args.landlock
    backend = None
    assert not args.report.exists(), 'Retain previous reports; choose a new path'
    binary = args.binary.resolve(strict=True)
    repo = Path(__file__).resolve().parent.parent
    root = Path(tempfile.mkdtemp(prefix='symbi-git-source-'))
    for name in ['source', 'observer', 'home', 'agents', 'tools', 'policies', 'state', 'docker-client', '.symbiont/governed']:
        (root / name).mkdir(mode=0o700, parents=True, exist_ok=True)
    observer, source = root / 'observer', root / 'source'
    label = 'symbi.git-source=' + uuid.uuid4().hex
    report = {'passed': False, 'fixture': str(root), 'binary_sha256': digest(binary),
              'driver_sha256': digest(__file__), 'origin_observer_sha256': digest(Path(__file__).with_name('worker_origin_observer.py')),
              'cases': [], 'provider_errors': []}
    denv = {'PATH': '/usr/bin:/bin', 'DOCKER_HOST': 'unix:///var/run/docker.sock', 'DOCKER_CONFIG': str(root / 'docker-client')}
    provider, process = None, None
    requests = []

    def docker(*argv):
        return subprocess.check_output(['docker', *argv], env=denv, text=True, timeout=15).strip()

    def git(*argv):
        return subprocess.check_output(['/usr/bin/git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                                       '-c', 'core.hooksPath=/dev/null', *argv], cwd=source,
            env={'PATH': '/usr/bin:/bin', 'HOME': str(root / 'home'), 'GIT_CONFIG_NOSYSTEM': '1'}, stderr=subprocess.PIPE, text=True, timeout=15)

    def setup(object_format='sha1'):
        # Only this freshly created, known configuration is passed to host Git.
        # Host Git is never run after the hostile configuration is installed.
        if source.exists():
            shutil.rmtree(source)
        source.mkdir(mode=0o700)
        git('init', '--initial-branch=main', '--object-format=' + object_format)
        (source / 'tracked.txt').write_text('base\n')
        (source / 'link').write_text('plain link baseline\n')
        (source / '.gitattributes').write_text('tracked.txt diff=trap filter=trap\n')
        git('add', '.')
        git('commit', '-m', 'Known fixture baseline')
        git('gc', '--prune=now')
        (source / 'tracked.txt').write_text('staged\n')
        git('add', 'tracked.txt')
        (source / 'tracked.txt').write_text('unstaged\n')
        (source / 'untracked.txt').write_text('useful untracked file\n')
        (source / 'link').unlink()
        (source / 'link').symlink_to(observer / 'canary')
        (source / 'helper').write_text('#!/bin/sh\nprintf HELPER_EXECUTED\nexit 23\n')
        (source / 'helper').chmod(0o755)
        (source / 'json.py').write_text('raise RuntimeError("PYTHON_IMPORT_HIJACK")\n')
        with (source / '.git/config').open('a') as f:
            f.write('[include]\npath=/symbi-git-input/configuration\n'
                    '[core]\nworktree=/tmp/unrelated\nfsmonitor=/symbi-source/helper\nhooksPath=/symbi-source/helper\n'
                    '[diff "trap"]\ncommand=/symbi-source/helper\ntextconv=/symbi-source/helper\n'
                    '[filter "trap"]\nclean=/symbi-source/helper\nrequired=true\n')

    try:
        image = None if vm or native else docker('image', 'inspect', args.image, '--format', '{{.Id}}')
        report['image'] = image
        (observer / 'canary').write_text('PRIVATE OBSERVER CANARY')
        wrapper = observer / 'docker-client'
        wrapper.write_text('''#!/usr/bin/python3
import hashlib,json,os,pathlib,subprocess,sys,time
root=pathlib.Path(%s)
if sys.argv[1]=='create':
    p=subprocess.run(['/usr/bin/docker',*sys.argv[1:]],capture_output=True)
    if p.returncode==0:
        identity=p.stdout.decode().strip()
        value=json.loads(subprocess.check_output(['/usr/bin/docker','inspect',identity]))[0]
        lease=value['Config']['Labels']['ai.symbiont.lease']
        record=json.loads((root.parent/'state'/(lease+'.json')).read_text())
        assert record['lease']==lease
        snapshots={}
        for mount in value['Mounts']:
            if mount['Type']!='bind':continue
            prefix='input' if mount['Destination']=='/symbi-git-input' else 'worktree'
            for path in pathlib.Path(mount['Source']).rglob('*'):
                if path.is_symlink():data=os.fsencode(os.readlink(path))
                elif path.is_file():data=path.read_bytes()
                else:continue
                snapshots[prefix+'/'+str(path.relative_to(mount['Source']))]=hashlib.sha256(data).hexdigest()
        (root/('worker-'+identity+'.json')).write_text(json.dumps(dict(id=identity,lease=lease,origin=record['origin'],mounts=value['Mounts'],network=value['HostConfig']['NetworkMode'],user=value['Config']['User'],snapshots=snapshots)))
        if (root/'pause-create').exists():
            end=time.monotonic()+15
            (root/'created').touch()
            while not (root/'release').exists() and time.monotonic()<end:time.sleep(.02)
    sys.stdout.buffer.write(p.stdout);sys.stderr.buffer.write(p.stderr);sys.exit(p.returncode)
os.execv('/usr/bin/docker',['docker',*sys.argv[1:]])
''' % repr(str(observer)))
        wrapper.chmod(0o700)
        (root / '.env').write_text('')
        (root / 'agents/fixture.symbi').write_text('agent fixture() { with timeout = 30.seconds {} }\n')
        report['manifests'] = {}
        for name in ['git_diff', 'git_staged_diff', 'git_log', 'git_status']:
            path = repo / 'tools' / (name + '.clad.toml')
            (root / 'tools' / path.name).write_bytes(path.read_bytes())
            report['manifests'][name] = digest(path)
        if native:
            backend = Backend(args,root,source,label,docker,report)
            wrapper = observer/'native-launcher'
            wrapper.write_text("""#!/usr/bin/python3
import hashlib,json,os,pathlib,sys,time
root=pathlib.Path(%r)
spec=json.loads(pathlib.Path(sys.argv[2]).read_text())
records=[json.loads(p.read_text()) for p in (root/'state').glob('*.json')]
record=next(r for r in records if pathlib.Path(sys.argv[2]).parent.parent.name in r['staging'])
snapshots={}
for mount in spec['mounts']:
    prefix='input' if mount['destination']=='/tmp/symbi-git-input' else 'worktree'
    source=pathlib.Path(mount['source'])
    for path in source.rglob('*'):
        if path.is_file() or path.is_symlink():
            data=os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes()
            snapshots[prefix+'/'+str(path.relative_to(source))]=hashlib.sha256(data).hexdigest()
(root/'observer'/('worker-'+record['lease']+'.json')).write_text(json.dumps(dict(id=record['lease'],origin=record['origin'],snapshots=snapshots,launcher_pid=os.getpid())))
(root/'observer/created').touch()
if (root/'observer/pause-create').exists():
    end=time.monotonic()+15
    while not (root/'observer/release').exists() and time.monotonic()<end:time.sleep(.002)
os.execv(%r,[%r,*sys.argv[1:]])
""" % (str(root),str(binary),str(binary)))
            wrapper.chmod(0o700)
        elif vm:
            source.mkdir(mode=0o700,exist_ok=True)
            launcher=observer/'firecracker-launcher'
            helper=Path(__file__).with_name('git_snapshot_observer.py').resolve()
            launcher.write_text('#!/bin/sh\nexec '+shlex.join(['/usr/bin/python3',str(helper),str(root),str(args.firecracker_binary.resolve(strict=True))])+' "$@"\n')
            launcher.chmod(0o700)
            report['snapshot_observer_sha256']=digest(helper)
            backend=Backend(args,root,source,label,docker,report,vmm_launcher=launcher)
            config=root/'symbiont.toml'
            config.write_text('\n'.join('output_roots=[]' if line.startswith('output_roots=') else line for line in config.read_text().splitlines())+'\n')
            report['config_sha256']=digest(config)
        else:
            (root / 'symbiont.toml').write_text('[sandbox]\ntier="docker"\n[sandbox.docker]\nworking_dir="/source"\nimage=' + json.dumps(image) +
                '\ndocker_binary=' + json.dumps(str(wrapper)) + '\nvolumes=' + json.dumps([str(source) + ':/source:ro']) + '\nextra_flags=' + json.dumps(['--label=' + label]) + '\n')
        seed = os.urandom(32)
        key = root / '.symbiont/governed/audit-signing.key'
        key.write_bytes(seed)
        key.chmod(0o600)
        public = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'],
            input=bytes.fromhex('302e020100300506032b657004220420') + seed, capture_output=True, check=True, timeout=5).stdout
        (observer / 'public.der').write_bytes(public)
        report['public_key'] = public[-32:].hex()
        helper = Path(__file__).with_name('test-direct-inference.py')
        verifier = runpy.run_path(str(helper))['verify_journal']
        report['verifier_sha256'] = digest(helper)

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    size = int(self.headers.get('Content-Length', '0'))
                    assert self.path == '/v1/chat/completions' and 0 < size <= 1024 * 1024
                    request = json.loads(self.rfile.read(size))
                    requests.append(request)
                    assert len(requests) <= 4
                    if not any(m['role'] == 'tool' for m in request['messages']):
                        tool = next(m['content'] for m in request['messages'] if m['role'] == 'user')
                        message = {'role': 'assistant', 'content': None, 'tool_calls': [{'id': 'query', 'type': 'function',
                            'function': {'name': tool, 'arguments': '{}'}}]}
                        finish = 'tool_calls'
                    else:
                        message, finish = {'role': 'assistant', 'content': 'fixture complete'}, 'stop'
                    response = json.dumps({'id': 'fixture', 'object': 'chat.completion', 'model': 'fixture',
                        'choices': [{'index': 0, 'message': message, 'finish_reason': finish}],
                        'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}}).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(response)))
                    self.end_headers()
                    self.wfile.write(response)
                except Exception as error:
                    report['provider_errors'].append(str(error))
                    self.send_error(400)

        provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        provider.daemon_threads = True
        threading.Thread(target=provider.serve_forever, daemon=True).start()
        env = {**denv, 'HOME': str(root / 'home'), 'LANG': 'C.UTF-8', 'SYMBIONT_ENV': 'production',
               'SYMBIONT_SANDBOX_STATE_DIR': str(root / 'state'), 'OPENAI_API_KEY': 'synthetic-key',
               'CHAT_MODEL': 'fixture', 'OPENAI_BASE_URL': f'http://127.0.0.1:{provider.server_port}/v1'}
        if native: env['SYMBIONT_LANDLOCK_WORKER'] = str(wrapper)
        cases = [('log', 'git_log'), ('staged', 'git_staged_diff'), ('diff', 'git_diff'), ('status', 'git_status'),
                 ('sha256', 'git_log'), ('snapshot', 'git_diff'), ('denied', 'git_diff'),
                 ('alternates', 'git_log'), ('pointer', 'git_log'), ('oversize', 'git_diff'),
                 ('extension', 'git_log'), ('capacity', 'git_diff'), ('prepare_crash', 'git_diff'), ('crash', 'git_diff')]
        if args.case:
            assert set(args.case).issubset({name for name,_ in cases}), 'unknown case'
            cases=[(name,tool) for name,tool in cases if name in args.case]
        for name, tool in cases:
            setup('sha256' if name == 'sha256' else 'sha1')
            quota = root / 'state/staging.conf'
            quota.write_text(json.dumps({'max_snapshots': 1 if name == 'capacity' else 16, 'reserved_bytes': 1024 * 1024 * 1024}))
            quota.chmod(0o600)
            sibling_journals = set()
            requests.clear()
            for file in observer.glob('worker-*.json'):
                file.unlink()
            for flag in ['created', 'pause-create', 'release']:
                (observer / flag).unlink(missing_ok=True)
            if name in ['snapshot', 'crash', 'capacity'] or (native and name not in ['denied','alternates','pointer','oversize','prepare_crash']):
                (observer / 'pause-create').touch()
            if name == 'alternates':
                (source / '.git/objects/info/alternates').write_text(str(observer))
            if name == 'pointer':
                (source / '.git').rename(source / 'external-metadata')
                (source / '.git').write_text('gitdir: external-metadata\n')
            if name == 'oversize':
                with (source / 'huge').open('wb') as file:
                    file.truncate(64 * 1024 * 1024 + 1)
            if name == 'prepare_crash' or (vm and name == 'crash'):
                with (source / '00-large-input').open('wb') as file:
                    file.truncate(60 * 1024 * 1024)
            if name == 'extension':
                with (source / '.git/config').open('a') as file:
                    file.write('[extensions]\nrefStorage=reftable\n')
            policy = 'permit(principal, action, resource);\n'
            if name == 'denied':
                policy += 'forbid(principal, action == Action::"tool_call::git_diff", resource);\n'
            (root / 'policies/fixture.cedar').write_text(policy)
            before = set((root / '.symbiont/governed').glob('*.jsonl'))
            row = {'name': name, 'passed': False, 'policy_sha256': digest(root / 'policies/fixture.cedar')}
            report['cases'].append(row)
            process = subprocess.Popen([str(binary), 'run', 'fixture', '--input', tool, '--max-iterations', '3'],
                cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            if name == 'prepare_crash':
                until = time.monotonic() + 15
                while not list((root / 'state/staging').glob('*')) and time.monotonic() < until:
                    assert process.poll() is None
                    time.sleep(.001)
                assert list((root / 'state/staging').glob('*'))
                process.send_signal(signal.SIGSTOP)
                until = time.monotonic() + 2
                while '\nState:\tT' not in Path(f'/proc/{process.pid}/status').read_text() and time.monotonic() < until:
                    time.sleep(.001)
                assert '\nState:\tT' in Path(f'/proc/{process.pid}/status').read_text()
                assert not (backend.active() if vm or native else docker('ps', '-aq', '--filter', 'label=' + label))
                assert not list((root / 'state').glob('*.json'))
                row['interrupted_preparation'] = [str(p) for p in (root / 'state/staging').glob('*')]
                process.kill()
            actual_workers=[]
            if name in ['snapshot', 'crash', 'capacity'] or ((vm or native) and name not in ['denied','alternates','pointer','oversize','prepare_crash']):
                until = time.monotonic() + 15
                while not (observer / 'created').exists() and time.monotonic() < until:
                    assert process.poll() is None
                    time.sleep(.02)
                assert (observer / 'created').exists()
                if name == 'snapshot':
                    (source / 'tracked.txt').write_text('changed after authorization\n')
                elif name == 'crash':
                    if not vm and not native: process.kill()
                elif name == 'capacity':
                    held = list((root / 'state/staging').glob('*/reservation'))
                    assert len(held) == 1
                    charge = json.loads(held[0].read_text())
                    row['retained_charge'] = charge
                    assert charge['reserved_bytes'] >= 128 * 1024 * 1024
                    sibling_before = set((root / '.symbiont/governed').glob('*.jsonl'))
                    sibling = subprocess.run([str(binary), 'run', 'fixture', '--input', tool, '--max-iterations', '3'],
                        cwd=root, env=env, capture_output=True, text=True, timeout=20)
                    row['sibling'] = {'exit_code': sibling.returncode, 'stdout': sibling.stdout, 'stderr': sibling.stderr}
                    assert sibling.returncode == 0 and len(requests) == 3, row
                    assert 'shared staging capacity exhausted' in json.dumps(requests[-1]), row
                    assert len(list(observer.glob('worker-*.json'))) == 1
                    assert json.loads(held[0].read_text()) == charge
                    sibling_journals = set((root / '.symbiont/governed').glob('*.jsonl')) - sibling_before
                    assert len(sibling_journals) == 1
                    for sibling_path in sibling_journals:
                        sibling_principal, sibling_run = sibling_path.stem.split('.')
                        verifier(sibling_path, observer / 'public.der', observer, sibling_principal, sibling_run)
                    row['sibling']['journal_sha256'] = digest(next(iter(sibling_journals)))
                (observer / 'release').touch()
                if vm or native:
                    until=time.monotonic()+10
                    while not actual_workers and time.monotonic()<until:
                        actual_workers.extend(backend.observe())
                        assert process.poll() is None, 'runtime ended before a real isolated worker was observed'
                        if not actual_workers: time.sleep(.001)
                    assert len(actual_workers)==1
                    if name=='crash': process.kill()
            stdout, stderr = process.communicate(timeout=40)
            row.update(exit_code=process.returncode, stdout=stdout, stderr=stderr, requests=list(requests))
            process = None
            row['workers'] = [json.loads(p.read_text()) for p in observer.glob('worker-*.json')]
            if vm or native: row['actual_workers']=actual_workers
            assert not report['provider_errors'] and 'PRIVATE OBSERVER CANARY' not in json.dumps(requests), row
            journals = set((root / '.symbiont/governed').glob('*.jsonl')) - before - sibling_journals
            assert len(journals) == 1
            path = journals.pop()
            principal, run_id = path.stem.split('.')
            entries = verifier(path, observer / 'public.der', observer, principal, run_id)
            row['worker_origins'] = (backend.origins(actual_workers, entries, principal, run_id, report['public_key']) if vm or native else
                                     [verify_origin(w['origin'], entries, principal, run_id, report['public_key']) for w in row['workers']])
            grants = [call for e in entries for call in e['event'].get('PolicyEvaluated', {}).get('approved_calls', []) if call.get('contract')]
            row['journal'] = {'path': str(path), 'sha256': digest(path), 'grants': grants}
            if name == 'prepare_crash':
                assert row['exit_code'] == -9 and len(requests) == 1 and not grants and not row['workers'], row
                checked = subprocess.run([str(binary), 'audit', 'inspect', str(path), '--run-id', run_id, '--public-key', report['public_key']], cwd=root, env=env, capture_output=True, text=True, timeout=10)
                assert checked.returncode == 2, checked.stdout + checked.stderr
                row['crash_inspection'] = checked.stdout
                # Ensure a real production sweep also runs if no worker has ever
                # needed to start the independent supervisor in this process.
                helper = subprocess.run([str(binary), '__sandbox_supervisor', '--state-dir', str(root / 'state')],
                    cwd=root, env=env, capture_output=True, text=True, timeout=10)
                assert helper.returncode == 0, helper.stderr
            elif name in ['denied', 'alternates', 'pointer', 'oversize']:
                assert row['exit_code'] == 0 and len(requests) == 2 and not grants and not row['workers'], row
            else:
                assert len(grants) == 1 and len(row['workers']) == 1, row
                worker = row['workers'][0]
                descriptor = grants[0]['resolved']['command_boundary']['filesystem']
                if native:
                    assert len(actual_workers) == 1 and actual_workers[0]['id'] == worker['id']
                    backend.mounts(actual_workers, [('/tmp/symbi-git-input', False), ('/tmp/symbi-source', False)])
                    assert descriptor['worker_mounts'] == []
                elif vm:
                    assert len(actual_workers)==1 and actual_workers[0]['id']==worker['id']
                    assert actual_workers[0]['pid']==worker['launcher_pid'] and worker['prepared_before_guest']
                    assert worker['manifest']==descriptor['worker_boundary']['vm']['git_snapshot']
                    assert descriptor['worker_mounts']==[]
                else:
                    assert worker['network'] == 'none' and worker['user'] == '65534:65534'
                    assert sorted((m['Destination'], m['RW']) for m in worker['mounts'] if m['Type'] == 'bind') == [('/symbi-git-input', False), ('/symbi-source', False)]
                    assert all(m['Source'] != str(source) for m in worker['mounts'])
                    descriptor = grants[0]['resolved']['command_boundary']['filesystem']
                assert grants[0]['resolved']['execution_transport'] == 'selected_git_snapshot_boundary'
                for item in descriptor['read']:
                    key = 'input/configuration' if item['path'] == '.git/config' else 'input/metadata/' + item['path'][5:] if item['path'].startswith('.git/') else 'worktree/' + item['path']
                    assert worker['snapshots'][key] == item['sha256'], item
                if tool in ['git_log', 'git_staged_diff']:
                    assert not any(p.startswith('worktree/') for p in worker['snapshots'])
                if tool == 'git_log':
                    assert 'input/metadata/index' not in worker['snapshots']
                if name == 'crash':
                    assert row['exit_code'] == -9 and len(requests) == 1 and 'Terminated' not in entries[-1]['event'], row
                    checked = subprocess.run([str(binary), 'audit', 'inspect', str(path), '--run-id', run_id, '--public-key', report['public_key']], cwd=root, env=env, capture_output=True, text=True, timeout=10)
                    assert checked.returncode == 2, checked.stdout + checked.stderr
                    row['crash_inspection'] = checked.stdout
                else:
                    observations = [o for e in entries for o in e['event'].get('ToolBatchCompleted', {}).get('observations', [])]
                    assert len(observations) == 1
                    envelope = json.loads(observations[0]['content'])
                    row['envelope'] = envelope
                    if vm:
                        receipt=envelope['snapshot_transfer']
                        assert receipt['grant']==worker['manifest'] and receipt['read_only'] is True
                        assert 0 < receipt['filesystem_bytes'] <= 384*1024*1024
                    if name == 'extension':
                        assert row['exit_code'] == 2 and len(requests) == 1 and envelope['status'] == 'error'
                        assert 'unsupported repository extension' in envelope['results']['stderr']
                    else:
                        assert row['exit_code'] == 0 and len(requests) == (4 if name == 'capacity' else 2) and envelope['status'] == 'success', row
                        output = envelope['results']['raw_output']
                        assert envelope['output_hash'] == 'sha256:' + hashlib.sha256(output.encode()).hexdigest()
                        assert 'HELPER_EXECUTED' not in json.dumps(envelope), envelope
                        if tool == 'git_log':
                            assert 'Known fixture baseline' in output
                            assert len(output.split()[0]) == (64 if name == 'sha256' else 40)
                        elif tool == 'git_staged_diff':
                            assert '-base\n+staged\n' in output and 'unstaged' not in output, output
                        elif tool == 'git_diff':
                            assert '-staged\n+unstaged\n' in output and 'changed after authorization' not in output, output
                        else:
                            assert 'MM tracked.txt' in output and '?? untracked.txt' in output, output
            until = time.monotonic() + 20
            while ((backend.active() if vm or native else docker('ps', '-aq', '--filter', 'label=' + label)) or list((root / 'state').glob('*.json'))) and time.monotonic() < until:
                time.sleep(.1)
            assert not (backend.active() if vm or native else docker('ps', '-aq', '--filter', 'label=' + label)) and not list((root / 'state').glob('*.json'))
            assert (observer / 'canary').read_text() == 'PRIVATE OBSERVER CANARY'
            until = time.monotonic() + 10
            while list((root / 'state/staging').glob('*')) and time.monotonic() < until:
                time.sleep(.1)
            assert not list((root / 'state/staging').glob('*')), 'production cleanup retained a staging copy'
            if vm or native: backend.cleaned()
            row['automatic_staging_cleanup'] = True
            row['passed'] = True
            print(json.dumps({'case': name, 'passed': True}), flush=True)
        assert digest(binary) == report['binary_sha256']
        report['passed'] = True
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
    finally:
        (observer / 'release').touch()
        if process is not None and process.poll() is None:
            process.kill()
            process.communicate(timeout=10)
        if provider is not None:
            provider.shutdown()
            provider.server_close()
        try:
            ids = backend.active() if vm or native else docker('ps', '-aq', '--filter', 'label=' + label).split()
            report['remaining_workers'] = ids
            report['remaining_leases'] = [str(p) for p in (root / 'state').glob('*.json')]
            if ids or report['remaining_leases']:
                report['passed'] = False
            if vm or native: backend.finish(report)
            elif ids: docker('rm', '-f', *ids)
        except Exception as error:
            report.update(passed=False, cleanup_error=str(error))
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'report': str(args.report), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

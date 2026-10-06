#!/usr/bin/env python3
"""Exercise shipping source queries, denied access and a real mount-free worker.

Local scripted inference, synthetic data and independently verified signed audit
records. No external provider or repository data is used.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import runpy
import shlex
import subprocess
import tempfile
import threading
import time
import traceback
import uuid
from file_grant_backend import Backend


def digest(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def main():
    assert __debug__, 'Run without Python optimization'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--landlock', action='store_true')
    parser.add_argument('--image', default='python:3.12-slim')
    parser.add_argument('--firecracker-binary', type=Path)
    parser.add_argument('--kernel', type=Path)
    parser.add_argument('--rootfs', type=Path)
    args = parser.parse_args()
    vm = args.firecracker_binary is not None
    native = args.landlock
    assert not (vm and native)
    backend = None
    assert vm == (args.kernel is not None) == (args.rootfs is not None), 'Provide all three VM artifact paths together'
    assert not args.report.exists(), 'Preserve existing reports'
    binary = args.binary.resolve(strict=True)
    repo = Path(__file__).resolve().parent.parent
    root = Path(tempfile.mkdtemp(prefix='symbi-source-e2e-'))
    for name in ['source', 'observer', 'home', 'agents', 'tools', 'policies', 'state', 'docker-client', '.symbiont/governed']:
        (root / name).mkdir(mode=0o700, parents=True, exist_ok=True)
    label = 'symbi.source-e2e=' + uuid.uuid4().hex
    report = {'passed': False, 'fixture': str(root), 'binary_sha256': digest(binary),
              'driver_sha256': digest(__file__), 'cases': [], 'provider_errors': []}
    denv = {'PATH': '/usr/bin:/bin', 'DOCKER_HOST': 'unix:///var/run/docker.sock', 'DOCKER_CONFIG': str(root / 'docker-client')}
    provider, process = None, None
    requests = []

    def docker(*argv):
        return subprocess.check_output(['docker', *argv], env=denv, text=True, timeout=10).strip()

    try:
        image = None
        if native:
            backend = Backend(args, root, root/"source", label, docker, report)
        elif vm:
            args.firecracker_binary = args.firecracker_binary.resolve(strict=True)
            args.kernel = args.kernel.resolve(strict=True)
            args.rootfs = args.rootfs.resolve(strict=True)
            report['vm_artifacts'] = {str(p): digest(p) for p in [args.firecracker_binary, args.kernel, args.rootfs]}
        else:
            image = docker('image', 'inspect', args.image, '--format', '{{.Id}}')
            report['image'] = image
        (root / 'source/alpha.txt').write_text('first\nneedle.* useful result\nlast\n')
        (root / 'source/beta.txt').write_text('DENIED RESULT CANARY')
        (root / 'observer/canary').write_text('PRIVATE OBSERVER CANARY')
        (root / 'source/link').symlink_to(root / 'observer/canary')
        os.link(root / 'observer/canary', root / 'source/hardlink')
        (root / '.env').write_text('')
        (root / 'agents/fixture.symbi').write_text('agent fixture() {}\n')
        report['manifests'] = {}
        for name in ['read_file', 'list_files', 'grep_files']:
            path = repo / 'tools' / (name + '.clad.toml')
            (root / 'tools' / path.name).write_bytes(path.read_bytes())
            report['manifests'][name] = digest(path)
        code = ('import json,os,pathlib,time; time.sleep(2); '
                'print(json.dumps(dict(source_visible=pathlib.Path("alpha.txt").exists(),'
                'host_visible=pathlib.Path(' + repr(str(root / 'observer/canary')) + ').exists(), uid=os.getuid())))')
        (root / 'tools/scratch.clad.toml').write_text('[tool]\nname="scratch"\nversion="1"\nbinary="python3"\ndescription="Scratch isolation control"\ntimeout_seconds=10\n[command]\ntemplate=' +
            json.dumps('python3 -c ' + shlex.quote(code)) + '\n[output]\nformat="json"\n')
        if vm:
            script = 'sleep 2; test ! -e alpha.txt || exit 9; test ! -e ' + shlex.quote(str(root / 'observer/canary')) + ' || exit 9; '
            script += '''printf '{"source_visible":false,"host_visible":false,"uid":%s}\\n' "$(id -u)"'''
            (root / 'tools/scratch.clad.toml').write_text('[tool]\nname="scratch"\nversion="1"\nbinary="sh"\ndescription="VM scratch isolation control"\ntimeout_seconds=10\n[command]\ntemplate=' +
                json.dumps('sh -c ' + shlex.quote(script)) + '\n[output]\nformat="json"\n')
        seed = os.urandom(32)
        key = root / '.symbiont/governed/audit-signing.key'
        key.write_bytes(seed)
        key.chmod(0o600)
        public = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'],
            input=bytes.fromhex('302e020100300506032b657004220420') + seed, capture_output=True, check=True, timeout=5).stdout
        (root / 'observer/public.der').write_bytes(public)
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
                    assert len(requests) <= 2
                    if len(requests) == 1:
                        payload = json.loads(next(m['content'] for m in request['messages'] if m['role'] == 'user'))
                        message = {'role': 'assistant', 'content': None, 'tool_calls': [{'id': 'source', 'type': 'function',
                            'function': {'name': payload['tool'], 'arguments': json.dumps(payload['args'])}}]}
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
        cases = [('read', 'read_file', {'path': 'alpha.txt', 'offset': '6', 'limit': '23'}),
                 ('list', 'list_files', {}), ('grep', 'grep_files', {'needle': 'needle.*'}),
                 ('traversal', 'read_file', {'path': '../observer/canary'}),
                 ('symlink', 'read_file', {'path': 'link'}), ('hardlink', 'read_file', {'path': 'hardlink'}),
                 ('no_ceiling', 'list_files', {}), ('denied', 'grep_files', {'needle': 'DENIED'}),
                 ('scratch', 'scratch', {})]
        for name, tool, values in cases:
            requests.clear()
            volumes = [] if name == 'no_ceiling' else [str(root / 'source') + ':/source:ro']
            if not vm and not native:
                (root / 'symbiont.toml').write_text('[sandbox]\ntier="docker"\n[sandbox.docker]\nworking_dir="/source"\nimage=' + json.dumps(image) +
                    '\nvolumes=' + json.dumps(volumes) + '\nextra_flags=' + json.dumps(['--label=' + label]) + '\n')
            if native:
                roots = [] if name == "no_ceiling" else [str(root/"source") + ":/workspace:ro"]
                (root/"symbiont.toml").write_text('[sandbox]\ntier="landlock"\n[sandbox.roots]\nsource_roots=' + json.dumps(roots) + "\n")
            elif vm:
                roots = [] if name == 'no_ceiling' else [str(root / 'source') + ':/tmp:ro']
                (root / 'symbiont.toml').write_text('[sandbox]\ntier="firecracker"\n[sandbox.firecracker]\nworking_dir="/tmp"\n' +
                    '\n'.join(key + '=' + json.dumps(str(value)) for key, value in [('kernel_image_path', args.kernel), ('rootfs_path', args.rootfs), ('firecracker_binary', args.firecracker_binary)]) +
                    '\nsource_roots=' + json.dumps(roots) + '\n')
            policy = 'permit(principal, action, resource);\n'
            if name == 'denied':
                policy += 'forbid(principal, action == Action::"tool_call::grep_files", resource);\n'
            (root / 'policies/fixture.cedar').write_text(policy)
            before = set((root / '.symbiont/governed').glob('*.jsonl'))
            row = {'name': name, 'passed': False, 'config_sha256': digest(root / 'symbiont.toml'),
                   'policy_sha256': digest(root / 'policies/fixture.cedar'), 'workers': []}
            report['cases'].append(row)
            process = subprocess.Popen([str(binary), 'run', 'fixture', '--input', json.dumps({'tool': tool, 'args': values}), '--max-iterations', '3'],
                cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            until = time.monotonic() + 25
            while process.poll() is None and time.monotonic() < until:
                for identity in ([] if vm or native else docker('ps', '-q', '--filter', 'label=' + label).split()):
                    if identity not in [w['id'] for w in row['workers']]:
                        inspected = json.loads(docker('inspect', identity))[0]
                        row['workers'].append({'id': identity, 'mounts': inspected['Mounts']})
                if native:
                    row["workers"].extend(backend.observe())
                elif vm:
                    for record_path in (root / 'state').glob('*.json'):
                        try:
                            record = json.loads(record_path.read_text())
                            state = record['state']
                            if state['phase'] != 'vm_created' or any(w['lease'] == record['lease'] for w in row['workers']):
                                continue
                            pid = state['pid']
                            assert int(Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]) == state['start_ticks']
                            assert Path(f'/proc/{pid}/exe').resolve() == args.firecracker_binary
                            config_path = root / 'state' / ('vm-' + record['lease']) / 'vm-config.json'
                            configuration = json.loads(config_path.read_text())
                            assert len(configuration['drives']) == 1 and configuration['drives'][0]['path_on_host'] == str(args.rootfs)
                            assert configuration['drives'][0]['is_read_only'] is True and not configuration.get('network-interfaces')
                            row['workers'].append({'lease': record['lease'], 'pid': pid, 'start_ticks': state['start_ticks'], 'configuration': configuration, 'configuration_sha256': digest(config_path)})
                        except FileNotFoundError:
                            continue
                time.sleep(0.1)
            stdout, stderr = process.communicate(timeout=5)
            row.update(exit_code=process.returncode, stdout=stdout, stderr=stderr, requests=list(requests))
            process = None
            assert row['exit_code'] == 0 and len(requests) == 2 and not report['provider_errors'], row
            messages = [m['content'] for m in requests[-1]['messages'] if m['role'] == 'tool']
            assert len(messages) == 1 and 'PRIVATE OBSERVER CANARY' not in json.dumps(requests), row
            journals = set((root / '.symbiont/governed').glob('*.jsonl')) - before
            assert len(journals) == 1
            path = journals.pop()
            principal, run_id = path.stem.split('.')
            entries = verifier(path, root / 'observer/public.der', root / 'observer', principal, run_id)
            assert 'Terminated' in entries[-1]['event']
            grants = [call for e in entries for call in e['event'].get('PolicyEvaluated', {}).get('approved_calls', []) if call.get('contract')]
            row['journal'] = {'path': str(path), 'sha256': digest(path), 'grants': grants}
            if name in ['read', 'list', 'grep', 'scratch']:
                envelope = json.loads(messages[0])
                row['envelope'] = envelope
                assert envelope['status'] == 'success' and len(grants) == 1, row
                result = envelope['results']
                if name != 'scratch':
                    assert grants[0]['resolved']['execution_transport'] == 'fixed_source_broker'
                    descriptor = grants[0]['resolved']['command_boundary']['filesystem']
                    assert descriptor['host_mounts'] == [] and descriptor['result_sha256'] == envelope['output_hash']
                    canonical = json.dumps(result, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
                    assert envelope['output_hash'] == 'sha256:' + hashlib.sha256(canonical).hexdigest()
                    for item in descriptor['read']:
                        data = (root / 'source' / item['path']).read_bytes()
                        offset = item.get('offset', 0)
                        assert item['sha256'] == hashlib.sha256(data[offset:offset + item['bytes']]).hexdigest()
                    if native:
                        backend.boundary(grants[0]["resolved"]["command_boundary"])
                    elif vm:
                        boundary = grants[0]['resolved']['command_boundary']
                        assert boundary['tier'] == 'firecracker' and boundary['vm']['broker_source_roots'] == []
                    else:
                        assert grants[0]['resolved']['command_boundary']['container']['mounts'] == []
                    if name == 'read':
                        assert result['text'] == 'needle.* useful result\n' and descriptor['read'][0]['bytes'] == 23, result
                    elif name == 'list':
                        assert result['files'] == ['alpha.txt', 'beta.txt'] and descriptor['read'] == []
                    else:
                        assert result['matches'] == [{'path': 'alpha.txt', 'line': 2, 'text': 'needle.* useful result'}]
                else:
                    assert result['source_visible'] is False and result['host_visible'] is False and result['uid'] == (os.getuid() if native else 65534)
                    assert row['workers'] and (vm or all(m['Type'] != 'bind' for w in row['workers'] for m in w['mounts']))
            else:
                assert not grants, row
                if name == 'denied':
                    assert 'DENIED RESULT CANARY' not in json.dumps(requests), row
                elif name == 'no_ceiling':
                    assert 'no configured mount ceiling' in messages[0], row
            if name != 'scratch':
                assert not row['workers'], row
            if native:
                backend.cleaned()
            elif not vm:
                assert not docker('ps', '-aq', '--filter', 'label=' + label)
            else:
                assert all(not Path(f"/proc/{w['pid']}").exists() for w in row['workers'])
            assert not list((root / 'state').glob('*.json'))
            assert (root / 'observer/canary').read_text() == 'PRIVATE OBSERVER CANARY'
            row['passed'] = True
            print(json.dumps({'case': name, 'passed': True}), flush=True)
        assert len(report['cases']) == len(cases) and digest(binary) == report['binary_sha256']
        if vm:
            assert all(digest(Path(p)) == value for p, value in report['vm_artifacts'].items())
        report['passed'] = True
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.communicate(timeout=10)
        if provider is not None:
            provider.shutdown()
            provider.server_close()
        try:
            ids = backend.active() if native else [] if vm else docker('ps', '-aq', '--filter', 'label=' + label).split()
            report['remaining_workers'] = ids
            report['remaining_leases'] = [str(p) for p in (root / 'state').glob('*.json')]
            if ids or report['remaining_leases']:
                report['passed'] = False
            if native: backend.finish(report)
            elif ids:
                docker('rm', '-f', *ids)
        except Exception as error:
            report.update(passed=False, cleanup_error=str(error))
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'report': str(args.report), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

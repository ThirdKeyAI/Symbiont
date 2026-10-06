#!/usr/bin/env python3
"""Shipping CLI file-grant E2E with real workers and protected host observations.

Uses a cached Python image and a synthetic loopback provider. Input snapshots,
atomic output creation, adjacent-file isolation, parser isolation and file-size
limits are checked alongside useful work. No external provider is contacted.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import threading
import time
import traceback
import uuid
from file_grant_backend import Backend, arguments as backend_arguments


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--image', default='python:3.12-slim')
    cases = ['granted', 'undeclared', 'parser', 'oversize', 'existing', 'symlink', 'nested', 'descendant', 'substitution', 'no_read_root', 'no_write_root']
    parser.add_argument('--case', choices=cases, action='append')
    backend_arguments(parser)
    args = parser.parse_args()
    assert __debug__, 'Python assertions must be enabled'
    root = Path(tempfile.mkdtemp(prefix='symbi-file-grants-'))
    observer = root / 'observer'
    data = root / 'data'
    for name in ['observer', 'data', 'home', 'tools', 'agents', 'policies', 'state', 'docker-client', '.symbiont/governed']:
        (root / name).mkdir(mode=0o700, parents=True, exist_ok=True)
    docker_env = {'PATH': '/usr/bin:/bin', 'DOCKER_HOST': 'unix:///var/run/docker.sock', 'DOCKER_CONFIG': str(root / 'docker-client')}

    def docker(*arguments):
        return subprocess.check_output(['docker', *arguments], env=docker_env, text=True, timeout=10).strip()

    label = 'symbi.file-grants=' + uuid.uuid4().hex
    binary = args.binary.resolve(strict=True)
    report = {'passed': False, 'fixture': str(root), 'binary_sha256': digest(binary),
              'driver_sha256': digest(__file__), 'cases': [], 'provider_errors': []}
    process, provider = None, None
    requests = []
    try:
        backend = Backend(args, root, data, label, docker, report)
        (data / 'input.txt').write_text('2 3 5')
        (data / 'adjacent.txt').write_text('ungranted synthetic data')
        (data / 'linked.txt').symlink_to('adjacent.txt')
        (root / 'host-canary').write_text('protected synthetic data')
        (root / '.env').write_text('')
        (root / 'agents/fixture.symbi').write_text('agent fixture() {}\n')
        (root / 'policies/fixture.cedar').write_text('permit(principal, action, resource);\n')
        seed = os.urandom(32)
        key = root / '.symbiont/governed/audit-signing.key'
        key.write_bytes(seed)
        key.chmod(0o600)
        public = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'],
            input=bytes.fromhex('302e020100300506032b657004220420') + seed, capture_output=True, check=True, timeout=5).stdout
        (observer / 'public.der').write_bytes(public)
        report['public_key'] = public[-32:].hex()
        verifier = runpy.run_path(str(Path(__file__).with_name('test-direct-inference.py')))['verify_journal']

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
                        message = {'role': 'assistant', 'content': None, 'tool_calls': [{'id': 'files', 'type': 'function',
                            'function': {'name': 'files', 'arguments': json.dumps(payload)}}]}
                        finish = 'tool_calls'
                    else:
                        message = {'role': 'assistant', 'content': 'fixture complete'}
                        finish = 'stop'
                    body = json.dumps({'id': 'fixture', 'object': 'chat.completion', 'model': 'fixture',
                        'choices': [{'index': 0, 'message': message, 'finish_reason': finish}],
                        'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}}).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except Exception as error:
                    report['provider_errors'].append(str(error))
                    self.send_error(400)

        provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        provider.daemon_threads = True
        threading.Thread(target=provider.serve_forever, daemon=True).start()
        env = {**docker_env, 'HOME': str(root / 'home'), 'LANG': 'C.UTF-8', 'SYMBIONT_ENV': 'production',
            'SYMBIONT_SANDBOX_STATE_DIR': str(root / 'state'), 'OPENAI_API_KEY': 'synthetic-key',
            'CHAT_MODEL': 'fixture', 'OPENAI_BASE_URL': f'http://127.0.0.1:{provider.server_port}/v1',
            'SYMBIONT_TOOLCLAD_ALLOWED_PARSERS': backend.python}
        journal_outputs = {}
        for name in args.case or cases:
            requests.clear()
            journals_before = set((root / '.symbiont/governed').glob('*.jsonl'))
            output_name = ('nested/' if name == 'nested' else '') + name + '.json'
            if name == 'nested':
                (data / 'nested').mkdir()
                (data / 'nested/input.txt').write_text('2 3 5')
            configuration = (root / 'symbiont.toml').read_text()
            if (backend.vm or backend.landlock) and name in ['no_read_root', 'no_write_root']:
                key = 'source_roots' if name == 'no_read_root' else 'output_roots'
                (root / 'symbiont.toml').write_text('\n'.join(key + '=[]' if line.startswith(key + '=') else line for line in configuration.splitlines()) + '\n')
            elif name in ['no_read_root', 'no_write_root']:
                continue
            if name == 'existing':
                (data / output_name).write_text('existing protected result')
            row = {'name': name, 'passed': False}
            report['cases'].append(row)
            code = '''import json,os,pathlib,resource,sys,time
time.sleep(1)
source=pathlib.Path(sys.argv[1])
target=pathlib.Path(sys.argv[2])
result={"input_visible":source.exists(),"adjacent_visible":pathlib.Path("adjacent.txt").exists(),"uid":os.getuid(),"sum":None,"input_writable":False}
if source.exists():
    result["sum"]=sum(map(int,source.read_text().split()))
    try:
        source.write_text("changed")
        result["input_writable"]=True
    except OSError:
        pass
pathlib.Path("sibling.txt").write_text("scratch only")
result["file_limit"]=list(resource.getrlimit(resource.RLIMIT_FSIZE))
target.write_text(json.dumps(result))
'''
            if name == 'descendant':
                code += 'if os.fork()==0:\n    os.setsid()\n    time.sleep(.4)\n    target.write_text("LATE DESCENDANT WRITE")\n    time.sleep(30)\n    os._exit(0)\n'
            if name == 'substitution':
                code += 'for action in [lambda: target.unlink(), lambda: source.write_text("changed")]:\n    try: action()\n    except OSError: pass\n    else: raise RuntimeError("declared inode was replaceable")\n'
            if name == 'oversize':
                code += 'try:\n    target.write_bytes(b"x"*2048)\nexcept OSError:\n    result["oversize_blocked"]=True\n    print(json.dumps(result))\n    sys.exit(3)\nraise RuntimeError("file limit was not enforced")\n'
            elif name == 'parser':
                parser_code = 'import json,pathlib,time; time.sleep(1); print(json.dumps({"parsed":True,"input_visible":pathlib.Path("input.txt").exists(),"output_visible":pathlib.Path("parser.json").exists(),"adjacent_visible":pathlib.Path("adjacent.txt").exists()}))'
                code += 'print(' + repr(parser_code) + ')\n'
            else:
                code += 'print(json.dumps(result))\n'
            manifest = '[tool]\nname="files"\nversion="1"\nbinary="python3"\ndescription="Declared file operation"\ntimeout_seconds=10\n'
            for index, argument in enumerate(['input', 'output']):
                manifest += f'[args.{argument}]\nposition={index + 1}\nrequired=true\ntype="path"\n'
            # JSON encoding gives TOML a literal single argv script after parsing.
            import shlex
            manifest += '[command]\ntemplate=' + json.dumps('python3 -c ' + shlex.quote(code) + ' {input} {output}') + '\n'
            manifest += '[output]\nformat="json"\n'
            if name == 'parser':
                manifest += 'parser=' + json.dumps('custom:' + backend.python) + '\n'
            if name != 'undeclared':
                manifest += '[filesystem]\nread=["{input}"]\ncreate=["{output}"]\nmax_file_bytes=1024\n'
            path = root / 'tools/files.clad.toml'
            path.write_text(manifest)
            row['manifest_sha256'] = digest(path)
            payload = {'input': 'linked.txt' if name == 'symlink' else ('nested/input.txt' if name == 'nested' else 'input.txt'), 'output': output_name}
            process = subprocess.Popen([str(binary), 'run', 'fixture', '--input', json.dumps(payload), '--max-iterations', '3'],
                cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            until = time.monotonic() + 15
            observations = []
            while process.poll() is None and time.monotonic() < until:
                observations.extend(backend.observe())
                time.sleep(0.1)
            stdout, stderr = process.communicate(timeout=15)
            row.update(exit_code=process.returncode, stdout=stdout, stderr=stderr, requests=list(requests), workers=observations)
            process = None
            (root / 'symbiont.toml').write_text(configuration)
            assert len(requests) == (1 if name == 'oversize' else 2) and not report['provider_errors'], row
            messages = [m for m in requests[-1]['messages'] if m['role'] == 'tool']
            if name == 'oversize':
                paths = set((root / '.symbiont/governed').glob('*.jsonl')) - journals_before
                assert len(paths) == 1
                path = paths.pop()
                principal, run_id = path.stem.split('.')
                entries = verifier(path, observer / 'public.der', observer, principal, run_id)
                assert entries[-1]['event']['Terminated']['reason'] == 'UnconfirmedEffects'
                observations_in_audit = [o for entry in entries for o in entry['event'].get('ToolBatchCompleted', {}).get('observations', [])]
                assert len(observations_in_audit) == 1
                messages = [{'content': observations_in_audit[0]['content']}]
                assert row['exit_code'] == 2
                row['further_inference_refused'] = True
            text = '\n'.join(m['content'] for m in messages)
            if name in ['existing', 'symlink', 'no_read_root', 'no_write_root']:
                assert not observations, row
                assert ('already exists' if name == 'existing' else ('outside configured mount ceilings' if name.startswith('no_') else 'cannot open declared input')) in text + stdout + stderr, row
                if name == 'existing':
                    assert (data / output_name).read_text() == 'existing protected result'
                else:
                    assert not (data / output_name).exists()
            else:
                assert observations and len(messages) == 1, row
                envelope = json.loads(messages[0]['content'].removeprefix('[Error] '))
                row['envelope'] = envelope
                if name == 'oversize':
                    assert envelope['status'] == 'error' and envelope['results']['oversize_blocked'] and not (data / output_name).exists(), row
                elif name == 'undeclared':
                    assert envelope['status'] == 'success' and not (data / output_name).exists(), row
                    assert envelope['results']['input_visible'] is False and envelope['results']['adjacent_visible'] is False
                    backend.mounts(observations, [])
                else:
                    assert envelope['status'] == 'success', row
                    result = json.loads((data / output_name).read_text())
                    assert result == {'input_visible': True, 'adjacent_visible': False, 'uid': backend.uid, 'sum': 10, 'input_writable': False, 'file_limit': [1024, 1024]}, result
                    assert envelope['created_files'][0]['sha256'] == digest(data / output_name)
                    backend.mounts(observations, [(('/tmp/symbi-workspace' if backend.landlock else '/workspace') + '/' + payload['input'], False), (('/tmp/symbi-workspace' if backend.landlock else '/workspace') + '/' + output_name, True)] + ([('/tmp/symbi-parser-input', False)] if backend.landlock and name == 'parser' else []))
                    if name == 'parser':
                        assert envelope['results'] == {'parsed': True, 'input_visible': False, 'output_visible': False, 'adjacent_visible': False, 'exit_code': 0}
            assert (data / 'input.txt').read_text() == '2 3 5'
            assert (data / 'adjacent.txt').read_text() == 'ungranted synthetic data'
            assert not (data / 'sibling.txt').exists()
            paths = set((root / '.symbiont/governed').glob('*.jsonl')) - journals_before
            assert len(paths) == 1
            journal = paths.pop(); principal, run_id = journal.stem.split('.')
            entries = verifier(journal, observer / 'public.der', observer, principal, run_id)
            row['worker_origins'] = backend.origins(observations, entries, principal, run_id, report['public_key'])
            journal_outputs[str(journal)] = [output_name] if name in ['granted', 'parser', 'nested', 'descendant', 'substitution'] else []
            backend.cleaned()
            row['passed'] = True
            print(json.dumps({'case': name, 'passed': True}), flush=True)
        report['journals'] = []
        for path in (root / '.symbiont/governed').glob('*.jsonl'):
            principal, run_id = path.stem.split('.')
            entries = verifier(path, observer / 'public.der', observer, principal, run_id)
            assert 'Terminated' in entries[-1]['event']
            backend.publication(entries, data, journal_outputs[str(path)])
            grants = [call for entry in entries for call in entry['event'].get('PolicyEvaluated', {}).get('approved_calls', []) if call.get('contract')]
            for grant in grants:
                filesystem = grant['resolved']['command_boundary']['filesystem']
                assert all('sha256' in read and 'bytes' in read for read in filesystem['read'])
                backend.boundary(grant['resolved']['command_boundary'])
            report['journals'].append({'path': str(path), 'sha256': digest(path), 'file_grants': [g['resolved']['command_boundary']['filesystem'] for g in grants]})
        assert len(report['journals']) == len(report['cases'])
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
            backend.finish(report)
        except Exception as error:
            report.update(passed=False, cleanup_error=str(error))
        if digest(binary) != report['binary_sha256']:
            report.update(passed=False, artifact_error='binary changed during test')
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'report': str(args.report)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

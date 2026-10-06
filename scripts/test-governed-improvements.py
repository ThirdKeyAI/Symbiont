#!/usr/bin/env python3
"""Installed CLI lifecycle with synthetic local inference and signed evidence.

No external services or credentials. Exercises optional behavior, real ORGA
trial execution, offline evaluation, approval, activation, rollback and refusal.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    requests = []
    checks = []

    class Provider(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            assert self.path == '/v1/chat/completions'
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(body)
            system = '\n'.join(m.get('content', '') for m in body['messages'] if m['role'] == 'system')
            user = next(m['content'] for m in reversed(body['messages']) if m['role'] == 'user')
            content = 'BASELINE'
            if 'EVIDENCE_WORKFLOW_V1' in system or 'EVIDENCE_WORKFLOW_V2' in system:
                content = 'REQUEST_DOCUMENT' if user == 'missing_document' else 'ESCALATE'
            result = dict(id='synthetic', object='chat.completion', model=body['model'],
                          choices=[dict(index=0, message=dict(role='assistant', content=content), finish_reason='stop')],
                          usage=dict(prompt_tokens=10, completion_tokens=3, total_tokens=13))
            if user == 'attempt_transfer' and not any(m['role'] == 'tool' for m in body['messages']):
                result['choices'][0] = dict(index=0, finish_reason='tool_calls', message=dict(
                    role='assistant', content=None, tool_calls=[dict(id='forbidden-transfer', type='function',
                        function=dict(name='execute_payment', arguments='{}'))]))
            payload = json.dumps(result).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    thread = threading.Thread(target=provider.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='symbi-improvements-') as temporary:
            root = Path(temporary)
            (root / 'agents').mkdir()
            (root / 'agents/claims.symbi').write_text('agent claims {}\n')
            (root / '.env').write_text('')
            env = {k: v for k, v in os.environ.items() if not (
                k.endswith('_API_KEY') or k.endswith('_API_KEY_REF') or k.endswith('_BASE_URL')
                or k.startswith(('SYMBI', 'SYMBIONT', 'ANTHROPIC', 'OPENROUTER', 'OPENAI', 'CHAT_')))}
            env.update(OPENAI_API_KEY='synthetic-key', CHAT_MODEL='fixture',
                       OPENAI_BASE_URL=f'http://127.0.0.1:{provider.server_port}/v1')

            def cli(*words, code=0, extra_env=None):
                result = subprocess.run([str(binary), *words], cwd=root,
                                        env=dict(env, **(extra_env or {})), capture_output=True, text=True, timeout=30)
                assert result.returncode == code, (words, result.returncode, result.stdout, result.stderr)
                return result

            def admin(operation, *words, code=0):
                return json.loads(cli('improvement', operation, '--workflow', 'claims', *words, code=code).stdout)

            def refused(*words, extra_env=None):
                before = len(requests)
                result = cli(*words, code=1, extra_env=extra_env)
                assert len(requests) == before, result.stderr
                return result

            def save(name, value):
                path = root / name
                path.write_text(json.dumps(value))
                return str(path)

            def run(candidate=None, selected=True, input_value='missing_document', **kwargs):
                words = ['run', 'claims', '--input', input_value]
                if selected:
                    words += ['--improvement', 'claims']
                if candidate:
                    words += ['--improvement-trial', candidate]
                return cli(*words, **kwargs)

            assert 'BASELINE' in run(selected=False).stdout
            assert not (root / '.symbiont/improvements').exists()
            checks.append('default run has no improvement state or behavioral change')
            refused('run', 'claims', '--improvement', 'claims')
            suite = dict(schema_version=1, minimum_passed=2, maximum_tokens_per_case=1000,
                         maximum_denied_calls=0, cases=[
                             dict(id='missing', input='missing_document', expected_output='REQUEST_DOCUMENT'),
                             dict(id='exception', input='exception_case', expected_output='ESCALATE')])
            initialization = admin('init', '--agent', 'agents/claims.symbi', '--suite', save('suite.json', suite))
            assert initialization['workflow']['enabled']
            assert 'BASELINE' in run(selected=False).stdout
            refused('run', 'claims', '--improvement', 'claims')
            checks.append('initialization is explicit and does not activate or affect ordinary runs')

            def candidate(marker):
                return admin('propose', '--file', save('proposal.json',
                    dict(instructions=marker, rationale='Prepare evidence and escalate exceptions.')))['candidate']

            def evaluate(candidate_id):
                references = []
                for case in suite['cases']:
                    result = run(candidate_id, input_value=case['input'])
                    fields = dict(re.findall(r'^Audit (run|journal|public key): (.+)$', result.stderr, re.MULTILINE))
                    audit = dict(run_id=fields['run'], path=fields['journal'], public_key=fields['public key'])
                    cli('audit', 'inspect', audit['path'], '--run-id', audit['run_id'], '--public-key', audit['public_key'])
                    records = [json.loads(line)['payload']['entry'] for line in Path(audit['path']).read_text().splitlines()]
                    binding = records[0]['event']['Started']['execution_context']['improvement']
                    assert binding['candidate'] == candidate_id and binding['mode'] == 'trial'
                    references.append(dict(case_id=case['id'], audit=audit))
                report = admin('evaluate', '--candidate', candidate_id, '--trials', save('trials.json', references),
                               code=2 if candidate_id == bad else 0)
                return report, references

            bad = candidate('INEFFECTIVE_WORKFLOW')
            failed, _ = evaluate(bad)
            assert not failed['report']['accepted']
            refused('improvement', 'approve', '--workflow', 'claims', '--candidate', bad,
                    '--evaluation', failed['evaluation'], '--rationale', 'Must refuse failed evidence')
            checks.append('failed real trials cannot be approved')

            first = candidate('EVIDENCE_WORKFLOW_V1')
            report, references = evaluate(first)
            assert report['report']['accepted']
            swapped = list(references)
            swapped[0] = dict(swapped[0], case_id='exception')
            refused('improvement', 'evaluate', '--workflow', 'claims', '--candidate', first,
                    '--trials', save('bad-trials.json', swapped))
            forged = json.loads(json.dumps(references))
            forged[0]['audit']['public_key'] = '00' * 32
            refused('improvement', 'evaluate', '--workflow', 'claims', '--candidate', first,
                    '--trials', save('forged-trials.json', forged))
            checks.append('evaluation refuses duplicate cases and untrusted evidence keys')
            approval = admin('approve', '--candidate', first, '--evaluation', report['evaluation'],
                             '--rationale', 'Reviewed both independently specified acceptance cases')['approval']
            exported = admin('export', '--kind', 'candidate', '--id', first)
            exported_path = save('candidate.signed.json', exported)
            verified = json.loads(cli('improvement', 'verify', '--kind', 'candidate', '--file', exported_path,
                                      '--public-key', initialization['public_key']).stdout)
            assert verified['verified']
            refused('improvement', 'verify', '--kind', 'approval', '--file', exported_path,
                    '--public-key', initialization['public_key'])
            checks.append('exported evidence verifies independently and signatures bind document kind')
            refused('improvement', 'promote', '--workflow', 'claims', '--candidate', bad,
                    '--approval', approval, '--expected-active', 'none')
            admin('promote', '--candidate', first, '--approval', approval, '--expected-active', 'none')
            assert 'REQUEST_DOCUMENT' in run().stdout
            approved_journal = Path(re.search(r'^Audit journal: (.+)$', run().stderr, re.MULTILINE)[1])
            binding = json.loads(approved_journal.read_text().splitlines()[0])['payload']['entry']['event']['Started']['execution_context']['improvement']
            assert binding['mode'] == 'approved' and binding['approval'] == approval
            checks.append('exact approval activates a version and approved runs retain signed attribution')
            denied = run(input_value='attempt_transfer')
            path = Path(re.search(r'^Audit journal: (.+)$', denied.stderr, re.MULTILINE)[1])
            events = [json.loads(line)['payload']['entry']['event'] for line in path.read_text().splitlines()]
            assert not any('ToolDispatchStarted' in event for event in events)
            assert any('execute_payment' in json.dumps(event) for event in events)
            checks.append('selected instructions do not authorize an unregistered financial action')
            invocation_id = str(uuid.uuid4())
            cli('run', 'claims', '--improvement', 'claims', '--input', 'missing_document', '--invocation-id', invocation_id)
            before = len(requests)
            cli('run', 'claims', '--improvement', 'claims', '--input', 'missing_document', '--invocation-id', invocation_id)
            assert len(requests) == before

            second = candidate('EVIDENCE_WORKFLOW_V2')
            second_report, _ = evaluate(second)
            second_approval = admin('approve', '--candidate', second, '--evaluation', second_report['evaluation'],
                                    '--rationale', 'Reviewed replacement')['approval']
            refused('improvement', 'promote', '--workflow', 'claims', '--candidate', second,
                    '--approval', second_approval, '--expected-active', 'none')
            admin('promote', '--candidate', second, '--approval', second_approval, '--expected-active', first)
            refused('run', 'claims', '--improvement', 'claims', '--input', 'missing_document', '--invocation-id', invocation_id)
            checks.append('idempotent retries preserve the version and refuse reuse after promotion')
            admin('rollback', '--candidate', first, '--approval', approval, '--expected-active', second)
            assert admin('inspect')['workflow']['active'] == first
            checks.append('activation uses compare-and-swap and rollback restores an approved prior version')

            admin('disable')
            refused('run', 'claims', '--improvement', 'claims')
            assert 'BASELINE' in run(selected=False).stdout
            admin('enable')
            for changed in [{'CHAT_MODEL': 'changed-model'},
                            {'OPENAI_BASE_URL': 'http://127.0.0.1:1/v1'}]:
                rejection = refused('run', 'claims', '--improvement', 'claims', extra_env=changed)
                assert 'differs from the approved evaluation' in rejection.stderr
            refused('run', 'claims', '--improvement', 'claims', extra_env={'SYMBI_INSECURE_ALLOW_ALL': '1'})
            checks.append('disabled, changed-model, changed-endpoint and permissive-bypass selections refuse before inference')
            (root / 'policies').mkdir()
            (root / 'policies/new.cedar').write_text('permit(principal,action,resource);\n')
            rejection = refused('run', 'claims', '--improvement', 'claims')
            assert 'deployment inputs changed' in rejection.stderr
            (root / 'policies/new.cedar').unlink()
            (root / 'policies').rmdir()
            for name, content in [('toolclad.toml', '[types]\n'), ('scope/scope.toml', '[scope]\n')]:
                path = root / name
                path.parent.mkdir(exist_ok=True)
                path.write_text(content)
                rejection = refused('run', 'claims', '--improvement', 'claims')
                assert 'deployment inputs changed' in rejection.stderr
                path.unlink()
            (root / 'scope').rmdir()
            (root / 'agents/claims.symbi').write_text('agent claims {\n}\n')
            rejection = refused('run', 'claims', '--improvement', 'claims')
            assert 'agent source changed or does not match' in rejection.stderr
            (root / 'agents/claims.symbi').write_text('agent claims {}\n')
            checks.append('new deployment files and changed agent definitions invalidate selection')
            state = root / '.symbiont/improvements/claims/state.json'
            envelope = json.loads(state.read_text())
            envelope['payload']['enabled'] = False
            state.write_text(json.dumps(envelope))
            refused('run', 'claims', '--improvement', 'claims')
            checks.append('modified signed state fails closed')
    finally:
        provider.shutdown()
        provider.server_close()
        thread.join(timeout=5)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        hasher = hashlib.sha256()
        with binary.open('rb') as executable:
            while chunk := executable.read(1024 * 1024):
                hasher.update(chunk)
        binary_sha256 = hasher.hexdigest()
        args.report.write_text(json.dumps(dict(binary_sha256=binary_sha256, checks=checks,
                                              provider_requests=len(requests)), indent=2) + '\n')
    print(json.dumps(dict(passed=len(checks), report=str(args.report))))


if __name__ == '__main__':
    main()

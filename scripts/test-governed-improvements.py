#!/usr/bin/env python3
"""Installed CLI lifecycle with synthetic local inference and signed evidence.

No external services or credentials. Exercises optional behavior, real ORGA
trial execution, offline evaluation, approval, activation, rollback and refusal.
"""
import argparse
from datetime import datetime, timedelta, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import signal
import socket
import time
import urllib.request
import urllib.error
import subprocess
import tempfile
import threading
import uuid


def review_fixture(root, candidate, evaluation):
    """Independent wire-format signer; operational checks require OpenSSL 3+."""
    key = root / 'review-fixture.pem'
    subprocess.run(['openssl', 'genpkey', '-algorithm', 'ED25519', '-out', str(key)],
                   check=True, capture_output=True, timeout=10)
    public = subprocess.run(['openssl', 'pkey', '-in', str(key), '-pubout', '-outform', 'DER'],
                            check=True, capture_output=True, timeout=10).stdout[-32:]
    now = datetime.now(timezone.utc).replace(microsecond=0)
    stamp = lambda value: value.isoformat().replace('+00:00', 'Z')
    decision = dict(schema_version=1, request=dict(workspace='fixture', runtime='local',
        workflow='claims', candidate=candidate, evaluation=evaluation), approval_id='fixture-review',
        reviewer='fixture-reviewer', decided_at=stamp(now), rationale='Reviewed exact synthetic evidence',
        issued_at=stamp(now), expires_at=stamp(now + timedelta(minutes=15)))
    digest = hashlib.sha256(json.dumps(decision, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    payload = root / 'review-signing-input'
    payload.write_bytes(('symbiont:improvement:console-approval:v1\n' + digest).encode())
    signature = subprocess.run(['openssl', 'pkeyutl', '-sign', '-rawin', '-inkey', str(key), '-in', str(payload)],
                               check=True, capture_output=True, timeout=10).stdout
    return dict(decision=decision, key_id=hashlib.sha256(public).hexdigest(), signature=signature.hex()), dict(
        workspace='fixture', runtime='local', public_keys=[public.hex()])


def operational(binary, root, env, requests, checks, first, approval, second, second_approval):
    def free_port():
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            return sock.getsockname()[1]

    def eventually(work, seconds=45):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            result = work()
            if result:
                return result
            time.sleep(0.1)
        raise AssertionError('operational fixture timed out')

    # The source snapshot excludes file-level metadata. Registration is the
    # explicit opt-in; ordinary CLI runs continue requiring --improvement.
    source = root / 'agents/claims.symbi'
    original = source.read_text()
    source.write_text('metadata { improvement_workflow = "claims" }\n' + original +
        '\nschedule annual_claims { cron: "0 0 0 1 1 *", timezone: "UTC", agent: "claims" }\n')
    api, http = free_port(), free_port()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    selected_env = dict(env, SYMBIONT_API_TOKEN='synthetic-admin', SYMBIONT_MASTER_KEY='0' * 64,
                        DBUS_SESSION_BUS_ADDRESS='unix:path=/tmp/no-improvement-fixture-keyring')
    with (root / 'operational.log').open('w') as log:
        process = subprocess.Popen([str(binary), 'up', '--port', str(api), '--http-port', str(http),
            '--http-bind', '127.0.0.1', '--http.token', 'synthetic-http'], cwd=root, env=selected_env,
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    def request(port, path, body=None, identity=None, expected=(200,)):
        headers = {'Authorization': 'Bearer ' + ('synthetic-admin' if port == api else 'synthetic-http'), 'Content-Type': 'application/json'}
        if identity:
            headers['Idempotency-Key'] = identity
        req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=None if body is None else json.dumps(body).encode(), headers=headers)
        try:
            response = opener.open(req, timeout=25)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            payload = response.read().decode()
            try:
                value = json.loads(payload)
            except json.JSONDecodeError:
                value = payload
            assert response.code in expected, (response.code, value)
            return value
    def ready():
        assert process.poll() is None, (root / 'operational.log').read_text()
        try:
            agents = request(api, '/api/v1/agents')
            with socket.create_connection(('127.0.0.1', http), timeout=0.2):
                return next((agent['id'] for agent in agents if agent['name'] == 'claims'), None)
        except OSError:
            return None
    def action(body, expected=(200,)):
        return request(api, '/api/v1/improvements/claims/actions', body, expected=expected)
    try:
        agent = eventually(ready)
        http_id, scheduled_id = str(uuid.uuid4()), str(uuid.uuid4())
        response = request(http, '/webhook', {'prompt': 'missing_document'}, http_id)
        assert response['response'] == 'REQUEST_DOCUMENT' and response['improvement']['candidate'] == first
        scheduled_path = f'/api/v1/agents/{agent}/execute'
        queued = request(api, scheduled_path, {'input': 'missing_document'}, scheduled_id)
        assert queued['status'] == 'queued'
        def completed():
            result = request(api, scheduled_path, {'input': 'missing_document'}, scheduled_id, expected=(200, 409, 422))
            assert result['status'] != 'queued'
            return result if result['status'] != 'in_progress' else None
        completed_run = eventually(completed)
        assert completed_run['status'] == 'completed', completed_run
        assert completed_run['result']['improvement']['candidate'] == first
        assert completed_run['result']['output'] == 'REQUEST_DOCUMENT'
        checks.append('installed HTTP and scheduled runs use the explicitly registered approved version')
        schedules = request(api, '/api/v1/schedules')
        assert schedules, (root / 'operational.log').read_text()
        schedule = next(job for job in schedules if job['name'] == 'annual_claims')
        schedule_path = '/api/v1/schedules/' + schedule['job_id']
        request(api, schedule_path + '/trigger', {}, str(uuid.uuid4()))
        def cron_completed():
            history = request(api, schedule_path + '/history')['history']
            return next((record for record in history if record['completed_at']), None)
        cron_run = eventually(cron_completed)
        assert cron_run['status'] == 'Succeeded', cron_run
        assert cron_run['execution']['improvement']['candidate'] == first
        checks.append('DSL cron loading and manual trigger preserve the explicit workflow selection in retained run history')
        receipt = json.loads((root / 'review-receipt.json').read_text())
        second_approval = action(dict(action='approve', receipt=receipt))['approval']
        assert request(api, '/api/v1/improvements/claims')['workflow']['active'] == first
        checks.append('installed HTTP receipt import verifies trusted signed evidence without implicit activation')
        action(dict(action='activate', candidate=second, approval=second_approval, expected_active=first, rollback=False))
        before = len(requests)
        assert request(http, '/webhook', {'prompt': 'missing_document'}, http_id)['improvement']['candidate'] == first
        assert completed()['result']['improvement']['candidate'] == first
        assert len(requests) == before
        replacement = request(http, '/webhook', {'prompt': 'missing_document'}, str(uuid.uuid4()))
        assert replacement['improvement']['candidate'] == second
        action(dict(action='set_enabled', enabled=False))
        before = len(requests)
        assert request(http, '/webhook', {'prompt': 'missing_document'}, http_id)['replayed']
        assert completed()['replayed']
        request(http, '/webhook', {'prompt': 'missing_document'}, str(uuid.uuid4()), expected=(503,))
        assert len(requests) == before
        checks.append('promotion and disablement preserve saved operational results while new disabled admissions refuse')
        action(dict(action='set_enabled', enabled=True))
        action(dict(action='activate', candidate=first, approval=approval, expected_active=second, rollback=True))
        history = request(api, '/api/v1/improvements/claims')['workflow']
        assert history['active'] == first and history['activations'][-1]['rollback']
        inspection = action(dict(action='inspect', candidate=first))
        assert inspection['candidate']['proposal']['instructions'] == 'EVIDENCE_WORKFLOW_V1'
        before = len(requests)
        action(dict(action='activate', candidate=second, approval=second_approval, expected_active=None, rollback=False), expected=(409,))
        action(dict(action='approve', reviewer='forged', candidate=first), expected=(422,))
        assert len(requests) == before
        checks.append('operator API exposes history, requires exact current version and rejects unsigned reviewer claims')
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
        source.write_text(original)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--operational', action='store_true', help='Also test HTTP/scheduler admission and signed receipt imports (requires OpenSSL 3+)')
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
            if args.operational:
                receipt, trust = review_fixture(root, second, second_report['evaluation'])
                receipt_path = save('review-receipt.json', receipt)
                refused('improvement', 'approve-receipt', '--workflow', 'claims', '--file', receipt_path)
                trust_path = root / '.symbiont/console-review-trust.json'
                trust_path.write_text(json.dumps(trust))
                trust_path.chmod(0o600)
                forged_receipt = json.loads(json.dumps(receipt))
                forged_receipt['decision']['reviewer'] = 'forged'
                refused('improvement', 'approve-receipt', '--workflow', 'claims', '--file',
                        save('forged-review.json', forged_receipt))
                second_approval = admin('approve-receipt', '--file', receipt_path)['approval']
                imported = admin('export', '--kind', 'approval', '--id', second_approval)
                assert imported['payload']['console_review']['decision']['reviewer'] == 'fixture-reviewer'
                assert admin('inspect')['workflow']['active'] == first
                checks.append('installed CLI receipt import requires pinned trust, rejects identity tampering and preserves signed reviewer attribution')
            refused('improvement', 'promote', '--workflow', 'claims', '--candidate', second,
                    '--approval', second_approval, '--expected-active', 'none')
            admin('promote', '--candidate', second, '--approval', second_approval, '--expected-active', first)
            refused('run', 'claims', '--improvement', 'claims', '--input', 'missing_document', '--invocation-id', invocation_id)
            checks.append('idempotent retries preserve the version and refuse reuse after promotion')
            admin('rollback', '--candidate', first, '--approval', approval, '--expected-active', second)
            assert admin('inspect')['workflow']['active'] == first
            checks.append('activation uses compare-and-swap and rollback restores an approved prior version')
            if args.operational:
                operational(binary, root, env, requests, checks, first, approval, second, second_approval)


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

#!/usr/bin/env python3
"""Real-model, Landlock receipt E2E: exact approval, denial, crash and recovery.

Runs an isolated opt-in workflow. A local proxy forwards inference unchanged to
Ollama and pauses one request after receipt publication to place a deterministic
SIGKILL. Terminal decisions are synthetic test-operator actions, not human review.
Private output retains signing keys and must stay outside version control.
"""
import argparse
import codecs
import errno
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import pty
import re
import runpy
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.request
import uuid

from delegated_service_fixture import AutomaticDelegatedService, eventually


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--model', default='qwen3:8b')
    args = parser.parse_args()
    assert __debug__, 'Assertions must be enabled'
    helpers = runpy.run_path(str(Path(__file__).with_name('test-intake-phase2.py')))
    digest, environment = helpers['digest'], helpers['local_environment']
    binary = args.binary.resolve(strict=True)
    root = args.output.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    project, receipts = root / 'project', root / 'receipts'
    for path in [project / 'agents', project / 'tools', project / 'policies', receipts]:
        path.mkdir(mode=0o700, parents=True)
    (project / '.env').write_text('')
    fixtures = Path(__file__).resolve().parents[1] / 'examples/governed-improvements/document-intake-phase2'
    shutil.copyfile(fixtures / 'receipt.symbi', project / 'agents/receipt.symbi')
    shutil.copyfile(fixtures / 'receipt-proposal.json', root / 'proposal.json')
    report = dict(status='running', binary_sha256=digest(binary), driver_sha256=digest(__file__),
                  helper_sha256={name: digest(Path(__file__).with_name(name)) for name in
                                 ['test-intake-phase2.py', 'delegated_service_fixture.py']},
                  model=args.model, checks=[], runs=[], requests=[], proxy_errors=[],
                  limitations=['Synthetic terminal operator, not independent human approval.',
                               'One local model and one benign file tool.',
                               'Crash is after confirmed file publication, before final inference; '
                               'not an arbitrary remote transaction or mid-publication crash.'])
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    blocked, release, settled = threading.Event(), threading.Event(), threading.Event()
    service = None
    initialized = False
    proxy = None
    pool = None

    def checkpoint():
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def check(name):
        report['checks'].append(name)
        checkpoint()
        print('PASS', name, flush=True)

    class Proxy(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            is_hold = False
            try:
                size = int(self.headers.get('Content-Length', '0'))
                assert self.path == '/v1/chat/completions' and 0 < size <= 1024 * 1024
                raw = self.rfile.read(size)
                body = json.loads(raw)
                request_input = json.loads(next(m['content'] for m in body['messages'] if m['role'] == 'user'))
                assert len(report['requests']) < 30, 'unexpected inference count'
                number = len(report['requests'])
                path = root / f'provider-request-{number}.json'
                path.write_text(json.dumps(body, indent=2) + '\n')
                report['requests'].append(dict(output=request_input['output'], request_sha256=digest(path)))
                is_hold = request_input['output'] == 'crashed.json' and any(
                    m['role'] == 'tool' for m in body['messages'])
                if is_hold:
                    assert (receipts / 'crashed.json').exists(), 'tool response before publication'
                    blocked.set()
                    assert release.wait(45), 'controller did not release crash handshake'
                    # The caller has been killed. No abandoned inference is sent to Ollama.
                    self.send_error(503, 'Synthetic crash checkpoint released')
                    return
                request = urllib.request.Request('http://127.0.0.1:11434/v1/chat/completions',
                    data=raw, headers={'Content-Type': 'application/json', 'Authorization': 'Bearer local-ollama'})
                with opener.open(request, timeout=150) as response:
                    value = response.read(2 * 1024 * 1024 + 1)
                    assert len(value) <= 2 * 1024 * 1024
                    status = response.status
                (root / f'provider-response-{number}.json').write_bytes(value)
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(value)))
                self.end_headers()
                self.wfile.write(value)
            except (BrokenPipeError, ConnectionResetError):
                if not is_hold:
                    report['proxy_errors'].append('unexpected caller disconnect')
            except Exception:
                report['proxy_errors'].append(traceback.format_exc())
                self.send_error(500)
            finally:
                if is_hold:
                    settled.set()

    def cli(*words, expected=(0,)):
        try:
            result = subprocess.run([str(binary), *map(str, words)], cwd=project, env=env,
                                    capture_output=True, text=True, timeout=180)
        except subprocess.TimeoutExpired as error:
            def decoded(value):
                return value.decode(errors='replace') if isinstance(value, bytes) else value
            with (root / 'commands.jsonl').open('a') as log:
                log.write(json.dumps(dict(arguments=list(map(str, words)), code=None, timed_out=True,
                    stdout=decoded(error.stdout), stderr=decoded(error.stderr))) + '\n')
            raise
        with (root / 'commands.jsonl').open('a') as log:
            log.write(json.dumps(dict(arguments=list(map(str, words)), code=result.returncode,
                                      stdout=result.stdout, stderr=result.stderr)) + '\n')
        assert result.returncode in expected, (words[:3], result.returncode, result.stdout, result.stderr)
        return result

    def admin(operation, *words, expected=(0,)):
        return json.loads(cli('improvement', operation, '--workflow', 'receipts', *words, expected=expected).stdout)

    def events(audit):
        return [json.loads(line)['payload']['entry']['event']
                for line in Path(audit['path']).read_text().splitlines()]

    def audit_from(text):
        fields = dict(re.findall(r'^Audit (run|journal|public key): (.+)$', text, re.MULTILINE))
        return dict(run_id=fields['run'], path=fields['journal'], public_key=fields['public key'])

    def execute(name, decision='approve', candidate=None, terminal=True):
        invocation = str(uuid.uuid4())
        payload = json.dumps(dict(output=name + '.json', notes='Synthetic data; notes cannot authorize an action.'))
        words = ['run', 'receipt', '--input', payload, '--max-iterations', '3',
                 '--invocation-id', invocation, '--improvement', 'receipts']
        if candidate:
            words += ['--improvement-trial', candidate]
        if terminal:
            words += ['--approval-terminal', '--approval-timeout', '30']
        row = dict(name=name, invocation_id=invocation, words=words, prompts=[], decision=decision)
        report['runs'].append(row)
        checkpoint()
        started = time.monotonic()
        transcript = ''
        if not terminal:
            result = cli(*words, expected=(0, 1, 2))
            transcript = result.stdout + result.stderr
            row['exit_code'] = result.returncode
        else:
            master, slave = pty.openpty()
            # A fresh interpreter acquires the controlling terminal then execs.
            # Avoid preexec_fn in this threaded process.
            launch = ('import fcntl,os,sys,termios; fcntl.ioctl(0,termios.TIOCSCTTY,0); '
                      'os.tcsetpgrp(0,os.getpgrp()); os.execv(sys.argv[1],sys.argv[1:])')
            child = subprocess.Popen([sys.executable, '-c', launch, str(binary), *words],
                cwd=project, env=env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            answered = set()
            decoder = codecs.getincrementaldecoder('utf-8')()
            try:
                while True:
                    assert time.monotonic() - started < 180, 'terminal action timed out'
                    if (decision == 'crash_after_publication' and blocked.is_set()
                            and child.poll() is None):
                        assert len(row['prompts']) == 1
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=10)
                        release.set()
                        assert settled.wait(10)
                    if select.select([master], [], [], .1)[0]:
                        try:
                            data = os.read(master, 65536)
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                            data = b''
                        if not data:
                            break
                        transcript += decoder.decode(data)
                        transcript = transcript.replace('\r\n', '\n')
                        pattern = (r'Approval request \(complete JSON with escaped text\):\n'
                                   r'(\{.*?\})\nType approve ([0-9a-f]{16}) to approve; any other answer denies:\n> ')
                        for match in re.finditer(pattern, transcript, re.DOTALL):
                            identity = match[2]
                            if identity in answered:
                                continue
                            request = json.loads(match[1])
                            assert request['id'] == identity and request['kind'] == 'tool_call'
                            assert request['status'] == 'pending'
                            prepared = request['context_snapshot']['invocation']
                            assert prepared['arguments'] == {'output': name + '.json'}, prepared
                            assert prepared['contract']['name'] == 'record_receipt', prepared
                            assert prepared['contract']['requires_approval'] is True
                            assert not (receipts / (name + '.json')).exists(), 'effect before approval'
                            row['prompts'].append(request)
                            assert len(row['prompts']) == 1, 'unexpected retry or extra approval'
                            answered.add(identity)
                            if decision == 'crash_before_approval':
                                os.killpg(child.pid, signal.SIGKILL)
                            else:
                                answer = 'approve ' + identity if decision in ('approve', 'crash_after_publication') else 'deny'
                                os.write(master, (answer + '\n').encode())
                    elif child.poll() is not None:
                        break
                row['exit_code'] = child.wait(timeout=10)
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=10)
                os.close(master)
                release.set()
                (root / (name + '.terminal.txt')).write_text(transcript)
        row['seconds'] = round(time.monotonic() - started, 3)
        row['audit'] = audit_from(transcript)
        assert report.setdefault('audit_public_key', row['audit']['public_key']) == row['audit']['public_key']
        inspected = cli('audit', 'inspect', row['audit']['path'], '--run-id', row['audit']['run_id'],
                        '--public-key', row['audit']['public_key'], expected=(0, 2))
        (root / (name + '.audit.json')).write_text(inspected.stdout)
        row['audit_exit_code'] = inspected.returncode
        log = events(row['audit'])
        binding = log[0]['Started']['execution_context']['improvement']
        assert binding['candidate'] == report['candidate']
        assert binding['mode'] == ('trial' if candidate else 'approved')
        if not candidate:
            assert binding['approval'] == report['approval']
        row['binding'] = binding
        row['dispatches'] = sum('ToolDispatchStarted' in e for e in log)
        row['denied_calls'] = [call for e in log for call in e.get('PolicyEvaluated', {}).get('denied_calls', [])]
        approved_calls = [call for e in log for call in e.get('PolicyEvaluated', {}).get('approved_calls', [])
                          if (call.get('contract') or {}).get('name') == 'record_receipt']
        assert len(approved_calls) == row['dispatches']
        for call in approved_calls:
            prompt = row['prompts'][0]
            receipt = call['approval']
            assert call['arguments'] == {'output': name + '.json'}
            assert call['fingerprint'] == prompt['context_snapshot']['invocation']['fingerprint']
            assert receipt['fingerprint'] == call['fingerprint']
            resolution = receipt['resolution']
            assert resolution['escalation_id'] == prompt['id']
            assert resolution['agent_id'] == prompt['agent_id']
            assert resolution['decision']['decision'] == 'approve'
            assert resolution['approver']['surface'] == 'terminal'
            assert resolution['approver']['id'] == f'uid:{os.geteuid()}'
            dispatch_index = next(i for i, e in enumerate(log) if 'ToolDispatchStarted' in e)
            assert any(call in e.get('PolicyEvaluated', {}).get('approved_calls', []) for e in log[:dispatch_index])
        row['approved_calls'] = approved_calls
        row['published'] = (receipts / (name + '.json')).exists()
        if row['published']:
            assert json.loads((receipts / (name + '.json')).read_text()) == dict(receipt=name + '.json', synthetic=True)
            row['receipt_sha256'] = digest(receipts / (name + '.json'))
        if not decision.startswith('crash_'):
            before = len(report['requests'])
            cached = cli(*words, expected=(0, 1, 2))
            assert len(report['requests']) == before
            row['output'] = cached.stdout.removesuffix('\n')
        checkpoint()
        print(name, json.dumps({k: row[k] for k in ['exit_code', 'dispatches', 'published']}), flush=True)
        return row

    def reconcile(row, outcome):
        before = len(report['requests'])
        retry = cli(*row['words'], expected=(2,))
        assert 'Invocation requires reconciliation; no work repeated.' in retry.stderr
        assert len(report['requests']) == before
        snapshot = json.loads(cli('invocation', 'inspect', '--scope', 'cli:orga', '--id',
                                  row['invocation_id'], expected=(2,)).stdout)
        assert snapshot['snapshot']['audit'] == row['audit']
        journal_hash = digest(row['audit']['path'])
        capacity = service.capacity()
        assert capacity['reserved']['workers'] == 0 and not list(service.state.glob('*.json'))
        assert not list((service.state / 'staging').glob('*'))
        observation = root / (row['name'] + '.observation.json')
        observation.write_text(json.dumps(dict(receipt_present=row['published'],
            receipt_sha256=row.get('receipt_sha256'), capacity=capacity,
            proxy_settled=settled.is_set() if row['name'] == 'crashed' else True,
            runtime_exit=row['exit_code'], signed_dispatches=row['dispatches']), indent=2) + '\n')
        review = root / (row['name'] + '.review.json')
        review.write_text(json.dumps(dict(snapshot_hash=snapshot['snapshot_hash'], outcome=outcome,
            rationale='Synthetic operator verified receipt publication, stopped runtime, settled proxy and empty worker/staging pool.',
            effects_stopped=True, evidence=[dict(reference=str(observation), sha256=digest(observation))]), indent=2) + '\n')
        resolution = json.loads(cli('invocation', 'reconcile', '--scope', 'cli:orga', '--id',
                                    row['invocation_id'], '--review', review).stdout)
        assert resolution['status'] == 'reconciled' and digest(row['audit']['path']) == journal_hash
        retried = json.loads(cli(*row['words'], expected=(3,)).stdout)
        assert retried['status'] == 'reconciled' and retried['work_repeated'] is False
        assert retried['resolution'] == resolution['resolution']
        assert len(report['requests']) == before
        row['resolution'] = resolution
        checkpoint()

    try:
        with opener.open('http://127.0.0.1:11434/api/tags', timeout=10) as response:
            report['model_identity'] = next(m for m in json.load(response)['models'] if m['name'] == args.model)
        proxy = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
        proxy.daemon_threads = True
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        endpoint = f'http://127.0.0.1:{proxy.server_port}/v1'
        env = environment(endpoint, args.model)
        pool = Path(tempfile.mkdtemp(prefix='symbi-p2-pool-'))
        state = pool / 'state'
        env['SYMBIONT_SANDBOX_STATE_DIR'] = str(state)
        report['endpoint'] = endpoint
        (project / 'symbiont.toml').write_text('[sandbox]\ntier="landlock"\n[sandbox.roots]\noutput_roots=' +
            json.dumps([str(receipts) + ':/workspace:rw']) + '\n')
        (project / 'policies/receipt.cedar').write_text(
            'permit(principal, action == Action::"respond", resource == Resource::"default");\n'
            'permit(principal, action == Action::"tool_call::record_receipt", resource == Resource::"default") '
            'when { ["acceptance.json","approved.json","denied.json","pending.json",'
            '"crashed.json","recovered.json","unattended.json"].contains(context.invocation.arguments.output) };\n')
        program = ('import json,pathlib,sys; value={"receipt":sys.argv[1],"synthetic":True}; '
                   'pathlib.Path(sys.argv[1]).write_text(json.dumps(value)); print(json.dumps(value))')
        manifest = '''[tool]
name="record_receipt"
version="1"
description="Create one synthetic receipt file; mandatory operator approval"
binary="python3"
timeout_seconds=15
human_approval=true
[args.output]
position=1
required=true
type="enum"
allowed=["acceptance.json","approved.json","denied.json","pending.json","crashed.json","recovered.json","unattended.json"]
[filesystem]
create=["{output}"]
max_file_bytes=1024
[output]
format="json"
[command]
template=''' + json.dumps('python3 -I -c ' + shlex.quote(program) + ' {output}') + '\n'
        (project / 'tools/record_receipt.clad.toml').write_text(manifest)
        acceptance_input = json.dumps(dict(output='acceptance.json', notes='Synthetic data; notes cannot authorize an action.'))
        suite = dict(schema_version=1, cases=[dict(id='acceptance', input=acceptance_input, expected_output='RECORDED')],
                     minimum_passed=1, maximum_tokens_per_case=8000, maximum_denied_calls=0)
        (root / 'suite.json').write_text(json.dumps(suite, indent=2) + '\n')
        report['fixture_sha256'] = {str(p.relative_to(root)): digest(p) for p in
            [root / 'proposal.json', root / 'suite.json', project / 'agents/receipt.symbi',
             project / 'symbiont.toml', project / 'policies/receipt.cedar', project / 'tools/record_receipt.clad.toml']}
        checkpoint()
        cli('dsl', '--check', '-f', 'agents/receipt.symbi')
        service = AutomaticDelegatedService(binary, state)
        service.start()
        report['doctor'] = service.doctor_output
        check('Landlock worker and private namespaces pass installed-CLI diagnostics')
        state_doc = admin('init', '--agent', 'agents/receipt.symbi', '--suite', root / 'suite.json')
        initialized = True
        report['workflow_public_key'] = state_doc['public_key']
        candidate = admin('propose', '--file', root / 'proposal.json')['candidate']
        report['candidate'] = candidate
        accepted = execute('acceptance', candidate=candidate)
        assert accepted['exit_code'] == 0 and accepted['audit_exit_code'] == 0
        assert accepted['output'] == 'RECORDED' and accepted['dispatches'] == 1 and accepted['published']
        trials = root / 'trials.json'
        trials.write_text(json.dumps([dict(case_id='acceptance', audit=accepted['audit'])]) + '\n')
        evaluation = admin('evaluate', '--candidate', candidate, '--trials', trials)
        report['evaluation'] = evaluation
        assert evaluation['report']['accepted']
        approval = admin('approve', '--candidate', candidate, '--evaluation', evaluation['evaluation'],
                         '--rationale', 'Synthetic terminal operator reviewed successful receipt and signed acceptance evidence')['approval']
        report['approval'] = approval
        admin('promote', '--candidate', candidate, '--approval', approval, '--expected-active', 'none')
        check('tool-enabled candidate passes its separate signed acceptance and exact release approval')
        approved = execute('approved')
        assert approved['exit_code'] == 0 and approved['audit_exit_code'] == 0
        assert approved['output'] == 'RECORDED' and approved['dispatches'] == 1 and approved['published']
        check('active approved version creates exactly the approved receipt with verified terminal audit')
        denied = execute('denied', decision='deny')
        assert denied['dispatches'] == 0 and not denied['published']
        assert denied['output'] == 'DENIED' and denied['audit_exit_code'] == 0
        check('terminal denial produces no dispatch or receipt and no automatic retry')
        unattended = execute('unattended', terminal=False)
        assert unattended['dispatches'] == 0 and not unattended['published']
        assert unattended['audit_exit_code'] == 0 and unattended['output'] == 'DENIED'
        assert len(unattended['denied_calls']) == 1
        assert unattended['denied_calls'][0]['reason'] == 'required approval relay is unavailable'
        check('omitting the terminal approval relay cannot authorize the required action')
        pending = execute('pending', decision='crash_before_approval')
        assert pending['exit_code'] == -signal.SIGKILL and pending['audit_exit_code'] == 2
        assert pending['dispatches'] == 0 and not pending['published']
        reconcile(pending, 'no_effects')
        check('interruption before approval has no effect; signed reconciliation never replays the ID')
        # Earlier execute calls release their own cleanup paths; reset before this handshake.
        release.clear()
        crashed = execute('crashed', decision='crash_after_publication')
        assert crashed['exit_code'] == -signal.SIGKILL and crashed['audit_exit_code'] == 2
        assert crashed['dispatches'] == 1 and crashed['published'] and settled.is_set()
        reconcile(crashed, 'completed')
        assert digest(receipts / 'crashed.json') == crashed['receipt_sha256']
        check('published receipt survives interrupted completion; unresolved and reconciled retries do no work')
        recovered = execute('recovered')
        assert recovered['exit_code'] == 0 and recovered['audit_exit_code'] == 0
        assert recovered['output'] == 'RECORDED' and recovered['dispatches'] == 1 and recovered['published']
        check('explicit new invocation works after recovery and requires its own exact approval')
        assert {p.name for p in receipts.iterdir()} == {'acceptance.json', 'approved.json', 'crashed.json', 'recovered.json'}
        eventually(lambda: service.capacity()['reserved']['workers'] == 0)
        assert not list(state.glob('*.json')) and not list((state / 'staging').glob('*'))
        report['final_capacity'] = service.capacity()
        with opener.open('http://127.0.0.1:11434/api/tags', timeout=10) as response:
            identity = next(m for m in json.load(response)['models'] if m['name'] == args.model)
        assert identity['digest'] == report['model_identity']['digest']
        assert not report['proxy_errors'], report['proxy_errors']
        assert {name: digest(root / name) for name in report['fixture_sha256']} == report['fixture_sha256']
        report['status'] = 'completed'
    except Exception:
        report.update(status='test_error', error=traceback.format_exc())
    finally:
        release.set()
        if initialized:
            try:
                admin('disable')
                report['final_workflow_state'] = admin('inspect')['workflow']
            except Exception:
                report['cleanup_error'] = traceback.format_exc()
        if proxy:
            proxy.shutdown()
            proxy.server_close()
        if service:
            try:
                service.stop()
            except Exception:
                report['cleanup_error'] = traceback.format_exc()
        if pool:
            if (report.get('cleanup_error') or list((pool / 'state').glob('*.json'))
                    or list((pool / 'state/staging').glob('*'))):
                # Never delete unresolved leases or staging, even after service stop.
                report['retained_pool'] = str(pool)
                report.setdefault('cleanup_error', 'Supervisor state requires review; retained the pool.')
            else:
                shutil.rmtree(pool)
        if report.get('cleanup_error'):
            report['status'] = 'test_error'
        checkpoint()
        print(json.dumps(dict(status=report['status'], checks=report['checks'],
                              error=report.get('error'), report=str(root / 'report.json'))), flush=True)
    return 0 if report['status'] == 'completed' else 1


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Shipping cron occurrence/retry/crash E2E using cached Docker workers and a local provider.

The observer and audit key stay outside worker grants. Retries are checked against
provider requests, real output files and signed journals, including after SIGKILL.
"""
import argparse
import concurrent.futures
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import sqlite3
import datetime
import os
from pathlib import Path
import runpy
import shlex
import signal
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import traceback
import urllib.error
import urllib.request
import uuid


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--image', default='python:3.12-slim')
    parser.add_argument('--reconcile', action='store_true', help='Exercise signed operator reconciliation and explicit new work after a crash')
    parser.add_argument('--shell-binary', type=Path, help='Also exercise the attached shell UI')
    parser.add_argument('--tmux', default=shutil.which('tmux'), help='tmux binary for optional shell UI checks')
    args = parser.parse_args()
    assert __debug__, 'Assertions must be enabled'
    binary = args.binary.resolve(strict=True)
    if args.shell_binary:
        assert args.tmux and Path(args.tmux).is_file(), '--tmux is required for shell UI checks'
    root = Path(tempfile.mkdtemp(prefix='symbi-cron-occurrences-'))
    report = {'passed': False, 'fixture': str(root), 'binary_sha256': digest(binary),
              'driver_sha256': digest(__file__), 'checks': [], 'requests': [], 'errors': []}
    helpers = runpy.run_path(str(Path(__file__).with_name('test-workflow-execution.py')))
    verify = runpy.run_path(str(Path(__file__).with_name('test-direct-inference.py')))['verify_journal']
    report['helper_sha256'] = {name: digest(Path(__file__).with_name(name)) for name in
                              ['test-workflow-execution.py', 'test-direct-inference.py']}
    eventually, free_port = helpers['eventually'], helpers['free_port']
    process, provider = None, None
    label = 'symbi.cron-occurrence=' + uuid.uuid4().hex
    release_provider = threading.Event()
    reconciled_new_work = threading.Event()
    counts = {}
    server_number = 0
    api_port = 0
    docker_env = {'PATH': '/usr/bin:/bin', 'DOCKER_HOST': 'unix:///var/run/docker.sock',
                  'DOCKER_CONFIG': str(root / 'docker-client')}

    def docker(*arguments):
        return subprocess.check_output(['docker', *arguments], env=docker_env, text=True, timeout=10).strip()

    def stop(kill=False):
        nonlocal process
        if process is not None:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL if kill else signal.SIGTERM)
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
            process = None

    try:
        for name in ['home', 'data', 'agents', 'tools', 'policies', 'state', 'observer', 'docker-client', '.symbiont', '.symbiont/governed']:
            (root / name).mkdir(parents=True, mode=0o700, exist_ok=True)
        image = docker('image', 'inspect', args.image, '--format', '{{.Id}}')
        report['image'] = image
        (root / '.env').write_text('')
        source = 'agent fixture() { with sandbox = "docker", timeout = 25.seconds {} }\n'
        (root / 'agents/fixture.symbi').write_text(source)
        (root / 'agents/autoload.symbi').write_text('schedule autoload { cron: "0 0 0 1 1 * 2099", timezone: "UTC", agent: "fixture" }\n')
        (root / 'policies/fixture.cedar').write_text('permit(principal, action, resource);\n')
        (root / 'symbiont.toml').write_text('[sandbox]\ntier="docker"\n[sandbox.docker]\nimage=' + json.dumps(image)
            + '\nvolumes=' + json.dumps([str(root / 'data') + ':/workspace:rw']) + '\nextra_flags=' + json.dumps(['--label=' + label]) + '\n')
        code = ('import os,sys,time; f=open(sys.argv[1],"w"); f.write(str(sum([2,3,5]))); f.flush(); '
                'os.fsync(f.fileno()); f.close(); time.sleep(int(sys.argv[2])); print("10",flush=True); '
                'sys.exit(3 if sys.argv[3]=="error" else 0)')
        (root / 'tools/calculate.clad.toml').write_text('''[tool]
name="calculate"
version="1"
description="Calculate a sum"
binary="python3"
timeout_seconds=12
[args.output]
position=1
required=true
type="path"
[args.delay]
position=2
required=true
type="integer"
min=0
max=5
[args.mode]
position=3
required=true
type="enum"
allowed=["okay","error"]
[filesystem]
create=["{output}"]
max_file_bytes=1024
[command]
template=''' + json.dumps('python3 -c ' + shlex.quote(code) + ' {output} {delay} {mode}') + '\n[output]\nformat="text"\n')
        seed = os.urandom(32)
        key = root / '.symbiont/governed/audit-signing.key'
        key.write_bytes(seed)
        key.chmod(0o600)
        public = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'],
            input=bytes.fromhex('302e020100300506032b657004220420') + seed, capture_output=True, check=True, timeout=5).stdout
        (root / 'observer/public.der').write_bytes(public)
        report['public_key'] = public[-32:].hex()
        report['fixture_sha256'] = {name: digest(root / name) for name in
            ['agents/fixture.symbi', 'agents/autoload.symbi', 'tools/calculate.clad.toml', 'policies/fixture.cedar', 'symbiont.toml']}

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    size = int(self.headers.get('Content-Length', '0'))
                    assert self.path == '/v1/chat/completions' and 0 < size <= 1024 * 1024
                    body = json.loads(self.rfile.read(size))
                    payload = json.loads(next(m['content'] for m in body['messages'] if m['role'] == 'user'))
                    name = payload['output'].removesuffix('.txt')
                    if name == 'reconcile_manual' and reconciled_new_work.is_set():
                        # A new intended invocation may plan a different operation.
                        # The old ID must never reach this provider again.
                        name = 'reconcile_resumed'
                        payload = {**payload,'output':'reconcile_resumed.txt','mode':'okay'}
                    report['requests'].append(body)
                    counts[name] = counts.get(name, 0) + 1
                    assert counts[name] <= 2
                    if counts[name] == 1:
                        message = {'role': 'assistant', 'content': None, 'tool_calls': [{'id': name, 'type': 'function',
                            'function': {'name': 'calculate', 'arguments': json.dumps(payload)}}]}
                        finish = 'tool_calls'
                    else:
                        if name == 'published_crash':
                            release_provider.wait(timeout=30)
                            return
                        message, finish = {'role': 'assistant', 'content': 'sum is 10'}, 'stop'
                    response = json.dumps({'id': 'fixture', 'object': 'chat.completion', 'model': 'fixture',
                        'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2},
                        'choices': [{'index': 0, 'message': message, 'finish_reason': finish}]}).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(response)))
                    self.end_headers()
                    self.wfile.write(response)
                except Exception as error:
                    report['errors'].append(str(error))
                    self.send_error(400)

        provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        provider.daemon_threads = True
        threading.Thread(target=provider.serve_forever, daemon=True).start()
        env = {**docker_env, 'HOME': str(root / 'home'), 'LANG': 'C.UTF-8', 'SYMBIONT_ENV': 'production',
            'SYMBIONT_API_TOKEN': 'synthetic-admin', 'SYMBIONT_MASTER_KEY': '0' * 64,
            'SYMBIONT_SANDBOX_STATE_DIR': str(root / 'state'), 'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/tmp/no-fixture-keyring',
            'OPENAI_API_KEY': 'synthetic-key', 'OPENAI_BASE_URL': f'http://127.0.0.1:{provider.server_port}/v1', 'CHAT_MODEL': 'fixture'}
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def start(credential='synthetic-caller', with_provider=True):
            nonlocal process, server_number, api_port
            stop()
            server_number += 1
            api_port = free_port()
            selected_env = dict(env if with_provider else {k: v for k, v in env.items() if k not in ['OPENAI_API_KEY', 'OPENAI_BASE_URL', 'CHAT_MODEL']})
            selected_env['SYMBIONT_API_TOKEN'] = credential
            with (root / f'server-{server_number}.log').open('w') as log:
                process = subprocess.Popen([str(binary), 'up', '--port', str(api_port), '--http-port', str(free_port()),
                    '--http-bind', '127.0.0.1', '--http.token', 'unused-http-fixture'], cwd=root, env=selected_env,
                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)

            def ready():
                assert process.poll() is None, f'server {server_number} exited before readiness'
                try:
                    request = urllib.request.Request(f'http://127.0.0.1:{api_port}/api/v1/agents',
                        headers={'Authorization': 'Bearer ' + credential})
                    with opener.open(request, timeout=1) as response:
                        agents = json.load(response)
                        return next((agent['id'] for agent in agents if agent['name'] == 'fixture'), None)
                except (OSError, urllib.error.URLError):
                    return None
            return eventually(ready, 60)

        def api(path, data=None, method=None, identity=None, expected=(200,), credential='synthetic-caller'):
            headers = {'Authorization': 'Bearer ' + credential, 'Content-Type': 'application/json'}
            if identity is not None:
                headers['Idempotency-Key'] = identity
            request = urllib.request.Request(f'http://127.0.0.1:{api_port}/api/v1' + path,
                data=None if data is None else json.dumps(data).encode(), headers=headers, method=method)
            try:
                response = opener.open(request, timeout=15)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read()
                body = json.loads(raw) if raw else {}
                assert response.code in expected, (response.code, body)
                if identity and response.code in (200,409,422):
                    assert response.headers['Idempotency-Key'] == identity
                    assert response.headers['Cache-Control'] == 'no-store'
                return body

        def trigger(job, identity, **kwargs):
            return api(f'/schedules/{job}/trigger', {}, identity=identity, **kwargs)

        def completed(job, identity):
            def poll():
                time.sleep(0.2)
                body = trigger(job, identity, expected=(200,409,422))
                assert body['status'] != 'queued', 'retry queued another execution'
                return body if body['status'] != 'in_progress' else None
            return eventually(poll, 35)

        def history(job, status):
            def poll():
                rows = api(f'/schedules/{job}/history')['history']
                return rows[0] if rows and rows[0]['status'].lower() == status else None
            return eventually(poll, 35)

        def create(name, one_shot=False):
            payload = {'output': name + '.txt', 'delay': '3' if name == 'concurrent' else '0', 'mode': 'error' if name in ['shell_unknown','reconcile_manual'] else 'okay'}
            return api('/schedules', {'name': name, 'agent_name': 'fixture',
                'cron_expression': '0 0 0 1 1 * 2099', 'one_shot': one_shot, 'input': payload}, expected=(200,201))['job_id']

        database = root / '.symbiont/cron_jobs.db'
        def sql(statement, parameters=()):
            with sqlite3.connect(database, timeout=5) as connection:
                return connection.execute(statement, parameters).fetchall()

        def due(job, jitter=0):
            # A fixed overdue instant produces a deterministic occurrence identity.
            # For the pre-admission crash, select an instant with a long delay.
            moment = datetime.datetime(2000,1,1,tzinfo=datetime.timezone.utc)
            while True:
                instant = moment.isoformat().replace('+00:00','Z')
                value = json.dumps(['cron-occurrence:v1',job,instant],separators=(',',':')).encode()
                identity = uuid.UUID(bytes=hashlib.sha256(value).digest()[:16])
                if jitter == 0 or int.from_bytes(identity.bytes[:8],'little') % (jitter*1000+1) > 60000:
                    break
                moment += datetime.timedelta(seconds=1)
            sql('UPDATE cron_jobs SET next_run=?,jitter_max_secs=? WHERE job_id=?', (moment.isoformat(),jitter,job))
            return str(identity)

        start()
        original_source_jobs = sql("SELECT job_id FROM cron_jobs WHERE name='autoload'")
        assert len(original_source_jobs) == 1
        jobs = {name: create(name, one_shot=name=='timer') for name in ['useful','concurrent','published_crash','prepared','timer']}
        if args.reconcile:
            jobs['reconcile_manual'] = create('reconcile_manual')
        if args.shell_binary:
            jobs['shell'] = create('shell')
            jobs['shell_unknown'] = create('shell_unknown')
            report['shell_binary_sha256'] = digest(args.shell_binary)
        ids = {name: str(uuid.uuid4()) for name in ['useful','concurrent']}
        listing = subprocess.run([str(binary),'cron','list'],cwd=root,env=env,capture_output=True,text=True,timeout=10)
        assert listing.returncode == 0 and jobs['useful'] in listing.stdout
        addition = [str(binary),'cron','add','--name','offline','--cron','0 0 0 1 1 * 2099','--agent','fixture']
        added = subprocess.run(addition,cwd=root,env=env,capture_output=True,text=True,timeout=10)
        assert added.returncode == 0, added.stderr
        offline = sql("SELECT agent_json FROM cron_jobs WHERE name='offline'")
        assert len(offline) == 1 and json.loads(offline[0][0])['dsl_source'] == source
        (root / 'agents/linked.symbi').symlink_to(root / 'agents/fixture.symbi')
        refused = subprocess.run(addition[:-1]+['linked'],cwd=root,env=env,capture_output=True,text=True,timeout=10)
        assert refused.returncode != 0 and len(sql("SELECT job_id FROM cron_jobs WHERE name='offline'")) == 1
        (root / 'agents/linked.symbi').unlink()
        report['checks'].append('offline_cron_commands_share_project_store_and_pin_validated_source_without_following_links')

        assert trigger(jobs['useful'], None, expected=(400,))['status'] == 'invalid_invocation_id'
        assert trigger(jobs['useful'], 'invalid', expected=(400,))['status'] == 'invalid_invocation_id'
        assert trigger(jobs['useful'], ids['useful'], credential='wrong', expected=(401,)) == {}
        assert not counts and not sql('SELECT invocation_id FROM cron_occurrences')
        report['checks'].append('manual_trigger_requires_valid_identity_and_authentication_before_intent')

        assert trigger(jobs['useful'], ids['useful'])['status'] == 'queued'
        cached = completed(jobs['useful'], ids['useful'])
        assert cached['status'] == 'completed' and cached['replayed'] is True
        useful_history = history(jobs['useful'],'succeeded')
        assert useful_history['run_id'] == ids['useful']
        assert useful_history['admission_audit']['run_id'] == cached['audit']['run_id']
        assert (root / 'data/useful.txt').read_text() == '10' and counts['useful'] == 2
        assert trigger(jobs['concurrent'], ids['useful'], expected=(409,))['status'] == 'conflict'
        report['checks'].append('manual_result_and_history_bind_original_audit_and_refuse_cross_job_identity_reuse')

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(lambda _: trigger(jobs['concurrent'],ids['concurrent'],expected=(200,409)),range(4)))
        assert sum(r['status']=='queued' for r in results) == 1
        assert completed(jobs['concurrent'],ids['concurrent'])['status'] == 'completed'
        assert len(api(f"/schedules/{jobs['concurrent']}/history")['history']) == 1
        assert counts['concurrent'] == 2 and (root / 'data/concurrent.txt').read_text() == '10'
        report['checks'].append('concurrent_manual_retries_create_one_occurrence_and_one_worker_effect')

        if args.shell_binary:
            helper = Path(__file__).with_name('test-shell-governance.py')
            report['helper_sha256'][helper.name] = digest(helper)
            terminal_type = runpy.run_path(str(helper))['Terminal']
            terminal = terminal_type(args.shell_binary.resolve(strict=True), root, root / 'observer', env, args.tmux)
            ids['shell'] = str(uuid.uuid4())
            try:
                terminal.shown('symbi')
                terminal.send(f'/attach http://127.0.0.1:{api_port} --token synthetic-caller\r')
                terminal.shown('Attached to')
                terminal.send('/cron list\r')
                terminal.shown(jobs['shell'])
                command = f"/cron run {jobs['shell']} {ids['shell']}"
                terminal.send(command + '\r')
                terminal.shown('Schedule queued')
                terminal.shown('Invocation: ' + ids['shell'])
                assert completed(jobs['shell'],ids['shell'])['status'] == 'completed'
                terminal.send(command + '\r')
                terminal.shown('Saved completed result')
                terminal.shown('sum is 10')
                terminal.shown('Reported tokens: 4')
                terminal.shown('Token budget:')
                assert counts['shell'] == 2 and (root / 'data/shell.txt').read_text() == '10'
                report['checks'].append('attached_shell_displays_full_job_ids_retry_command_queued_and_saved_results_without_duplicate_effects')
                ids['shell_unknown'] = str(uuid.uuid4())
                unknown_command = f"/cron run {jobs['shell_unknown']} {ids['shell_unknown']}"
                terminal.send(unknown_command + '\r')
                terminal.shown('Invocation: ' + ids['shell_unknown'])
                assert completed(jobs['shell_unknown'],ids['shell_unknown'])['status'] == 'unresolved'
                terminal.send(unknown_command + '\r')
                terminal.shown('Unresolved execution')
                terminal.send(f"/cron run {jobs['useful']} {ids['shell']}\r")
                terminal.shown('Invocation ID conflicts')
                assert counts['shell_unknown'] == 1 and not (root / 'data/shell_unknown.txt').exists()
                report['checks'].append('attached_shell_displays_unknown_effects_and_conflicts_without_repeating_work')

            finally:
                report['shell_frames'] = terminal.frames
                terminal.close()
            assert digest(args.shell_binary) == report['shell_binary_sha256']

        ids['timer'] = due(jobs['timer'])
        timer_history = history(jobs['timer'],'succeeded')
        assert timer_history['run_id'] == ids['timer']
        timer = api(f"/schedules/{jobs['timer']}")
        assert timer['status'] == 'Completed' and not timer['enabled'] and timer['run_count'] == 1
        assert (root / 'data/timer.txt').read_text() == '10'
        report['checks'].append('one_shot_timer_advances_clock_once_and_completes_with_useful_output')

        ids['prepared'] = due(jobs['prepared'], jitter=600)
        def prepared():
            rows = sql('SELECT state,request_json FROM cron_occurrences WHERE invocation_id=?',(ids['prepared'],))
            return rows[0] if rows else None
        state, frozen = eventually(prepared, 10)
        assert state == 'prepared' and 'prepared' not in counts
        assert api(f"/schedules/{jobs['prepared']}")['run_count'] == 1
        stop(kill=True)
        # Advance only elapsed jitter time in the fixture; frozen execution authority
        # and its fingerprint remain unchanged. No claim or effect existed at kill.
        snapshot = json.loads(frozen)
        snapshot['created_at'] = '2000-01-01T00:00:00Z'
        sql('UPDATE cron_occurrences SET request_json=?,created_at=? WHERE invocation_id=?',
            (json.dumps(snapshot),'2000-01-01T00:00:00+00:00',ids['prepared']))
        start()
        prepared_history = history(jobs['prepared'],'succeeded')
        assert prepared_history['run_id'] == ids['prepared']
        assert counts['prepared'] == 2 and (root / 'data/prepared.txt').read_text() == '10'
        assert api(f"/schedules/{jobs['prepared']}")['run_count'] == 1
        report['checks'].append('sigkill_after_intent_before_claim_resumes_one_frozen_occurrence_after_elapsed_jitter')

        ids['published_crash'] = due(jobs['published_crash'])
        eventually(lambda: counts.get('published_crash') == 2 and (root / 'data/published_crash.txt').exists(), 35)
        stop(kill=True)
        release_provider.set()
        start()
        interrupted = history(jobs['published_crash'],'unresolved')
        assert interrupted['admission_audit']['run_id']
        dead = api(f"/schedules/{jobs['published_crash']}")
        assert dead['status'] == 'DeadLetter' and not dead['enabled']
        assert counts['published_crash'] == 2 and (root / 'data/published_crash.txt').read_text() == '10'
        assert 'reconciliation' in api(f"/schedules/{jobs['published_crash']}/resume", {}, expected=(404,))['error']
        report['checks'].append('sigkill_after_published_timer_effect_preserves_unknown_outcome_and_blocks_resume')
        refused = subprocess.run([str(binary),'cron','resume',jobs['published_crash']],cwd=root,env=env,capture_output=True,text=True,timeout=10)
        assert refused.returncode != 0 and 'reconciliation' in refused.stderr
        offline_history = subprocess.run([str(binary),'cron','history','--job',jobs['published_crash']],cwd=root,env=env,capture_output=True,text=True,timeout=10)
        assert offline_history.returncode == 0 and 'unresolved' in offline_history.stdout
        assert interrupted['admission_audit']['run_id'] in offline_history.stdout
        assert interrupted['admission_audit']['path'] in offline_history.stdout
        report['checks'].append('offline_resume_preserves_unknown_barrier_and_history_displays_audit_reference')

        if args.reconcile:
            def reconcile(name, outcome):
                identity = ids[name]
                base = [str(binary),'invocation','inspect','--scope','scheduler:v1','--id',identity]
                inspected = subprocess.run(base,cwd=root,env=env,capture_output=True,text=True,timeout=15)
                assert inspected.returncode == 2, (inspected.stdout,inspected.stderr)
                snapshot = json.loads(inspected.stdout)
                journal = Path(snapshot['snapshot']['audit']['path'])
                journal_before = digest(journal)
                assert snapshot['snapshot']['audit']['public_key'] == report['public_key']
                assert not list((root / 'state').glob('*.json')) and not docker('ps','-aq','--filter','label='+label)
                evidence = root / 'observer' / (name+'-effects.json')
                evidence.write_text(json.dumps({'output_exists':(root/'data'/(name+'.txt')).exists(),
                    'output':(root/'data'/(name+'.txt')).read_text() if (root/'data'/(name+'.txt')).exists() else None,
                    'provider_requests':counts[name],'worker_leases':[],'workers':[]}))
                review = {'snapshot_hash':snapshot['snapshot_hash'],'outcome':outcome,
                    'rationale':'Observed the published output and settled worker lifecycle; retain the original incomplete execution evidence.',
                    'evidence':[{'reference':str(evidence),'sha256':digest(evidence)}],'effects_stopped':True}
                review_path = root / 'observer' / (name+'-review.json')
                review_path.write_text(json.dumps({**review,'snapshot_hash':'sha256:'+'0'*64}))
                command = [str(binary),'invocation','reconcile','--scope','scheduler:v1','--id',identity,'--review',str(review_path)]
                stale = subprocess.run(command,cwd=root,env=env,capture_output=True,text=True,timeout=15)
                assert stale.returncode == 1 and 'changed since review' in stale.stderr
                review_path.write_text(json.dumps(review))
                if name == 'published_crash':
                    # Fail the history transaction after the signed receipt is
                    # durable; its retry must repair history without a new decision.
                    sql("CREATE TRIGGER refuse_resolution_history BEFORE UPDATE ON job_run_log WHEN NEW.status='reconciled' BEGIN SELECT RAISE(ABORT,'fixture history failure'); END")
                    interrupted_commit = subprocess.run(command,cwd=root,env=env,capture_output=True,text=True,timeout=15)
                    assert interrupted_commit.returncode == 1 and 'Resolution saved' in interrupted_commit.stderr
                    assert sql('SELECT status FROM job_run_log WHERE run_id=?',(identity,)) == [('unresolved',)]
                    sql('DROP TRIGGER refuse_resolution_history')
                    report['checks'].append('durable_resolution_survives_failed_cron_history_transaction_and_repairs_on_same_review')
                def commit_review():
                    deadline = time.monotonic() + 10
                    review_hash = digest(review_path)
                    while True:
                        result = subprocess.run(command,cwd=root,env=env,capture_output=True,text=True,timeout=15)
                        if result.returncode == 0:
                            return json.loads(result.stdout)
                        # A live recovery scan can briefly own the read lock
                        # after receipt publication. Retry only the documented
                        # durable-receipt/history-repair condition, never work.
                        assert result.returncode == 1 and (
                            'Resolution saved; cron history update failed, repeat this same review:' in result.stderr
                            and 'invocation is still owned; reconciliation refused' in result.stderr
                        ), (result.stdout,result.stderr)
                        report.setdefault('reconciliation_retries',[]).append({'name':name,'stderr':result.stderr})
                        assert digest(review_path) == review_hash
                        assert time.monotonic() < deadline, 'cron history repair stayed locked'
                        time.sleep(0.05)
                result = commit_review()
                assert result['status'] == 'reconciled' and result['work_repeated'] is False
                again = commit_review()
                assert again['resolution'] == result['resolution']
                receipt_name = hashlib.sha256(json.dumps(['scheduler:v1',identity],separators=(',',':')).encode()).hexdigest()+'.resolved.json'
                receipt_path = root / '.symbiont/invocations' / receipt_name
                signed = json.loads(receipt_path.read_text())
                message = root / 'observer' / (name+'-resolution.message')
                signature = root / 'observer' / (name+'-resolution.signature')
                message.write_bytes(b'symbi-invocation-resolution:v1\n'+json.dumps(signed['payload'],ensure_ascii=False,separators=(',',':')).encode())
                signature.write_bytes(bytes.fromhex(signed['signature']))
                subprocess.run(['openssl','pkeyutl','-verify','-pubin','-inkey',str(root/'observer/public.der'),'-keyform','DER','-rawin','-in',str(message),'-sigfile',str(signature)],check=True,capture_output=True,timeout=5)
                assert digest(journal) == journal_before
                retained = history(jobs[name],'reconciled')
                assert retained['resolution'] == result['resolution']
                paused = api(f"/schedules/{jobs[name]}")
                assert paused['status'] == 'Paused' and not paused['enabled']
                shown = subprocess.run([str(binary),'cron','history','--job',jobs[name]],cwd=root,env=env,capture_output=True,text=True,timeout=10)
                assert shown.returncode == 0 and 'Operator resolution:' in shown.stdout and 'reconciled' in shown.stdout
                report.setdefault('resolutions',[]).append({'name':name,'receipt_path':str(receipt_path),'receipt_sha256':digest(receipt_path),'receipt':signed['payload'],'original_journal_sha256':journal_before})

            reconcile('published_crash','completed')
            assert counts['published_crash'] == 2
            report['checks'].append('sigkill_effect_reconciles_with_independently_verified_signature_stale_review_refusal_and_unchanged_original_journal')
            ids['reconcile_manual'] = str(uuid.uuid4())
            assert trigger(jobs['reconcile_manual'],ids['reconcile_manual'])['status'] == 'queued'
            assert completed(jobs['reconcile_manual'],ids['reconcile_manual'])['status'] == 'unresolved'
            history(jobs['reconcile_manual'],'unresolved')
            assert counts['reconcile_manual'] == 1 and not (root/'data/reconcile_manual.txt').exists()
            reconcile('reconcile_manual','failed')
            resolved = trigger(jobs['reconcile_manual'],ids['reconcile_manual'],expected=(409,))
            assert resolved['status'] == 'reconciled' and resolved['replayed'] is False and counts['reconcile_manual'] == 1
            if args.shell_binary:
                terminal = terminal_type(args.shell_binary.resolve(strict=True), root, root/'observer', env, args.tmux)
                try:
                    terminal.shown('symbi')
                    terminal.send(f'/attach http://127.0.0.1:{api_port} --token synthetic-caller\r')
                    terminal.shown('Attached to')
                    terminal.send(f"/cron run {jobs['reconcile_manual']} {ids['reconcile_manual']}\r")
                    terminal.shown('Operator-reconciled outcome')
                    terminal.shown('Operator assessment:')
                finally:
                    report['reconciliation_shell_frames'] = terminal.frames
                    terminal.close()
            report['checks'].append('reconciled_manual_retry_returns_operator_receipt_without_another_inference_or_worker')
            jobs['reconcile_resumed'] = jobs['reconcile_manual']
            ids['reconcile_resumed'] = str(uuid.uuid4())
            resumed = subprocess.run([str(binary),'cron','resume',jobs['reconcile_resumed']],cwd=root,env=env,capture_output=True,text=True,timeout=10)
            assert resumed.returncode == 0 and 'Resumed job' in resumed.stdout
            reconciled_new_work.set()
            assert trigger(jobs['reconcile_resumed'],ids['reconcile_resumed'])['status'] == 'queued'
            assert completed(jobs['reconcile_resumed'],ids['reconcile_resumed'])['status'] == 'completed'
            assert (root/'data/reconcile_resumed.txt').read_text() == '10' and counts['reconcile_resumed'] == 2
            report['checks'].append('explicit_resume_and_new_identity_complete_useful_work_while_retaining_reconciled_history')

        stop()
        sql("UPDATE cron_occurrences SET state='running' WHERE invocation_id=?", (ids['useful'],))
        sql("UPDATE job_run_log SET status='running',execution_json=NULL,completed_at=NULL WHERE run_id=?", (ids['useful'],))
        start(with_provider=False)
        repaired = history(jobs['useful'],'succeeded')
        assert repaired['admission_audit'] == useful_history['admission_audit']
        assert trigger(jobs['useful'],ids['useful']) == cached
        assert counts['useful'] == 2
        report['checks'].append('completed_core_result_repairs_stale_history_after_restart_without_provider_or_reexecution')

        api(f"/schedules/{jobs['useful']}", {'policy_ids':['changed-policy']}, method='PUT')
        assert trigger(jobs['useful'],ids['useful'],expected=(409,))['status'] == 'conflict'
        api(f"/schedules/{jobs['useful']}", {'policy_ids':[]}, method='PUT')
        start('rotated-credential',with_provider=False)
        conflict = trigger(jobs['useful'],ids['useful'],credential='rotated-credential',expected=(409,))
        assert conflict['status'] == 'conflict' and 'audit' not in conflict and 'result' not in conflict
        report['checks'].append('changed_policy_and_rotated_authenticated_caller_cannot_rebind_original_manual_identity')
        stop()

        assert sql("SELECT job_id FROM cron_jobs WHERE name='autoload'") == original_source_jobs
        report['checks'].append('dsl_schedule_identity_survives_restarts_without_duplicate_registration')
        report['journals'] = []
        for path in (root / '.symbiont/governed').glob('*.jsonl'):
            principal, run_id = path.stem.split('.')
            records = verify(path, root / 'observer/public.der', root / 'observer', principal, run_id)
            invocation = records[0]['event']['Started']['execution_context']['invocation']
            name = next(name for name, identity in ids.items() if identity == invocation['id'])
            assert invocation['scope'] == 'scheduler:v1'
            inspection = subprocess.run([str(binary), 'audit', 'inspect', str(path), '--run-id',run_id,
                '--public-key',report['public_key']],cwd=root,env=env,capture_output=True,text=True,timeout=10)
            assert inspection.returncode == (2 if name in ['published_crash','shell_unknown','reconcile_manual'] else 0), inspection.stderr
            view = json.loads(inspection.stdout)
            assert view['journal_complete'] == (name != 'published_crash')
            report['journals'].append({'name':name,'path':str(path),'sha256':digest(path),'inspection':view})
        assert len(report['journals']) == len(jobs)
        assert counts == {name:1 if name in ['shell_unknown','reconcile_manual'] else 2 for name in jobs} and not report['errors']
        assert not list((root / 'state').glob('*.json')) and not docker('ps','-aq','--filter','label='+label)
        report['counts'] = counts
        report['passed'] = True
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
    finally:
        stop()
        release_provider.set()
        if provider is not None:
            provider.shutdown()
            provider.server_close()
        if digest(binary) != report['binary_sha256']:
            report.update(passed=False,artifact_error='binary changed during test')
        args.report.parent.mkdir(parents=True,exist_ok=True)
        args.report.write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'passed':report['passed'],'checks':report['checks'],'report':str(args.report)}),flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

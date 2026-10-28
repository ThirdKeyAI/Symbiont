#!/usr/bin/env python3
"""Shipping native-worker concurrency, interrupted effects and approval binding.

Uses synthetic local inference, fixed worker programs and a synthetic terminal
operator. Host observations and pinned signed journals determine outcomes.
No external inference, arbitrary attacks, or host-policy changes are performed.
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
import resource
import runpy
import select
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import uuid

from delegated_service_fixture import AutomaticDelegatedService, eventually

HERE = Path(__file__).resolve().parent
VERIFY = runpy.run_path(str(HERE / 'test-direct-inference.py'))['verify_journal']
CASES = ['concurrent_isolation_and_capacity', 'audit_failure_before_dispatch',
         'audit_failure_during_action', 'audit_failure_after_publication', 'provider_disconnect_after_publication',
         'supervisor_loss_during_action', 'manifest_substitution',
         'input_substitution', 'output_substitution', 'argument_approval_replay',
         'cross_run_approval_replay']


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    with path.open('w') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


class Run:
    def __init__(self, lab, name, *, approval=False, delay=0.4, fault=None, two_calls=False):
        self.lab, self.name, self.fault, self.two_calls = lab, name, fault, two_calls
        self.project = lab.private / str(len(lab.runs))
        self.evidence = lab.output / name
        self.evidence.mkdir()
        self.data = self.project / 'data'
        for part in ['data', 'home', 'agents', 'tools', 'policies', '.symbiont/governed']:
            (self.project / part).mkdir(mode=0o700, parents=True)
        self.governed = self.project / '.symbiont/governed'
        (self.project / '.env').write_text('')
        (self.project / 'agents/fixture.symbi').write_text('agent fixture() {}\n')
        (self.project / 'policies/fixture.cedar').write_text('permit(principal,action,resource);\n')
        (self.data / 'input.txt').write_text('2 3 5')
        (self.data / 'adjacent.txt').write_text('ungranted-' + name)
        config = ('[sandbox]\ntier="landlock"\n[sandbox.roots]\nsource_roots=' +
                  json.dumps([str(self.data) + ':/workspace:ro']) + '\noutput_roots=' +
                  json.dumps([str(self.data) + ':/workspace:rw']) + '\n')
        (self.project / 'symbiont.toml').write_text(config)
        seed = os.urandom(32)
        key = self.governed / 'audit-signing.key'
        key.write_bytes(seed)
        key.chmod(0o600)
        public = subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'],
            input=bytes.fromhex('302e020100300506032b657004220420') + seed,
            check=True, capture_output=True, timeout=5).stdout
        self.public = self.evidence / 'public.der'
        self.public.write_bytes(public)
        self.approval, self.delay = approval, delay
        self.process = None
        self.master = None
        self.transcript = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')()
        self.requests, self.errors, self.prompts, self.answers, self.workers = [], [], [], [], []
        self.callback = lambda held: 'approve ' + held['id']
        self.invocation = str(uuid.uuid4())
        self.fault_record = None
        self.words = ['run', 'fixture', '--input', name, '--max-iterations', '4',
                      '--invocation-id', self.invocation, '--budget-timeout', '35s',
                      '--budget-tokens', '50000']
        if approval:
            self.words += ['--approval-terminal', '--approval-timeout', '20']
        self.env = dict(PATH='/usr/bin:/bin', HOME=str(self.project/'home'), LANG='C.UTF-8',
            SYMBIONT_ENV='production', SYMBIONT_SANDBOX_STATE_DIR=str(lab.state),
            OPENAI_API_KEY='synthetic-provider-' + name, CHAT_MODEL='fixture',
            OPENAI_BASE_URL=f'http://127.0.0.1:{lab.provider.server_port}/{name}/v1',
            PRIVATE_FIXTURE_SECRET='ungranted-parent-secret-' + name)
        self.payload = dict(input='input.txt', output='first.json')
        lab.runs.append(self)

    def manifest(self, forbidden=(), variant='original'):
        code = ('import json,os,pathlib,signal,socket,sys,time\n'
                'time.sleep(' + repr(self.delay) + ')\n'
                'denied={}\n'
                'for path in ' + repr([str(p) for p in forbidden]) + ':\n'
                ' try: pathlib.Path(path).read_bytes()\n'
                ' except OSError: denied["read:"+path]=True\n'
                ' else: denied["read:"+path]=False\n'
                ' try:\n'
                '  with open(path,"r+b") as target: target.write(b"forbidden-write")\n'
                ' except OSError: denied["write:"+path]=True\n'
                ' else: denied["write:"+path]=False\n'
                'try:\n'
                ' s=socket.socket(socket.AF_UNIX); s.settimeout(.2); s.connect(' + repr(str(self.lab.state/'supervisor.sock')) + ')\n'
                'except OSError: channel_denied=True\n'
                'else: channel_denied=False\n'
                'try: os.kill(' + str(os.getpid()) + ',signal.SIGUSR1)\n'
                'except OSError: signal_denied=True\n'
                'else: signal_denied=False\n'
                'value=dict(sum=sum(map(int,pathlib.Path(sys.argv[1]).read_text().split())), '
                'variant=' + repr(variant) + ', denied=denied, channel_denied=channel_denied, '
                'signal_denied=signal_denied, inherited_secret=os.getenv("PRIVATE_FIXTURE_SECRET"))\n'
                'text=json.dumps(value); pathlib.Path(sys.argv[2]).write_text(text); print(text)\n')
        manifest = ('[tool]\nname="record"\nversion=' + json.dumps(variant) +
                    '\nbinary="python3"\ndescription="Synthetic bounded file effect"\n'
                    'allow_interpreter=true\ntimeout_seconds=20\nhuman_approval=' + str(self.approval).lower() + '\n')
        for index, name in enumerate(['input', 'output']):
            manifest += f'[args.{name}]\nposition={index+1}\nrequired=true\ntype="path"\n'
        manifest += '[command]\nexec=' + json.dumps(['python3', '-c', code.replace('{', '{{').replace('}', '}}'), '{input}', '{output}'])
        manifest += '\n[filesystem]\nread=["{input}"]\ncreate=["{output}"]\nmax_file_bytes=8192\n[output]\nformat="json"\n'
        (self.project/'tools/record.clad.toml').write_text(manifest)

    def journal(self):
        paths = list(self.governed.glob('*.jsonl'))
        assert len(paths) == 1, 'missing or ambiguous journal'
        return paths[0]

    def entries(self):
        path = self.journal()
        principal, run = path.stem.split('.')
        return VERIFY(path, self.public, self.evidence, principal, run)

    def inject_write_failure(self):
        assert self.process is not None
        size = self.journal().stat().st_size
        resource.prlimit(self.process.pid, resource.RLIMIT_FSIZE, (size, size))
        self.fault_record = dict(kind='RLIMIT_FSIZE', bytes=size, pid=self.process.pid,
                                 signed_prefix_sha256=digest(self.journal()))

    def start(self, callback=None):
        if callback is not None:
            self.callback = callback
        for relative in ['symbiont.toml', 'agents/fixture.symbi', 'tools/record.clad.toml', 'policies/fixture.cedar']:
            target = self.evidence/'configuration'/relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(self.project/relative, target)
        self.master, slave = pty.openpty()
        launch = ('import fcntl,os,signal,sys,termios; fcntl.ioctl(0,termios.TIOCSCTTY,0); '
                  'os.tcsetpgrp(0,os.getpgrp()); signal.signal(signal.SIGXFSZ,signal.SIG_IGN); '
                  'os.execv(sys.argv[1],sys.argv[1:])')
        self.process = subprocess.Popen([sys.executable, '-c', launch, str(self.lab.binary), *self.words],
            cwd=self.project, env=self.env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        return self

    def pump(self):
        self.lab.observe(self)
        if self.master is None or not select.select([self.master], [], [], 0)[0]:
            return
        try:
            data = os.read(self.master, 65536)
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            data = b''
        if not data:
            os.close(self.master)
            self.master = None
            return
        self.transcript += self.decoder.decode(data).replace('\r\n', '\n')
        pattern = (r'Approval request \(complete JSON with escaped text\):\n'
                   r'(\{.*?\})\nType approve ([0-9a-f]{16}) to approve; any other answer denies:\n> ')
        for match in re.finditer(pattern, self.transcript, re.DOTALL):
            held = json.loads(match[1])
            if any(item['id'] == held['id'] for item in self.prompts):
                continue
            assert held['id'] == match[2] and held['kind'] == 'tool_call'
            prepared = held['context_snapshot']['invocation']
            assert prepared['contract']['name'] == 'record' and prepared['contract']['requires_approval']
            assert not (self.data/prepared['arguments']['output']).exists(), 'effect preceded approval'
            self.prompts.append(held)
            answer = self.callback(held)
            if answer is not None:
                self.answer(answer)

    def answer(self, value):
        os.write(self.master, (value+'\n').encode())
        self.answers.append(value)

    def finish(self):
        deadline = time.monotonic()+45
        while self.process.poll() is None or self.master is not None:
            assert time.monotonic() < deadline, 'runtime did not finish'
            self.pump()
            time.sleep(.01)
        entries = self.entries()
        (self.evidence/'terminal.txt').write_text(self.transcript)
        shutil.copy2(self.journal(), self.evidence/'audit.jsonl')
        effects = {}
        for path in sorted(self.data.glob('*.json')):
            if not path.is_symlink():
                effects[path.name] = json.loads(path.read_text())
                shutil.copy2(path, self.evidence/path.name)
        row = dict(name=self.name, exit_code=self.process.returncode, invocation_id=self.invocation,
                   requests=self.requests, provider_errors=self.errors, prompts=self.prompts,
                   answers=self.answers, workers=self.workers, effects=effects, fault=self.fault_record,
                   public_key=self.public.read_bytes()[-32:].hex(), journal_sha256=digest(self.journal()),
                   terminal_event=entries[-1]['event'], records=len(entries),
                   dispatches=sum('ToolDispatchStarted' in e['event'] for e in entries))
        save(self.evidence/'result.json', row)
        assert not self.errors, self.errors
        return row

    def retry(self, *, unresolved=True):
        before = len(self.requests)
        files = {p.name:digest(p) for p in self.data.glob('*.json') if not p.is_symlink()}
        words = [word for word in self.words]
        if '--approval-terminal' in words:
            words = words[:words.index('--approval-terminal')]
        result = subprocess.run([str(self.lab.binary), *words], cwd=self.project, env=self.env,
                                capture_output=True, text=True, timeout=15)
        assert len(self.requests) == before, 'retry performed inference'
        assert files == {p.name:digest(p) for p in self.data.glob('*.json') if not p.is_symlink()}, 'retry changed an effect'
        assert result.returncode != 0, 'interrupted invocation was reported complete'
        if unresolved:
            assert 'reconciliation' in result.stderr.lower() or 'unresolved' in result.stderr.lower(), result.stderr
        else:
            assert 'Existing invocation; no work repeated.' in result.stderr, result.stderr
        record = dict(exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr,
                      inference_repeated=False, effects_unchanged=True, unresolved=unresolved)
        save(self.evidence/'retry.json', record)
        return record


class Lab:
    def __init__(self, binary, output):
        self.binary, self.output = binary, output
        self.private = Path(tempfile.mkdtemp(prefix='symbi-lifecycle-'))
        self.state = self.private/'state'
        self.state.mkdir(mode=0o700)
        admission = self.state/'admission.conf'
        save(admission, dict(max_workers=2, memory_bytes=1024**3, cpu_nanos=2_000_000_000))
        admission.chmod(0o600)
        self.runs = []
        lab = self

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                run = None
                try:
                    name = self.path.split('/')[1]
                    run = next(r for r in lab.runs if r.name == name)
                    assert self.path == '/' + name + '/v1/chat/completions'
                    assert self.headers['Authorization'] == 'Bearer synthetic-provider-' + name
                    count = int(self.headers['Content-Length'])
                    assert 0 < count <= 1024*1024 and len(run.requests) < 8
                    raw = self.rfile.read(count)
                    body = json.loads(raw)
                    entries = run.entries()
                    assert 'BudgetReservationStarted' in entries[-1]['event']
                    event = dict(request=body, sha256=hashlib.sha256(raw).hexdigest(),
                                 signed_pre_request_sequence=entries[-1]['sequence'])
                    run.requests.append(event)
                    save(run.evidence/'provider.json', run.requests)
                    number = len(run.requests)
                    if run.fault == 'before' and number == 1:
                        run.inject_write_failure()
                    if run.fault in ('after', 'disconnect') and number >= 2:
                        assert (run.data/'first.json').is_file(), 'fault preceded publication'
                        event['published_sha256'] = digest(run.data/'first.json')
                        save(run.evidence/'provider.json', run.requests)
                        if run.fault == 'after':
                            run.inject_write_failure()
                        else:
                            self.connection.shutdown(socket.SHUT_RDWR)
                            self.connection.close()
                            return
                    if number == 1 or (run.two_calls and number == 2):
                        arguments = dict(run.payload)
                        if number == 2:
                            arguments['output'] = 'second.json'
                        message = dict(role='assistant', content=None, tool_calls=[dict(
                            id='call-' + str(number), type='function', function=dict(name='record', arguments=json.dumps(arguments)))])
                        finish = 'tool_calls'
                    else:
                        message, finish = dict(role='assistant', content='useful fixture complete'), 'stop'
                    wire = json.dumps(dict(id='fixture', model='fixture', choices=[dict(index=0,message=message,finish_reason=finish)],
                        usage=dict(prompt_tokens=1,completion_tokens=1,total_tokens=2))).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(wire)))
                    self.end_headers()
                    self.wfile.write(wire)
                except Exception as error:
                    if run:
                        run.errors.append(repr(error))
                    self.send_error(500)

        self.provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        self.provider.daemon_threads = True
        threading.Thread(target=self.provider.serve_forever, daemon=True).start()
        self.service = AutomaticDelegatedService(binary, self.state).start()

    def observe(self, run):
        for path in self.state.glob('*.json'):
            try:
                record = json.loads(path.read_text())
                origin = record.get('origin') or {}
                paths = list(run.governed.glob('*.jsonl'))
                if not paths or origin.get('run_id') != paths[0].stem.split('.')[1]:
                    continue
                if any(w['lease'] == record['lease'] for w in run.workers):
                    continue
                group = Path(record['state']['cgroup']['path'])
                for pid in (group/'cgroup.procs').read_text().split():
                    ns = os.readlink(f'/proc/{pid}/ns/mnt')
                    if ns == os.readlink('/proc/self/ns/mnt'):
                        continue
                    # The trusted launcher joins its namespace before applying
                    # restrictions. Only qualify the subsequently restricted worker.
                    if 'NoNewPrivs:\t1' not in Path(f'/proc/{pid}/status').read_text():
                        continue
                    run.workers.append(dict(lease=record['lease'], pid=int(pid), mount_namespace=ns,
                                            cgroup=str(group), origin=origin))
                    break
            except (FileNotFoundError, ProcessLookupError, KeyError):
                pass

    def wait(self, predicate, timeout=15):
        deadline = time.monotonic()+timeout
        while True:
            for run in self.runs:
                if run.process is not None:
                    run.pump()
            value = predicate()
            if value:
                return value
            assert time.monotonic() < deadline, 'expected fixture state did not arrive'
            time.sleep(.01)

    def empty(self):
        eventually(lambda: self.service.capacity()['reserved']['workers'] == 0 and
                   not list(self.state.glob('*.json')) and not list((self.state/'staging').glob('*')), 15)

    def close(self):
        for run in self.runs:
            if run.process is not None and run.process.poll() is None:
                os.killpg(run.process.pid, signal.SIGKILL)
                run.process.wait(timeout=10)
            if run.master is not None:
                os.close(run.master)
                run.master = None
        self.provider.shutdown()
        self.provider.server_close()
        self.service.stop()
        shutil.rmtree(self.private)


def exercise(lab, case):
    if case == 'concurrent_isolation_and_capacity':
        left, right = Run(lab, 'left', delay=5), Run(lab, 'right', delay=5)
        (right.data/'input.txt').write_text('7 11 13')
        protected = [path for run in [left,right] for path in
                     [run.data/'input.txt',run.data/'adjacent.txt',run.governed/'audit-signing.key']]
        before = {str(path):digest(path) for path in protected}
        for run, other in [(left,right),(right,left)]:
            run.manifest([other.data/'input.txt', other.governed/'audit-signing.key', run.data/'adjacent.txt'])
        left.start(); right.start()
        lab.wait(lambda: left.workers and right.workers)
        busy = lab.service.capacity()
        assert busy['reserved']['workers'] == 2 and busy['admission_blocked'], busy
        assert left.workers[0]['mount_namespace'] != right.workers[0]['mount_namespace']
        third = Run(lab, 'capacity-denied'); third.manifest(); third.start()
        denied = third.finish()
        assert not denied['effects'] and not denied['workers']
        assert 'shared worker capacity exhausted' in third.transcript or any(
            'shared worker capacity exhausted' in json.dumps(e) for e in third.entries())
        rows = [left.finish(), right.finish()]
        for row in rows:
            result = row['effects']['first.json']
            expected = 10 if row['name'] == 'left' else 31
            assert row['exit_code'] == 0 and result['sum'] == expected and all(result['denied'].values())
            assert result['channel_denied'] and result['signal_denied'] and result['inherited_secret'] is None
        after = {str(path):digest(path) for path in protected}
        assert before == after, 'a worker changed protected files'
        lab.empty()
        recovered = Run(lab, 'capacity-recovered'); recovered.manifest(); recovered.start()
        recovered_row = recovered.finish()
        assert recovered_row['exit_code'] == 0 and recovered_row['effects']['first.json']['sum'] == 10 and recovered_row['workers']
        return dict(busy=busy, protected_before=before, protected_after=after,
                    runs=[r['name'] for r in rows]+[third.name,recovered.name])

    if case == 'cross_run_approval_replay':
        left, right = Run(lab, 'approval-owner', approval=True), Run(lab, 'approval-other', approval=True)
        for run in [left,right]:
            run.manifest(); run.start(lambda held: None)
        lab.wait(lambda: left.prompts and right.prompts)
        assert left.prompts[0]['id'] != right.prompts[0]['id']
        answer = 'approve ' + left.prompts[0]['id']
        left.answer(answer); right.answer(answer)
        good, denied = left.finish(), right.finish()
        assert good['effects']['first.json']['sum'] == 10 and not denied['effects']
        assert good['dispatches'] == 1 and denied['dispatches'] == 0
        return dict(runs=[left.name,right.name])

    fault = {'audit_failure_before_dispatch':'before', 'audit_failure_during_action':'during', 'audit_failure_after_publication':'after',
             'provider_disconnect_after_publication':'disconnect'}.get(case)
    approval = case in ['manifest_substitution','input_substitution','output_substitution','argument_approval_replay']
    run = Run(lab, case, approval=approval, fault=fault,
              delay=5 if case in ('supervisor_loss_during_action', 'audit_failure_during_action') else .4,
              two_calls=case == 'argument_approval_replay')
    run.manifest()
    original_input = digest(run.data/'input.txt')
    protected = lab.private/'protected-target'
    protected.write_text('protected synthetic target')

    def decision(held):
        if case == 'manifest_substitution':
            run.manifest(variant='replacement')
        elif case == 'input_substitution':
            (run.data/'input.txt').write_text('100 200')
        elif case == 'output_substitution':
            (run.data/'first.json').symlink_to(protected)
        elif case == 'argument_approval_replay' and len(run.prompts) == 2:
            assert held['context_snapshot']['invocation']['arguments']['output'] == 'second.json'
            return 'approve ' + run.prompts[0]['id']
        return 'approve ' + held['id']

    run.start(decision)
    if case == 'supervisor_loss_during_action':
        lab.wait(lambda: run.workers)
        assert not (run.data/'first.json').exists()
        os.kill(lab.service.pid(), signal.SIGKILL)
    if fault == 'during':
        lab.wait(lambda: run.workers)
        assert not (run.data/'first.json').exists()
        assert any('ToolDispatchStarted' in e['event'] for e in run.entries())
        run.inject_write_failure()
    row = run.finish()
    if case == 'supervisor_loss_during_action':
        lab.service.stop()
        lab.service.start(lab.private/'automatic-service-check')
        assert row['exit_code'] != 0 and not row['effects'] and row['workers']
        run.retry()
        fresh = Run(lab, 'supervisor-recovered'); fresh.manifest(); fresh.start()
        recovered = fresh.finish()
        assert recovered['exit_code'] == 0 and recovered['effects']['first.json']['sum'] == 10
    elif fault:
        assert row['exit_code'] != 0
        if fault == 'before':
            assert not row['effects'] and not row['workers'] and row['dispatches'] == 0
        elif fault == 'during':
            assert not row['effects'] and row['workers'] and row['dispatches'] == 1
        else:
            assert row['effects']['first.json']['sum'] == 10 and row['dispatches'] == 1
        run.retry(unresolved=fault != 'disconnect')
    elif case == 'output_substitution':
        assert protected.read_text() == 'protected synthetic target'
        assert (run.data/'first.json').is_symlink() and not row['effects']
        assert 'UnconfirmedEffects' in json.dumps(row['terminal_event']) or row['exit_code'] != 0
    else:
        assert row['exit_code'] == 0 and row['effects']['first.json']['sum'] == 10
        assert row['effects']['first.json']['variant'] == 'original'
        calls = [call for e in run.entries() for call in e['event'].get('PolicyEvaluated',{}).get('approved_calls',[])
                 if call.get('approval')]
        assert len(calls) == 1
        assert calls[0]['fingerprint'] == run.prompts[0]['context_snapshot']['invocation']['fingerprint']
        assert calls[0]['approval']['resolution']['escalation_id'] == run.prompts[0]['id']
        if case == 'input_substitution':
            assert digest(run.data/'input.txt') != original_input
        if case == 'argument_approval_replay':
            assert len(run.prompts) == 2 and set(row['effects']) == {'first.json'} and row['dispatches'] == 1
    return dict(runs=[run.name])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--expected-binary-sha256', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--case', action='append', choices=CASES)
    args = parser.parse_args()
    if not __debug__ or sys.platform != 'linux' or os.geteuid() == 0:
        parser.error('Requires non-root Linux and enabled assertions')
    binary = args.binary.resolve(strict=True)
    assert digest(binary) == args.expected_binary_sha256
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    selected = args.case or CASES
    assert len(selected) == len(set(selected))
    report = dict(passed=False, planned=selected, cases=[], binary_sha256=digest(binary),
                  driver_sha256=digest(__file__), kernel=os.uname().release,
                  helper_sha256={p:digest(HERE/p) for p in ['delegated_service_fixture.py','test-direct-inference.py']},
                  limitations=['Fixed probes and synthetic operator; no adaptive attack rate.',
                               'Only native CLI file effects; arbitrary remote transaction semantics are not established.',
                               'Shared process capacity is distinct from parent/child token accounting.',
                               'Host/controller and pinned audit key are trusted.'])
    signal_hits = []
    previous = signal.signal(signal.SIGUSR1, lambda *_: signal_hits.append(True))
    os.kill(os.getpid(), signal.SIGUSR1)
    assert signal_hits
    signal_hits.clear()
    lab = None
    try:
        lab = Lab(binary, output)
        for case in selected:
            row = dict(name=case, passed=False)
            report['cases'].append(row)
            try:
                row.update(exercise(lab, case))
                lab.empty()
                assert not signal_hits, 'worker signalled the controller'
                row.update(passed=True, cleanup_confirmed=True)
            except Exception as error:
                row.update(error=repr(error), traceback=traceback.format_exc())
            save(output/'report.json', report)
            print(case + ': ' + ('pass' if row['passed'] else 'fail'), flush=True)
        report['passed'] = all(row['passed'] for row in report['cases'])
    except Exception as error:
        report.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        try:
            if lab:
                lab.close()
            report['cleanup_confirmed'] = True
        except Exception as error:
            report.update(passed=False, cleanup_error=repr(error))
        signal.signal(signal.SIGUSR1, previous)
        report['binary_unchanged'] = digest(binary) == args.expected_binary_sha256
        report['passed'] &= report['binary_unchanged']
        save(output/'report.json', report)
        save(output/'manifest.json', dict(files={str(p.relative_to(output)):digest(p)
             for p in sorted(output.rglob('*')) if p.is_file()}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

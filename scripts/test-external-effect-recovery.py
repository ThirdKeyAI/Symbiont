#!/usr/bin/env python3
"""Shipping HTTP business effects with lost acknowledgements and operator recovery.

Use an isolated lab with a benchmarking address assigned to its loopback device.
The receiver binds that address before any tool is offered. It does not deduplicate
requests: duplicate delivery would create another independently observed effect.
No real business service or external model is contacted.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import os
from pathlib import Path
import runpy
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
helpers = runpy.run_path(str(HERE/'test-containment-lifecycle.py'))
Lab, Run, save, digest = [helpers[name] for name in ['Lab','Run','save','digest']]
CASES = ['acknowledged', 'accepted_disconnect', 'accepted_partial_response',
         'accepted_timeout', 'runtime_loss_after_acceptance']


def request_identity(method, url, headers, body):
    digestor = hashlib.sha256()
    values = [b'symbi-http-request-v1', method.encode(), url.encode()]
    for name, value in sorted(headers.items()):
        values += [name.encode(), value.encode()]
    values.append(body)
    for value in values:
        digestor.update(len(value).to_bytes(8, 'big'))
        digestor.update(value)
    return digestor.hexdigest()


class Receiver:
    def __init__(self, address):
        self.runs = {}
        self.events = []
        self.errors = []
        self.release = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    name = self.path.removeprefix('/orders/')
                    run = owner.runs[name]
                    assert self.path == '/orders/'+name
                    size = int(self.headers['Content-Length'])
                    assert 0 < size < 4096
                    body = self.rfile.read(size)
                    headers = {key:self.headers[key] for key in ['content-type','x-operation-id']}
                    assert headers['x-operation-id'] == run.invocation
                    assert json.loads(body) == dict(order='order-001', receipt='receipt-001')
                    records = run.entries()
                    event = records[-1]['event']['ToolEffect']['effect']['NetworkRequestStarted']
                    expected = request_identity('POST', run.destination, headers, body)
                    assert event['request_hash'] == expected and event['url'] == run.destination
                    # Persist acceptance before simulating any missing acknowledgement.
                    observation = dict(operation=run.invocation, effect_number=len(owner.events)+1,
                        request_id=event['request_id'], request_hash=expected,
                        signed_sequence=records[-1]['sequence'], body=body.decode(),
                        method='POST', url=run.destination, headers=headers,
                        journal_prefix_sha256=digest(run.journal()), accepted=True)
                    owner.events.append(observation)
                    save(run.evidence/'receiver.json', observation)
                    run.accepted.set()
                    response = json.dumps(dict(accepted=True, order='order-001')).encode()
                    if name == 'acknowledged':
                        self.send_response(201)
                        self.send_header('Content-Length', str(len(response)))
                        self.send_header('Content-Type', 'application/json')
                        self.end_headers()
                        self.wfile.write(response)
                    elif name == 'accepted_partial_response':
                        self.send_response(201)
                        self.send_header('Content-Length', str(len(response)+100))
                        self.end_headers()
                        self.wfile.write(response[:5])
                        self.wfile.flush()
                    elif name in ['accepted_timeout','runtime_loss_after_acceptance']:
                        owner.release.wait(5)
                    if name != 'acknowledged':
                        try:
                            self.connection.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            pass  # The interrupted client may already have closed.
                        self.connection.close()
                except Exception as error:
                    owner.errors.append(repr(error))
                finally:
                    if 'run' in locals():
                        run.receiver_finished.set()

        self.server = ThreadingHTTPServer((address, 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()


def configure(run, receiver, address):
    run.accepted, run.receiver_finished = threading.Event(), threading.Event()
    receiver.runs[run.name] = run
    run.destination = f'http://{address}:{receiver.server.server_port}/orders/{run.name}'
    run.payload = dict(input='order-001', output='receipt-001')
    manifest = '''[tool]
name="record"
version="1"
description="Accept a synthetic order once"
human_approval=true
timeout_seconds=3
[args.input]
type="enum"
required=true
allowed=["order-001"]
[args.output]
type="enum"
required=true
allowed=["receipt-001"]
[http]
method="POST"
url='''+json.dumps(run.destination)+'''
body_template='{"order":"{input}","receipt":"{output}"}'
[http.headers]
content-type="application/json"
x-operation-id='''+json.dumps(run.invocation)+'''
[output]
format="json"
max_bytes=4096
'''
    (run.project/'tools/record.clad.toml').write_text(manifest)


def invoke(run, *arguments):
    return subprocess.run([str(run.lab.binary), *arguments], cwd=run.project, env=run.env,
                          capture_output=True, text=True, timeout=15)


def replay(run, receiver, expected):
    before = len(receiver.events), len(run.requests), digest(run.journal())
    words = run.words[:run.words.index('--approval-terminal')]
    result = invoke(run, *words)
    assert result.returncode == expected, (result.returncode,result.stdout,result.stderr)
    assert before == (len(receiver.events), len(run.requests), digest(run.journal()))
    if expected == 2:
        assert 'reconciliation' in result.stderr.lower() or 'unresolved' in result.stderr.lower()
    elif expected == 3:
        assert json.loads(result.stdout)['status'] == 'reconciled'
    else:
        assert 'Existing invocation; no work repeated.' in result.stderr
    return dict(exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr,
                effects_unchanged=True, inference_repeated=False, journal_unchanged=True)


def reconcile(run):
    before = digest(run.journal())
    inspected = invoke(run, 'invocation','inspect','--scope','cli:orga','--id',run.invocation)
    assert inspected.returncode == 2, inspected.stderr
    snapshot = json.loads(inspected.stdout)
    assert snapshot['snapshot']['audit']['public_key'] == run.public.read_bytes()[-32:].hex()
    save(run.evidence/'inspection.json', snapshot)
    review = dict(snapshot_hash=snapshot['snapshot_hash'], outcome='completed', effects_stopped=True,
                  rationale='The synthetic receiver durably recorded exactly one accepted order; the request and runtime have stopped.',
                  evidence=[dict(reference=str(run.evidence/'receiver.json'),sha256=digest(run.evidence/'receiver.json'))])
    path = run.evidence/'review.json'
    save(path, dict(review, snapshot_hash='sha256:'+'0'*64))
    refused = invoke(run, 'invocation','reconcile','--scope','cli:orga','--id',run.invocation,'--review',str(path))
    assert refused.returncode != 0, 'stale review was accepted'
    save(path, review)
    result = invoke(run, 'invocation','reconcile','--scope','cli:orga','--id',run.invocation,'--review',str(path))
    assert result.returncode == 0, (result.stdout,result.stderr)
    value = json.loads(result.stdout)
    assert value['status'] == 'reconciled' and value['work_repeated'] is False
    assert digest(run.journal()) == before
    save(run.evidence/'reconciliation.json', value)
    receipts = list((run.project/'.symbiont/invocations').glob('*.resolved.json'))
    assert len(receipts) == 1
    shutil.copy2(receipts[0],run.evidence/'signed-resolution.json')
    return dict(stale_review_refused=True, journal_unchanged=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--expected-binary-sha256', required=True)
    parser.add_argument('--service-address', default='198.18.0.1')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not __debug__ or sys.platform != 'linux' or os.geteuid() == 0:
        parser.error('Requires non-root Linux and enabled assertions')
    if ipaddress.ip_address(args.service_address) not in ipaddress.ip_network('198.18.0.0/15'):
        parser.error('Use a benchmarking address owned by the isolated lab loopback')
    binary = args.binary.resolve(strict=True)
    assert digest(binary) == args.expected_binary_sha256
    output = args.output.resolve(); output.mkdir(mode=0o700,parents=True,exist_ok=False)
    report = dict(passed=False,planned=CASES,cases=[],binary_sha256=digest(binary),driver_sha256=digest(__file__),
                  helper_sha256={name:digest(HERE/name) for name in
                    ['test-containment-lifecycle.py','delegated_service_fixture.py','test-direct-inference.py']},
                  limitations=['Synthetic external service and terminal operator; no independent human review.',
                    'Receiver deliberately does not deduplicate. These cases establish no replay under the same invocation ID.',
                    'A new operator invocation is new work; arbitrary-service exactly-once execution is not established.'])
    receiver = lab = None
    try:
        receiver = Receiver(args.service_address)
        lab = Lab(binary, output)
        for name in CASES:
            row = dict(name=name,passed=False);report['cases'].append(row)
            try:
                receiver.release.clear()
                run = Run(lab,name,approval=True)
                configure(run,receiver,args.service_address)
                start_count = len(receiver.events)
                run.start()
                if name == 'runtime_loss_after_acceptance':
                    lab.wait(lambda:run.accepted.is_set())
                    os.killpg(run.process.pid,signal.SIGKILL)
                result = run.finish()
                receiver.release.set()
                assert run.receiver_finished.wait(10) and not receiver.errors, receiver.errors
                assert run.accepted.is_set() and len(receiver.events) == start_count+1
                assert len(run.prompts) == 1 and result['dispatches'] == 1 and not result['workers']
                assert len(run.requests) == (2 if name=='acknowledged' else 1)
                if name=='acknowledged':
                    assert result['exit_code']==0
                    save(run.evidence/'retry.json',replay(run,receiver,0))
                else:
                    assert result['exit_code'] != 0
                    save(run.evidence/'retry.json',replay(run,receiver,2))
                    row.update(reconcile(run))
                    save(run.evidence/'reconciled-retry.json',replay(run,receiver,3))
                lab.empty()
                row.update(passed=True,receiver_effects=1,cleanup_confirmed=True)
            except Exception as error:
                row.update(error=repr(error),traceback=traceback.format_exc())
            save(output/'report.json',report)
            print(name+': '+('pass' if row['passed'] else 'fail'),flush=True)
        report['passed'] = all(row['passed'] for row in report['cases']) and not receiver.errors
    except Exception as error:
        report.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        try:
            if receiver: receiver.close()
            if lab: lab.close()
            report['cleanup_confirmed']=True
        except Exception as error:
            report.update(passed=False,cleanup_error=repr(error))
        report['receiver_events'] = receiver.events if receiver else []
        report['receiver_errors'] = receiver.errors if receiver else []
        report['binary_unchanged']=digest(binary)==args.expected_binary_sha256
        report['passed'] &= report['binary_unchanged']
        save(output/'report.json',report)
        save(output/'manifest.json',dict(files={str(p.relative_to(output)):digest(p)
             for p in sorted(output.rglob('*')) if p.is_file()}))
    return 0 if report['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())

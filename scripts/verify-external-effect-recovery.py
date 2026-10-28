#!/usr/bin/env python3
"""Verify receiver-observed business effects, signed journals and recovery receipts."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile

HERE=Path(__file__).resolve().parent
helpers=runpy.run_path(str(HERE/'test-external-effect-recovery.py'))
CASES, digest, identity = [helpers[name] for name in ['CASES','digest','request_identity']]
VERIFY=runpy.run_path(str(HERE/'test-direct-inference.py'))['verify_journal']


def verify_resolution(path, public):
    wire=path.read_text()
    prefix='{"payload":'
    assert wire.startswith(prefix)
    payload,end=json.JSONDecoder().raw_decode(wire,len(prefix))
    signed=json.loads(wire)
    with tempfile.TemporaryDirectory(prefix='effect-resolution-') as temporary:
        root=Path(temporary)
        (root/'payload').write_bytes(b'symbi-invocation-resolution:v1\n'+wire[len(prefix):end].encode())
        (root/'signature').write_bytes(bytes.fromhex(signed['signature']))
        result=subprocess.run(['openssl','pkeyutl','-verify','-pubin','-keyform','DER','-inkey',str(public),
                               '-rawin','-in',str(root/'payload'),'-sigfile',str(root/'signature')],
                              capture_output=True,timeout=5)
        assert result.returncode==0,'resolution signature is invalid'
    return payload


def verify(root):
    if not __debug__:
        raise RuntimeError('Assertions must be enabled; do not use Python optimization')
    root=Path(root).resolve(strict=True)
    manifest=json.loads((root/'manifest.json').read_text())['files']
    actual=set()
    for path in root.rglob('*'):
        assert not path.is_symlink()
        if path.is_file() and path!=root/'manifest.json':actual.add(str(path.relative_to(root)))
    assert actual==set(manifest),'artifact inventory mismatch'
    for name,expected in manifest.items():
        path=root/name
        assert not Path(name).is_absolute() and path.resolve().is_relative_to(root)
        assert digest(path)==expected,'artifact changed: '+name
    report=json.loads((root/'report.json').read_text())
    assert report['planned']==CASES and [row['name'] for row in report['cases']]==CASES
    assert report['passed'] and report['binary_unchanged'] and report['cleanup_confirmed']
    assert not report['receiver_errors'] and len(report['receiver_events'])==len(CASES)
    assert {p.parent.name for p in root.glob('*/result.json')}==set(CASES)
    for index,case in enumerate(report['cases']):
        name=case['name'];folder=root/name
        assert case['passed'] and case['cleanup_confirmed'] and case['receiver_effects']==1
        row=json.loads((folder/'result.json').read_text())
        assert not row['provider_errors'] and not row['workers'] and row['dispatches']==1
        assert row['public_key']==(folder/'public.der').read_bytes()[-32:].hex()
        assert digest(folder/'audit.jsonl')==row['journal_sha256']
        first=json.loads((folder/'audit.jsonl').read_bytes().splitlines()[0])['payload']
        with tempfile.TemporaryDirectory(prefix='effect-journal-') as temporary:
            records=VERIFY(folder/'audit.jsonl',folder/'public.der',Path(temporary),first['entry']['agent_id'],first['run_id'])
        assert len(records)==row['records'] and records[-1]['event']==row['terminal_event']
        assert json.loads((folder/'provider.json').read_text())==row['requests']
        for request in row['requests']:
            assert 'BudgetReservationStarted' in records[request['signed_pre_request_sequence']]['event']
        observation=json.loads((folder/'receiver.json').read_text())
        assert observation==report['receiver_events'][index] and observation['accepted']
        assert observation['operation']==row['invocation_id'] and observation['effect_number']==index+1
        assert observation['headers']['x-operation-id']==row['invocation_id']
        assert identity(observation['method'],observation['url'],observation['headers'],observation['body'].encode())==observation['request_hash']
        seq=observation['signed_sequence']
        effect=records[seq]['event']['ToolEffect']['effect']['NetworkRequestStarted']
        assert effect['request_id']==observation['request_id'] and effect['request_hash']==observation['request_hash']
        prefix=b''.join((folder/'audit.jsonl').read_bytes().splitlines(keepends=True)[:seq+1])
        assert hashlib.sha256(prefix).hexdigest()==observation['journal_prefix_sha256']
        calls=[call for entry in records for call in entry['event'].get('PolicyEvaluated',{}).get('approved_calls',[]) if call.get('approval')]
        assert len(calls)==len(row['prompts'])==1
        call,held=calls[0],row['prompts'][0]
        assert call['fingerprint']==held['context_snapshot']['invocation']['fingerprint']==call['approval']['fingerprint']
        assert call['arguments']==held['context_snapshot']['invocation']['arguments']
        assert call['resolved']['http_url']==observation['url'] and call['resolved']['http_method']=='POST'
        assert held['agent_id']==first['entry']['agent_id']==call['approval']['resolution']['agent_id']
        assert call['approval']['resolution']['escalation_id']==held['id']
        assert call['approval']['resolution']['decision']['decision']=='approve'
        assert row['answers']==['approve '+held['id']]
        retry=json.loads((folder/'retry.json').read_text())
        assert retry['effects_unchanged'] and retry['journal_unchanged'] and not retry['inference_repeated']
        if name=='acknowledged':
            assert row['exit_code']==retry['exit_code']==0 and len(row['requests'])==2
            assert 'Terminated' in records[-1]['event']
        else:
            assert row['exit_code']!=0 and retry['exit_code']==2 and len(row['requests'])==1
            assert case['stale_review_refused'] and case['journal_unchanged']
            payload=verify_resolution(folder/'signed-resolution.json',folder/'public.der')
            review=json.loads((folder/'review.json').read_text())
            inspection=json.loads((folder/'inspection.json').read_text())
            assert payload['review']==review and payload['snapshot']==inspection['snapshot']
            assert review['snapshot_hash']==inspection['snapshot_hash'] and review['outcome']=='completed'
            assert review['effects_stopped'] and review['evidence'][0]['sha256']==digest(folder/'receiver.json')
            reconciled=json.loads((folder/'reconciled-retry.json').read_text())
            assert reconciled['exit_code']==3 and reconciled['effects_unchanged'] and not reconciled['inference_repeated']
            assert json.loads(reconciled['stdout'])['resolution']==payload
    return dict(passed=True,cases=len(CASES),complete_suite=True,receiver_effects=len(CASES),
                manifest_sha256=digest(root/'manifest.json'),binary_sha256=report['binary_sha256'])


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence',type=Path)
    print(json.dumps(verify(parser.parse_args().evidence),indent=2))

#!/usr/bin/env python3
"""Independently verify lifecycle evidence; no runtime or worker is launched.

Pin the manifest hash outside the bundle. The host collector is trusted; these
checks do not authenticate a bundle against an operator replacing every artifact.
"""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import tempfile

HERE = Path(__file__).resolve().parent
helpers = runpy.run_path(str(HERE/'test-containment-lifecycle.py'))
CASES, digest, journal_verify = helpers['CASES'], helpers['digest'], helpers['VERIFY']


def verify(directory):
    if not __debug__:
        raise RuntimeError('Assertions must be enabled; do not use Python optimization')
    root = Path(directory).resolve(strict=True)
    manifest = json.loads((root/'manifest.json').read_text())['files']
    actual = set()
    for path in root.rglob('*'):
        assert not path.is_symlink(), 'linked evidence is unavailable'
        if path.is_file() and path != root/'manifest.json':
            actual.add(str(path.relative_to(root)))
    assert actual == set(manifest), 'evidence inventory mismatch'
    for name, expected in manifest.items():
        assert not Path(name).is_absolute() and (root/name).resolve().is_relative_to(root)
        assert digest(root/name) == expected, 'artifact changed: ' + name
    report = json.loads((root/'report.json').read_text())
    planned = report['planned']
    assert planned and len(planned) == len(set(planned)) and set(planned) <= set(CASES)
    assert [case['name'] for case in report['cases']] == planned, 'missing or repeated cases'
    assert report['passed'] and report['binary_unchanged'] and report['cleanup_confirmed']
    assert all(case['passed'] and case['cleanup_confirmed'] for case in report['cases'])
    expected_runs = set(planned) - {'concurrent_isolation_and_capacity', 'cross_run_approval_replay'}
    if 'concurrent_isolation_and_capacity' in planned:
        expected_runs.update(['left','right','capacity-denied','capacity-recovered'])
    if 'cross_run_approval_replay' in planned:
        expected_runs.update(['approval-owner','approval-other'])
    if 'supervisor_loss_during_action' in planned:
        expected_runs.add('supervisor-recovered')
    folders = {path.parent.name for path in root.glob('*/result.json')}
    assert folders == expected_runs, 'run inventory mismatch'
    rows = {}
    count = 0
    for name in sorted(expected_runs):
        folder = root/name
        row = json.loads((folder/'result.json').read_text())
        rows[name] = row
        assert row['name'] == name and not row['provider_errors']
        assert json.loads((folder/'provider.json').read_text()) == row['requests']
        assert (folder/'public.der').read_bytes()[-32:].hex() == row['public_key']
        journal = folder/'audit.jsonl'
        assert digest(journal) == row['journal_sha256']
        first = json.loads(journal.read_bytes().splitlines()[0])['payload']
        with tempfile.TemporaryDirectory(prefix='lifecycle-verifier-') as temporary:
            entries = journal_verify(journal, folder/'public.der', Path(temporary),
                                     first['entry']['agent_id'], first['run_id'])
        assert len(entries) == row['records'] and entries[-1]['event'] == row['terminal_event']
        assert sum('ToolDispatchStarted' in e['event'] for e in entries) == row['dispatches']
        if not name.startswith('audit_failure_'):
            assert 'Terminated' in entries[-1]['event'], 'signed run is incomplete'
        for request in row['requests']:
            event = entries[request['signed_pre_request_sequence']]['event']
            assert 'BudgetReservationStarted' in event, 'provider observation lacks its signed reservation'
        count += len(row['requests'])
        for worker in row['workers']:
            assert worker['origin']['run_id'] == first['run_id']
            assert worker['origin']['agent_id'] == first['entry']['agent_id']
        approved = [call for entry in entries for call in entry['event'].get('PolicyEvaluated', {}).get('approved_calls', [])
                    if call.get('approval')]
        for call in approved:
            receipt = call['approval']
            held = next(h for h in row['prompts'] if h['id'] == receipt['resolution']['escalation_id'])
            proposed = held['context_snapshot']['invocation']
            assert call['fingerprint'] == receipt['fingerprint'] == proposed['fingerprint']
            assert call['arguments'] == proposed['arguments']
            assert held['agent_id'] == receipt['resolution']['agent_id'] == first['entry']['agent_id']
            assert receipt['resolution']['decision']['decision'] == 'approve'
            assert receipt['resolution']['approver']['surface'] == 'terminal'
            assert 'approve ' + held['id'] in row['answers']
        effects = [entry['event']['ToolEffect']['effect'] for entry in entries if 'ToolEffect' in entry['event']]
        intents = {effect['FilePublicationPrepared']['intent']['publication_id']:effect['FilePublicationPrepared']['intent']
                   for effect in effects if 'FilePublicationPrepared' in effect}
        published = {intents[effect['FilePublicationFinished']['publication_id']]['path']:
                     intents[effect['FilePublicationFinished']['publication_id']]
                     for effect in effects if effect.get('FilePublicationFinished', {}).get('confirmed')}
        assert set(published) == set(row['effects']), 'reported files differ from signed publication results'
        for filename, value in row['effects'].items():
            assert filename == 'first.json'
            assert json.loads((folder/filename).read_text()) == value
            assert digest(folder/filename) == published[filename]['sha256'], 'published bytes were substituted'
            assert value['sum'] == (31 if name == 'right' else 10) and value['variant'] == 'original'
            assert value['channel_denied'] and value['signal_denied'] and value['inherited_secret'] is None
            assert all(value['denied'].values())
        if name in ['manifest_substitution','input_substitution','output_substitution','argument_approval_replay','approval-owner']:
            assert len(approved) == 1
            boundary = approved[0]['resolved']['command_boundary']
            assert boundary['filesystem']['input_snapshots'] and not boundary['filesystem']['overwrite']
            assert boundary['filesystem']['read'][0]['sha256'] == hashlib.sha256(b'2 3 5').hexdigest()
        if name in ['capacity-denied','approval-other','audit_failure_before_dispatch','audit_failure_during_action',
                    'supervisor_loss_during_action','output_substitution']:
            assert not row['effects']
        else:
            assert set(row['effects']) == {'first.json'} and row['workers']
        if name in ['approval-other','audit_failure_before_dispatch']:
            assert row['dispatches'] == 0
        if name in ['audit_failure_during_action','supervisor_loss_during_action','output_substitution']:
            assert row['workers'] and row['dispatches'] == 1
        if name == 'output_substitution':
            assert row['exit_code'] != 0
        if name.startswith('audit_failure_') or name in ['supervisor_loss_during_action','provider_disconnect_after_publication']:
            assert row['exit_code'] != 0
            retry = json.loads((folder/'retry.json').read_text())
            assert retry['exit_code'] != 0 and retry['effects_unchanged'] and not retry['inference_repeated']
            assert retry['unresolved'] == (name != 'provider_disconnect_after_publication')
        elif name not in ['capacity-denied','output_substitution']:
            assert row['exit_code'] == 0
        if name.startswith('audit_failure_'):
            assert row['fault']['kind'] == 'RLIMIT_FSIZE'
            if name in ['audit_failure_before_dispatch','audit_failure_during_action']:
                assert len(row['requests']) == 1
            else:
                assert len(row['requests']) == 2
        if name == 'argument_approval_replay':
            assert len(row['prompts']) == 2 and row['dispatches'] == 1
            assert row['answers'] == ['approve ' + row['prompts'][0]['id']]*2
            assert row['prompts'][1]['context_snapshot']['invocation']['arguments']['output'] == 'second.json'
    if 'concurrent_isolation_and_capacity' in planned:
        case = next(c for c in report['cases'] if c['name'] == 'concurrent_isolation_and_capacity')
        assert case['busy']['reserved'] == dict(workers=2,memory_bytes=1024**3,cpu_nanos=2_000_000_000)
        assert case['busy']['admission_blocked']
        assert len(case['protected_before']) == 6 and case['protected_before'] == case['protected_after']
        assert rows['left']['workers'][0]['mount_namespace'] != rows['right']['workers'][0]['mount_namespace']
        for name in ['left','right']:
            assert len(rows[name]['effects']['first.json']['denied']) == 6
        assert not rows['capacity-denied']['workers']
    if 'cross_run_approval_replay' in planned:
        owner, other = rows['approval-owner'], rows['approval-other']
        assert owner['prompts'][0]['id'] != other['prompts'][0]['id']
        assert owner['answers'] == other['answers'] == ['approve ' + owner['prompts'][0]['id']]
    return dict(passed=True, cases=len(planned), runs=len(rows), provider_requests=count,
                complete_suite=planned == CASES, manifest_sha256=digest(root/'manifest.json'),
                binary_sha256=report['binary_sha256'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    print(json.dumps(verify(parser.parse_args().evidence), indent=2))

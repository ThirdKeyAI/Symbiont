#!/usr/bin/env python3
"""Verify a complete local evidence bundle without starting workers or inference.

The manifest must be pinned externally for authenticity. A host operator able to
replace the bundle and its manifest is outside this verifier's trust boundary.
"""
import argparse
import json
from pathlib import Path
import runpy

HERE = Path(__file__).resolve().parent
driver = runpy.run_path(str(HERE/'test-containment-evidence.py'))
digest, score, CASES = driver['digest'], driver['score'], driver['CASES']
verify_journal = runpy.run_path(str(HERE/'test-direct-inference.py'))['verify_journal']


def verify(path):
    if not __debug__:
        raise RuntimeError('Assertions must be enabled; do not use Python optimization')
    root = Path(path).resolve(strict=True)
    manifest = json.loads((root/'manifest.json').read_text())
    assert manifest['schema_version'] == 1
    assert 'report.json' in manifest['files']
    actual = set()
    for item in root.rglob('*'):
        assert not item.is_symlink(), 'linked evidence is unavailable'
        if item.is_file() and item != root/'manifest.json':
            actual.add(str(item.relative_to(root)))
    assert actual == set(manifest['files']), 'evidence inventory mismatch'
    for name, expected in manifest['files'].items():
        item = root/name
        assert item.resolve().is_relative_to(root) and not Path(name).is_absolute()
        assert digest(item) == expected, 'changed artifact: ' + name
    report = json.loads((root/'report.json').read_text())
    planned = report['planned_cases']
    assert planned and len(planned) == len(set(planned)) and set(planned) <= set(CASES)
    assert [row['name'] for row in report['cases']] == planned, 'missing, repeated or reordered trials'
    assert report['binary_unchanged'] and report['service_cleanup_confirmed']
    for row in report['cases']:
        folder = root/row['name']
        assert json.loads((folder/'result.json').read_text()) == row
        failures = score(row)
        assert not failures and row['passed'] and row['status'] == 'pass', failures
        events = [json.loads(line) for line in (folder/'provider.jsonl').read_text().splitlines()]
        assert events == row['provider_events']
        assert [e['phase'] for e in events] == row['provider_phases']
        assert len({e['request_id'] for e in events}) == len(events)
        public = folder/'public.der'
        assert public.read_bytes()[-32:].hex() == row['pinned_public_key']
        journal = folder/'audit.jsonl'
        if row['name'] == 'audit_storage_failure':
            assert row['fault_injected'] and b'controlled-storage-fault\n' in journal.read_bytes()
            # Verify the signed prefix separately; the injected invalid suffix
            # must not be mistaken for complete, valid evidence.
            prefix = journal.read_bytes().split(b'controlled-storage-fault\n', 1)[0]
        else:
            prefix = journal.read_bytes()
        import tempfile
        with tempfile.TemporaryDirectory(prefix='evidence-verifier-') as temporary:
            temporary = Path(temporary)
            snapshot = temporary/'audit.jsonl'
            snapshot.write_bytes(prefix)
            first = json.loads(prefix.splitlines()[0])['payload']
            entries = verify_journal(snapshot, public, temporary, first['entry']['agent_id'], legacy=True)
        if row['name'] != 'audit_storage_failure':
            assert 'Terminated' in entries[-1]['event'], 'signed run is incomplete'
        requests = {e['event']['InferenceRequested']['request_id']:e
                    for e in entries if 'InferenceRequested' in e['event']}
        assert set(requests) == {e['request_id'] for e in events}
        for event in events:
            record = requests[event['request_id']]
            assert record['sequence'] == event['sequence']
            assert record['event']['InferenceRequested']['request_hash'] == event['request_sha256']
    assert report['passed'] and report['status'] == 'pass'
    return dict(passed=True, cases=len(planned), complete_suite=planned == CASES,
                runtime_source=report['runtime_source'], binary_sha256=report['binary_sha256'],
                manifest_sha256=digest(root/'manifest.json'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.evidence), indent=2))

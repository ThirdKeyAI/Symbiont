#!/usr/bin/env python3
"""Verify the bounded adaptive pilot and each independently checked round."""
import argparse
import json
from pathlib import Path
import runpy

HERE = Path(__file__).resolve().parent
base = runpy.run_path(str(HERE/'test-adaptive-containment-pilot.py'))
verify_round = runpy.run_path(str(HERE/'verify-containment-evidence.py'))['verify']
digest = base['digest']


def verify(root):
    if not __debug__:
        raise RuntimeError('Assertions must be enabled; do not use Python optimization')
    root = Path(root).resolve(strict=True)
    inventory = json.loads((root/'manifest.json').read_text())
    assert inventory['schema_version'] == 1
    actual = {str(path.relative_to(root)) for path in root.rglob('*')
              if path.is_file() and path != root/'manifest.json'}
    assert actual == set(inventory['files'])
    for name, expected in inventory['files'].items():
        path = root/name
        assert not Path(name).is_absolute() and path.resolve().is_relative_to(root)
        assert not path.is_symlink() and digest(path) == expected
    report = json.loads((root/'report.json').read_text())
    plan = json.loads((root/'plan.json').read_text())
    assert report['plan_sha256'] == digest(root/'plan.json')
    assert plan['attempt_limit'] == report['attempt_limit'] == report['attempts'] == 12
    assert report['passed'] and not report['stopped'] and report['binary_unchanged']
    assert len(plan['rounds']) == len(report['rounds']) == 3
    all_cases = []
    for index, (planned, row) in enumerate(zip(plan['rounds'], report['rounds']), 1):
        assert row['name'] == planned['name'] and row['cases'] == planned['cases']
        assert row['condition'] == planned['condition'] and row['passed'] and row['exit_code'] == 0
        checked = verify_round(root/f'round-{index}')
        assert checked == row['verification'] and checked['cases'] == len(planned['cases']) == 4
        assert not checked['complete_suite'] and checked['binary_sha256'] == report['binary_sha256']
        expected_decision = ('proceed_after_complete_safe_observations' if index < 3 else 'complete')
        assert row['decision'] == expected_decision
        assert [item['name'] for item in row['observations']] == planned['cases']
        assert all(item['passed'] and item['status']=='pass' for item in row['observations'])
        all_cases += planned['cases']
    assert len(all_cases) == len(set(all_cases)) == 12
    return dict(passed=True, attempts=12, rounds=3, binary_sha256=report['binary_sha256'],
                plan_sha256=report['plan_sha256'], manifest_sha256=digest(root/'manifest.json'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    print(json.dumps(verify(parser.parse_args().evidence), indent=2))

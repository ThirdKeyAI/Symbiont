#!/usr/bin/env python3
"""Run the bounded independently authored containment plan in adaptive rounds."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--expected-binary-sha256', required=True)
    parser.add_argument('--runtime-source', required=True)
    parser.add_argument('--upstream-source')
    parser.add_argument('--plan', type=Path, default=HERE/'adaptive-containment-plan.json')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not __debug__ or platform.system() != 'Linux' or os.geteuid() == 0:
        parser.error('Requires non-root Linux and enabled assertions')
    binary = args.binary.resolve(strict=True)
    if digest(binary) != args.expected_binary_sha256:
        parser.error('Binary checksum differs from the frozen target')
    plan_path = args.plan.resolve(strict=True)
    plan = json.loads(plan_path.read_text())
    rounds = plan['rounds']
    cases = [case for round_ in rounds for case in round_['cases']]
    assert plan['attempt_limit'] == 12 and len(cases) == len(set(cases)) == 12
    output = args.output.resolve()
    if output.is_relative_to('/tmp'):
        parser.error('Output must be outside /tmp for native executable fixtures')
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    (output/'plan.json').write_bytes(plan_path.read_bytes())
    report = dict(schema_version=1, kind='bounded_adaptive_containment_pilot', passed=False,
                  binary_sha256=digest(binary), runtime_source=args.runtime_source,
                  upstream_source=args.upstream_source, plan_sha256=digest(plan_path),
                  attempt_limit=plan['attempt_limit'], attempts=0, rounds=[],
                  stopped=False, binary_unchanged=False,
                  limitations=plan['limitations'])
    save(output/'report.json', report)
    for index, round_ in enumerate(rounds, 1):
        folder = output/f'round-{index}'
        command = [sys.executable, str(HERE/'test-containment-evidence.py'),
                   '--binary', str(binary), '--expected-binary-sha256', args.expected_binary_sha256,
                   '--runtime-source', args.runtime_source, '--output', str(folder)]
        if args.upstream_source:
            command += ['--upstream-source', args.upstream_source]
        for case in round_['cases']:
            command += ['--case', case]
        started = time.monotonic()
        result = subprocess.run(command, capture_output=True, text=True, timeout=600)
        (output/f'round-{index}.log').write_text(result.stdout+result.stderr)
        row = dict(name=round_['name'], condition=round_['condition'],
                   cases=round_['cases'], exit_code=result.returncode,
                   seconds=round(time.monotonic()-started, 3), passed=False)
        report['rounds'].append(row)
        if (folder/'report.json').is_file():
            detail = json.loads((folder/'report.json').read_text())
            report['attempts'] += len(detail.get('cases', []))
            row['observations'] = [{key: case.get(key) for key in
                ['name','status','passed','provider_phases','sink_events','cleanup_confirmed']}
                for case in detail.get('cases', [])]
        verification = subprocess.run(
            [sys.executable, str(HERE/'verify-containment-evidence.py'), str(folder)],
            capture_output=True, text=True, timeout=60)
        (output/f'round-{index}-verification.json').write_text(
            verification.stdout if verification.stdout else json.dumps({'stderr':verification.stderr})+'\n')
        if verification.returncode == 0:
            checked = json.loads(verification.stdout)
            row['verification'] = checked
            row['passed'] = (result.returncode == 0 and checked['passed'] and
                             checked['cases'] == len(round_['cases']) and
                             checked['binary_sha256'] == report['binary_sha256'])
        row['decision'] = ('proceed_after_complete_safe_observations' if row['passed'] and index < len(rounds)
                           else 'complete' if row['passed'] else 'stop_on_adverse_or_incomplete_observation')
        save(output/'report.json', report)
        if not row['passed']:
            report['stopped'] = True
            break
    report['binary_unchanged'] = digest(binary) == report['binary_sha256']
    report['passed'] = (not report['stopped'] and len(report['rounds']) == len(rounds)
                        and report['attempts'] == plan['attempt_limit']
                        and all(row['passed'] for row in report['rounds'])
                        and report['binary_unchanged'])
    save(output/'report.json', report)
    files = {str(path.relative_to(output)):digest(path) for path in sorted(output.rglob('*'))
             if path.is_file() and path != output/'manifest.json'}
    save(output/'manifest.json', dict(schema_version=1, files=files))
    print(json.dumps(dict(passed=report['passed'], attempts=report['attempts'],
                          rounds=len(report['rounds']), stopped=report['stopped']), indent=2))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

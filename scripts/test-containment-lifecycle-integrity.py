#!/usr/bin/env python3
"""Ensure rehashed artifact changes cannot counterfeit passing lifecycle evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
verify = runpy.run_path(str(HERE/'verify-containment-lifecycle.py'))['verify']


def save(path, value):
    path.write_text(json.dumps(value, indent=2)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    args = parser.parse_args()
    if not __debug__:
        raise RuntimeError('Assertions must be enabled; do not use Python optimization')
    original = args.evidence.resolve(strict=True)
    baseline = verify(original)
    assert baseline['complete_suite'], 'Integrity challenges require the complete suite'
    rejected = []
    for attack in ['missing_case','substituted_effect','substituted_approval','incomplete_journal']:
        with tempfile.TemporaryDirectory(prefix='lifecycle-integrity-') as temporary:
            root = Path(temporary)/'copy'
            shutil.copytree(original, root)
            report = json.loads((root/'report.json').read_text())
            folder = root/'manifest_substitution'
            row = json.loads((folder/'result.json').read_text())
            if attack == 'missing_case':
                report['cases'].pop()
                save(root/'report.json', report)
            elif attack == 'substituted_effect':
                value = json.loads((folder/'first.json').read_text())
                value['extra'] = 'unapproved data'
                save(folder/'first.json', value)
                row['effects']['first.json'] = value
                save(folder/'result.json', row)
            elif attack == 'substituted_approval':
                row['prompts'][0]['context_snapshot']['invocation']['fingerprint'] = 'sha256:'+'0'*64
                save(folder/'result.json', row)
            else:
                path = folder/'audit.jsonl'
                lines = path.read_bytes().splitlines(keepends=True)
                assert 'Terminated' in json.loads(lines[-1])['payload']['entry']['event']
                path.write_bytes(b''.join(lines[:-1]))
                row.update(records=len(lines)-1, journal_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                           terminal_event=json.loads(lines[-2])['payload']['entry']['event'])
                save(folder/'result.json', row)
            manifest = json.loads((root/'manifest.json').read_text())
            for name in manifest['files']:
                manifest['files'][name] = hashlib.sha256((root/name).read_bytes()).hexdigest()
            save(root/'manifest.json', manifest)
            try:
                verify(root)
            except (AssertionError, ValueError, KeyError, StopIteration):
                rejected.append(attack)
            else:
                raise AssertionError('Verifier accepted changed evidence: '+attack)
    optimized = subprocess.run([sys.executable,'-O',str(HERE/'verify-containment-lifecycle.py'),str(original)],
                               capture_output=True,text=True,timeout=10)
    assert optimized.returncode != 0 and 'Assertions must be enabled' in optimized.stderr
    print(json.dumps(dict(passed=True, original_cases=baseline['cases'], rejected=rejected,
                          optimization_refused=True),indent=2))


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Reject rehashed substitutions of external-effect and reconciliation evidence."""
import argparse
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
helpers = runpy.run_path(str(HERE/'verify-external-effect-recovery.py'))
verify, digest = helpers['verify'], helpers['digest']


def save(path, value):
    path.write_text(json.dumps(value, indent=2)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    original = parser.parse_args().evidence.resolve(strict=True)
    baseline = verify(original)
    rejected = []
    for attack in ['missing_case', 'receiver_substitution', 'approval_substitution',
                   'resolution_substitution', 'resolution_signature']:
        with tempfile.TemporaryDirectory(prefix='external-effect-integrity-') as temporary:
            root = Path(temporary)/'copy'
            shutil.copytree(original, root)
            folder = root/'accepted_disconnect'
            report = json.loads((root/'report.json').read_text())
            if attack == 'missing_case':
                report['cases'].pop()
                save(root/'report.json', report)
            elif attack == 'receiver_substitution':
                observation = json.loads((folder/'receiver.json').read_text())
                observation['body'] = '{"order":"different-order","receipt":"receipt-001"}'
                observation['request_hash'] = helpers['identity'](
                    observation['method'], observation['url'], observation['headers'], observation['body'].encode())
                report['receiver_events'][1] = observation
                save(root/'report.json', report)
                save(folder/'receiver.json', observation)
            elif attack == 'approval_substitution':
                row = json.loads((folder/'result.json').read_text())
                row['prompts'][0]['context_snapshot']['invocation']['fingerprint'] = 'sha256:'+'0'*64
                save(folder/'result.json', row)
            else:
                path = folder/'signed-resolution.json'
                signed = json.loads(path.read_text())
                if attack == 'resolution_substitution':
                    signed['payload']['review']['outcome'] = 'not_completed'
                else:
                    signed['signature'] = '00'*64
                path.write_text(json.dumps(signed, separators=(',', ':')))
            inventory = json.loads((root/'manifest.json').read_text())
            for name in inventory['files']:
                inventory['files'][name] = digest(root/name)
            save(root/'manifest.json', inventory)
            try:
                verify(root)
            except (AssertionError, ValueError, KeyError):
                rejected.append(attack)
            else:
                raise AssertionError('Changed evidence was accepted: '+attack)
    result = subprocess.run([sys.executable, '-O', str(HERE/'verify-external-effect-recovery.py'), str(original)],
                            capture_output=True, text=True, timeout=10)
    assert result.returncode != 0 and 'Assertions must be enabled' in result.stderr
    print(json.dumps(dict(passed=True, original_cases=baseline['cases'], rejected=rejected,
                          optimization_refused=True), indent=2))


if __name__ == '__main__':
    main()

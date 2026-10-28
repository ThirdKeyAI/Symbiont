#!/usr/bin/env python3
"""Challenge the offline verifier with corrupted copies of real signed evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import tempfile

verify = runpy.run_path(str(Path(__file__).with_name('verify-containment-evidence.py')))['verify']


def rewrite(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    args = parser.parse_args()
    if not __debug__:
        raise RuntimeError('Assertions must be enabled; do not use Python optimization')
    original = args.evidence.resolve(strict=True)
    baseline = verify(original)
    rejected = []
    for attack in ['changed_signature', 'forged_receiver_hash', 'missing_trial', 'false_success', 'incomplete_journal']:
        with tempfile.TemporaryDirectory(prefix='containment-integrity-') as temporary:
            root = Path(temporary)/'copy'
            shutil.copytree(original, root)
            report = json.loads((root/'report.json').read_text())
            first = report['cases'][0]
            folder = root/first['name']
            if attack == 'changed_signature':
                path = folder/'audit.jsonl'
                lines = path.read_text().splitlines(keepends=True)
                record = json.loads(lines[0])
                signature = record['signature']
                replacement = ('A' if signature[0] != 'A' else 'B') + signature[1:]
                lines[0] = lines[0].replace(signature, replacement)
                path.write_text(''.join(lines))
            elif attack == 'forged_receiver_hash':
                events = [json.loads(line) for line in (folder/'provider.jsonl').read_text().splitlines()]
                events[0]['request_sha256'] = '0'*64
                (folder/'provider.jsonl').write_text(''.join(json.dumps(e)+'\n' for e in events))
                first['provider_events'] = events
                rewrite(folder/'result.json', first)
                rewrite(root/'report.json', report)
            elif attack == 'missing_trial':
                report['cases'] = []
                rewrite(root/'report.json', report)
            elif attack == 'false_success':
                first.update(passed=True, status='pass', sink_events=[dict(protocol='http')])
                rewrite(folder/'result.json', first)
                rewrite(root/'report.json', report)
            else:
                path = folder/'audit.jsonl'
                lines = path.read_bytes().splitlines(keepends=True)
                assert 'Terminated' in json.loads(lines[-1])['payload']['entry']['event']
                path.write_bytes(b''.join(lines[:-1]))
            # Rehash to test semantic/signature verification, rather than only
            # demonstrate that a checksum changes when a file changes.
            manifest = json.loads((root/'manifest.json').read_text())
            for name in manifest['files']:
                manifest['files'][name] = hashlib.sha256((root/name).read_bytes()).hexdigest()
            rewrite(root/'manifest.json', manifest)
            try:
                verify(root)
            except (AssertionError, ValueError, KeyError):
                rejected.append(attack)
            else:
                raise AssertionError('Verifier accepted corrupted evidence: ' + attack)
    print(json.dumps(dict(passed=True, original_cases=baseline['cases'], rejected=rejected), indent=2))


if __name__ == '__main__':
    main()

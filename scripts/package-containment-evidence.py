#!/usr/bin/env python3
"""Package verified public containment evidence for one exact release binary."""
import argparse
import json
from pathlib import Path
import runpy
import tarfile

HERE=Path(__file__).resolve().parent
release=runpy.run_path(str(HERE/'test-landlock-release.py'))


def package(directory, binary, source, output):
    if not __debug__:
        raise RuntimeError('Assertions must be enabled')
    root=directory.resolve(strict=True)
    report=json.loads((root/'qualification.json').read_text())
    assert report['passed'] and not report['dirty'] and report['binary_unchanged']
    assert report['profile']=='containment-evidence' and report['sha']==source
    assert release['digest'](binary)==report['binary_sha256']
    suites=release['EVIDENCE_SUITES']
    assert {row['name'] for row in report['cases']}==set(suites) and len(report['cases'])==len(suites)
    selected=[root/'qualification.json']
    for name,(_,verifier,_,expected) in suites.items():
        row=next(row for row in report['cases'] if row['name']==name)
        assert row['passed'] and row['exit_code']==row['verification_exit_code']==row['integrity_exit_code']==0
        assert release['evidence_result'](root/name,root/(name+'-verification.json'),report['binary_sha256'],expected)
        checked=runpy.run_path(str(HERE/verifier))['verify'](root/name)
        assert checked['passed'] and checked['complete_suite']
        selected.extend(p for p in (root/name).rglob('*') if p.is_file())
        selected.extend(root/(name+suffix) for suffix in ['.log','-verification.json','-integrity.json'])
    for path in selected:
        assert path.is_file() and not path.is_symlink() and path.resolve().is_relative_to(root)
        assert path.suffix not in ['.key','.pem','.p12','.pfx'], 'private material is unavailable for publication'
    with output.open('xb') as stream, tarfile.open(fileobj=stream,mode='w:gz') as archive:
        for path in sorted(selected):
            archive.add(path,arcname='containment/'+str(path.relative_to(root)),recursive=False)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory',type=Path,required=True)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--source',required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    package(args.directory,args.binary,args.source,args.output)

#!/usr/bin/env python3
"""Run the shipping Linux release checks with synthetic providers and real isolation.

Missing kernel, namespace or delegated-service support is a failure, never a
skip. The output directory must be new; private action fixtures contain signing
keys. Publish only the top-level reports and logs, never private-actions/.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--require-clean', action='store_true')
    args = parser.parse_args()
    if not __debug__ or platform.system() != 'Linux' or os.geteuid() == 0:
        parser.error('Requires Linux, a non-root user, and Python assertions enabled')
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    if output.is_relative_to('/tmp'):
        parser.error('Output must be outside /tmp for managed executable grant fixtures')
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=HERE, text=True).strip()
    dirty = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=HERE, text=True))
    if args.require_clean and dirty:
        parser.error('Release qualification requires a clean source tree')
    report = dict(sha=revision, dirty=dirty, binary=str(binary), binary_sha256=digest(binary),
                  kernel=platform.release(), machine=platform.machine(), uid=os.geteuid(),
                  caller_cgroup=Path('/proc/self/cgroup').read_text().strip(),
                  systemd=subprocess.check_output(['systemctl', '--version'], text=True).splitlines()[0],
                  driver_sha256=digest(Path(__file__)), cases=[], passed=False,
                  limitations=['Synthetic providers and terminal operator; no model-quality or human-review claim.',
                               'This host and binary only; not macOS, Windows or arbitrary third-party CLIs.'])
    cases = [
        ('onboarding', 'test-landlock-onboarding.py', ['--development', '--install-dir', str(output/'installation')]),
        ('files', 'test-filesystem-grants.py', ['--landlock']),
        ('source', 'test-source-broker.py', ['--landlock']),
        ('git', 'test-git-source.py', ['--landlock', '--case', 'log', '--case', 'diff',
                                     '--case', 'snapshot', '--case', 'capacity', '--case', 'crash']),
        ('managed', 'test-landlock-managed.py', []),
        ('actions', 'test-intake-actions.py', ['--synthetic-provider', '--output', str(output/'private-actions')]),
    ]
    def checkpoint():
        (output/'qualification.json').write_text(json.dumps(report, indent=2) + '\n')
    checkpoint()
    for name, driver, extra in cases:
        target = output/(name + '.json')
        command = [sys.executable, str(HERE/driver), '--binary', str(binary), *extra]
        if name != 'actions':
            command += ['--report', str(target)]
        row = dict(name=name, driver=driver, driver_sha256=digest(HERE/driver), passed=False)
        report['cases'].append(row)
        started = time.monotonic()
        with (output/(name + '.log')).open('w') as log:
            child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                row['exit_code'] = child.wait(timeout=600)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=10)
                row.update(exit_code=child.returncode, error='driver exceeded ten minutes')
        if name == 'actions' and (output/'private-actions/report.json').exists():
            shutil.copyfile(output/'private-actions/report.json', target)
        if target.exists():
            detail = json.loads(target.read_text())
            row['report_sha256'] = digest(target)
            row['passed'] = row['exit_code'] == 0 and (
                detail.get('passed') is True or detail.get('success') is True or detail.get('status') == 'completed')
        row['seconds'] = round(time.monotonic() - started, 3)
        checkpoint()
        print(f'{name}: {"passed" if row["passed"] else "FAILED"} ({row["seconds"]}s)', flush=True)
    report['binary_unchanged'] = digest(binary) == report['binary_sha256']
    report['passed'] = all(row['passed'] for row in report['cases']) and report['binary_unchanged']
    checkpoint()
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

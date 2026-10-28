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
import re
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
EVIDENCE_SUITES = {
    'broker': ('test-containment-evidence.py', 'verify-containment-evidence.py',
               'test-containment-evidence-integrity.py', 19),
    'lifecycle': ('test-containment-lifecycle.py', 'verify-containment-lifecycle.py',
                  'test-containment-lifecycle-integrity.py', 11),
    'external-effects': ('test-external-effect-recovery.py', 'verify-external-effect-recovery.py',
                         'test-external-effect-integrity.py', 5),
}


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def execute(command, log, timeout=600):
    with log.open('w') as stream:
        child = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT,
                                 start_new_session=True)
        try:
            return child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=10)
            stream.write('\nRelease check exceeded its deadline.\n')
            return 124


def evidence_result(directory, verification, binary_hash, expected_cases):
    """Require complete independently checked evidence for the selected binary."""
    if not directory.is_dir() or not verification.is_file():
        return False
    result = json.loads(verification.read_text())
    return (result.get('passed') is True and result.get('complete_suite') is True
            and result.get('cases') == expected_cases
            and result.get('binary_sha256') == binary_hash
            and result.get('manifest_sha256') == digest(directory/'manifest.json'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--require-clean', action='store_true')
    parser.add_argument('--source-directory', type=Path, default=HERE.parent,
                        help='Checkout used to build the binary; defaults to the driver checkout')
    parser.add_argument('--evidence-only', action='store_true',
                        help='Qualify the containment suites for the exact publication binary')
    parser.add_argument('--upstream-source', help='Known upstream commit, distinct from the public mirror commit')
    args = parser.parse_args()
    if not __debug__ or platform.system() != 'Linux' or os.geteuid() == 0:
        parser.error('Requires Linux, a non-root user, and Python assertions enabled')
    if args.upstream_source and not re.fullmatch('[0-9a-f]{40}', args.upstream_source):
        parser.error('--upstream-source must be a full commit hash')
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    if output.is_relative_to('/tmp'):
        parser.error('Output must be outside /tmp for managed executable grant fixtures')
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    source = args.source_directory.resolve(strict=True)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip()
    dirty = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=source, text=True))
    if args.require_clean and dirty:
        parser.error('Release qualification requires a clean source tree')
    report = dict(sha=revision, dirty=dirty, binary=str(binary), binary_sha256=digest(binary),
                  kernel=platform.release(), machine=platform.machine(), uid=os.geteuid(),
                  caller_cgroup=Path('/proc/self/cgroup').read_text().strip(),
                  systemd=subprocess.check_output(['systemctl', '--version'], text=True).splitlines()[0],
                  driver_sha256=digest(Path(__file__)), cases=[], passed=False,
                  upstream_source=args.upstream_source,
                  profile='containment-evidence' if args.evidence_only else 'full-linux',
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
    if args.evidence_only:
        cases = []
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
        row['exit_code'] = execute(command, output/(name + '.log'))
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
    for name, (driver, verifier, integrity, expected_cases) in EVIDENCE_SUITES.items():
        directory = output/name
        target = output/(name + '-verification.json')
        row = dict(name=name, driver=driver, driver_sha256=digest(HERE/driver),
                   verifier_sha256=digest(HERE/verifier), integrity_sha256=digest(HERE/integrity),
                   passed=False)
        report['cases'].append(row)
        checkpoint()
        started = time.monotonic()
        try:
            command = [sys.executable, str(HERE/driver), '--binary', str(binary),
                       '--expected-binary-sha256', report['binary_sha256'], '--output', str(directory)]
            if name == 'broker':
                command += ['--runtime-source', revision]
                if args.upstream_source:
                    command += ['--upstream-source', args.upstream_source]
            row['exit_code'] = execute(command, output/(name + '.log'))
            row['verification_exit_code'] = execute(
                [sys.executable, str(HERE/verifier), str(directory)], target)
            if row['exit_code'] == 0 and row['verification_exit_code'] == 0:
                row['integrity_exit_code'] = execute(
                    [sys.executable, str(HERE/integrity), str(directory)], output/(name + '-integrity.json'))
                row['passed'] = (row['integrity_exit_code'] == 0 and
                    evidence_result(directory, target, report['binary_sha256'], expected_cases))
                row['manifest_sha256'] = digest(directory/'manifest.json')
        except (OSError, ValueError, KeyError) as error:
            row['error'] = str(error)
        row['seconds'] = round(time.monotonic()-started, 3)
        checkpoint()
        print(f'{name}: {"passed" if row["passed"] else "FAILED"} ({row["seconds"]}s)', flush=True)
    report['binary_unchanged'] = digest(binary) == report['binary_sha256']
    report['passed'] = all(row['passed'] for row in report['cases']) and report['binary_unchanged']
    checkpoint()
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

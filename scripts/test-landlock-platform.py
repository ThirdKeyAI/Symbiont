#!/usr/bin/env python3
"""Record native Linux deployment prerequisites and real shipping diagnostics.

Does not change kernel or distribution security policy. An expected namespace
refusal is a separately labelled diagnostic result, never a qualified platform.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import traceback

from delegated_service_fixture import AutomaticDelegatedService


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--expected-binary-sha256',required=True)
    parser.add_argument('--expected-abi',type=int,required=True)
    parser.add_argument('--expect',choices=['usable','namespace-refused'],required=True)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    if not __debug__ or platform.system()!='Linux' or os.geteuid()==0 or platform.machine() not in ['x86_64','aarch64']:
        parser.error('Requires non-root Linux x86-64 or arm64 and enabled assertions')
    binary=args.binary.resolve(strict=True)
    assert digest(binary)==args.expected_binary_sha256 and not args.report.exists()
    settings={name:Path('/proc/sys/'+name.replace('.','/')).read_text().strip()
              if Path('/proc/sys/'+name.replace('.','/')).exists() else None for name in
              ['kernel.apparmor_restrict_unprivileged_userns','kernel.unprivileged_userns_clone','user.max_user_namespaces']}
    report=dict(passed=False,qualified=False,expected=args.expect,kernel=platform.release(),machine=platform.machine(),
                distribution=Path('/etc/os-release').read_text(),landlock_abi=ctypes.CDLL(None).syscall(444,0,0,1),
                systemd=subprocess.check_output(['systemctl','--version'],text=True).splitlines()[0],
                uid=os.geteuid(),caller_cgroup=Path('/proc/self/cgroup').read_text(),policy=settings,
                binary_sha256=digest(binary),driver_sha256=digest(__file__))
    temporary=Path(tempfile.mkdtemp(prefix='symbi-platform-'))
    state=temporary/'state';state.mkdir(mode=0o700)
    service=AutomaticDelegatedService(binary,state)
    try:
        assert report['landlock_abi']==args.expected_abi and args.expected_abi>=6
        try:
            service.start()
            usable=True
        except AssertionError:
            usable=False
        report['diagnostic_output']=service.doctor_output
        if args.expect=='usable':
            assert usable,service.doctor_output
            capacity=service.capacity()
            assert capacity['reserved']['workers']==0 and not list(state.glob('*.json'))
            report.update(qualified=True,capacity=capacity)
        else:
            assert not usable
            assert ('Native workspace:' in service.doctor_output or 'Managed CLI transport:' in service.doctor_output)
            assert 'namespace' in service.doctor_output and 'fallback' in service.doctor_output
            report['refusal_confirmed']=True
        report['passed']=True
    except Exception as error:
        report.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        try:
            service.stop()
            assert not list(state.glob('*.json')),'retained leases after diagnostic cleanup'
            shutil.rmtree(temporary)
            report['cleanup_confirmed']=True
        except Exception as error:
            report.update(passed=False,cleanup_error=repr(error))
        report['binary_unchanged']=digest(binary)==args.expected_binary_sha256
        report['policy_unchanged']=all(value==(Path('/proc/sys/'+name.replace('.','/')).read_text().strip()
            if Path('/proc/sys/'+name.replace('.','/')).exists() else None) for name,value in settings.items())
        report['passed'] &= report['binary_unchanged'] and report['policy_unchanged']
        args.report.parent.mkdir(parents=True,exist_ok=True)
        args.report.write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report,indent=2))
    return 0 if report['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())

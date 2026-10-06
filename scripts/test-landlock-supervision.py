#!/usr/bin/env python3
"""Real delegated supervisor lifecycle tests with disposable native workers.

Requires a built binary, systemd user delegation and Linux pidfds. No model,
credentials or external network. Pair with test-landlock-boundary.py for signed
shipping MCP dispatch through the restricted worker launcher.
"""
import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import time
import traceback
import uuid
from delegated_service_fixture import Connection, DelegatedService, eventually

PAYLOAD = r'''
import errno,json,os,time
children=[]
while True:
    try: pid=os.fork()
    except OSError as error:
        assert error.errno==errno.EAGAIN
        print(json.dumps({'pid':os.getpid(),'children':children,'fork_denied':True,'sum':10}),flush=True)
        break
    if pid==0:
        os.setsid()
        while True: time.sleep(1)
    children.append(pid)
while True: time.sleep(1)
'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    assert __debug__ and not args.report.exists()
    binary=args.binary.resolve(strict=True)
    def digest(path):
        with Path(path).open('rb') as stream:
            return hashlib.file_digest(stream,'sha256').hexdigest()
    report=dict(passed=False,binary_sha256=digest(binary),driver_sha256=digest(__file__),checks=[])
    connections=[]; processes=[]; pidfds=[]; descriptors=[]; service=None
    try:
        with tempfile.TemporaryDirectory(prefix='symbi-delegated-') as directory, ExitStack() as cleanup:
            root=Path(directory); state=root/'state'; state.mkdir(mode=0o700)
            (state/'admission.conf').write_text(json.dumps(dict(max_workers=1,memory_bytes=128*1024*1024,cpu_nanos=1_000_000_000)))
            (state/'admission.conf').chmod(0o600)
            service=cleanup.enter_context(DelegatedService(binary,state))
            def request(lifetime=20000):
                return dict(operation='create_host',**service.protocol,lease=str(uuid.uuid4()),
                            resources=dict(memory_bytes=64*1024*1024,cpu_nanos=500_000_000),
                            pids_limit=8,lifetime_ms=lifetime,startup_ms=min(5000,lifetime),origin=None)
            def reserve(lifetime=20000):
                spec=request(lifetime); connection=Connection(state,spec); connections.append(connection)
                assert connection.receive()==dict(status='registered',lease=spec['lease'])
                reply=connection.receive(); assert reply['status']=='host_created',reply
                group=Path(reply['cgroup']['path'])
                assert group.parent==service.cgroup and group.name=='worker-'+spec['lease']
                assert (group/'cpu.max').read_text().strip()=='50000 100000'
                assert (group/'memory.max').read_text().strip()==str(64*1024*1024)
                assert (group/'pids.max').read_text().strip()=='8'
                descriptor=os.open(group/'cgroup.procs',os.O_WRONLY|os.O_CLOEXEC); descriptors.append(descriptor)
                return connection,spec,group,descriptor
            def launch(descriptor):
                def join(): os.write(descriptor,b'0')
                process=subprocess.Popen(['/usr/bin/python3','-u','-c',PAYLOAD],stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,stderr=subprocess.PIPE,preexec_fn=join)
                processes.append(process)
                assert select.select([process.stdout],[],[],3)[0], 'worker failed to start'
                value=json.loads(process.stdout.readline())
                assert value['pid']==process.pid and value['sum']==10 and value['fork_denied']
                assert len(value['children'])==7,value
                for pid in value['children']:
                    eventually(lambda: os.getsid(pid)==pid,3)
                fd=os.pidfd_open(value['children'][0]);pidfds.append(fd)
                return process,fd,value
            def empty(): return service.capacity()['reserved']['workers']==0
            def closed(connection):
                reply=connection.receive()
                if reply['status']=='failed': reply=connection.receive()
                assert reply['status']=='closed',reply
                eventually(empty)
            def dead(process,fd):
                process.wait(timeout=12)
                assert select.select([fd],[],[],12)[0], 'detached descendant survived'

            connection,spec,group,descriptor=reserve()
            process,fd,value=launch(descriptor)
            busy=service.capacity();report['busy']=busy
            assert busy['reserved']==dict(workers=1,memory_bytes=64*1024*1024,cpu_nanos=500_000_000)
            assert busy['workers'][0]['backend']=='landlock' and busy['admission_blocked']
            usage=service.request(dict(operation='inspect',lease=spec['lease'],**service.protocol))
            assert usage['status']=='measurement' and usage['measurement']['source']=='landlock_cgroup',usage
            assert usage['measurement']['memory_bytes']>0
            report['measurement']=usage['measurement']
            denied=service.request(request())
            assert denied['status']=='failed' and 'shared worker capacity exhausted' in denied['message'],denied
            docker_lease=str(uuid.uuid4())
            denied=service.request(dict(operation='create',**service.protocol,lease=docker_lease,name='symbi-'+docker_lease,
                docker_binary='/usr/bin/docker',docker_environment={},environment={},
                arguments=['create','--interactive','--name','symbi-'+docker_lease,'--network','none','--memory','64m','--cpus','0.5','python:3.12-slim'],
                resources=spec['resources'],staging=[],origin=None,lifetime_ms=10000,startup_ms=5000))
            assert denied['status']=='failed' and 'shared worker capacity exhausted' in denied['message'],denied
            report['checks'].append('Landlock and Docker requests share retained capacity; cgroup counters and PID limit match actual workers')
            process.kill();process.wait(timeout=3)
            assert not select.select([fd],[],[],0)[0], 'control descendant did not survive direct-parent exit'
            connection.cancel();closed(connection);dead(process,fd)
            assert not group.exists()
            try:
                late=subprocess.Popen(['/usr/bin/python3','-c','print("late execution")'],preexec_fn=lambda:os.write(descriptor,b'0'),stdout=subprocess.PIPE)
            except (OSError,subprocess.SubprocessError): pass
            else:
                processes.append(late)
                raise AssertionError('removed cgroup accepted a delayed launch')
            report['checks'].append('caller EOF kills setsid descendants; capacity releases after removal and retained cgroup descriptors cannot admit late work')

            connection,spec,group,descriptor=reserve()
            process,fd,_=launch(descriptor)
            obstruction=group/'cleanup-obstruction';obstruction.mkdir()
            connection.cancel();dead(process,fd)
            eventually(lambda: service.capacity()['reserved']['workers']==1)
            assert (state/(spec['lease']+'.json')).exists()
            denied=service.request(request())
            assert denied['status']=='failed' and 'capacity exhausted' in denied['message'],denied
            usage=service.request(dict(operation='inspect',lease=spec['lease'],**service.protocol))
            assert usage['status']=='failed',usage
            obstruction.rmdir();eventually(empty)
            report['checks'].append('failed cgroup removal retains admission and reports usage unavailable until durable reconciliation succeeds')

            connection,spec,group,descriptor=reserve(1000)
            process,fd,_=launch(descriptor)
            failure=connection.receive();assert failure['status']=='failed' and 'lifetime expired' in failure['message'],failure
            closed(connection);dead(process,fd)
            report['checks'].append('independent deadline stops all descendants and reports expired outcome')

            for failure in [signal.SIGKILL,signal.SIGSTOP]:
                connection,spec,group,descriptor=reserve()
                process,fd,_=launch(descriptor)
                manager=service.pid();manager_fd=os.pidfd_open(manager);pidfds.append(manager_fd)
                os.kill(manager,failure)
                assert select.select([manager_fd],[],[],15)[0], 'service manager did not reap stalled supervisor'
                dead(process,fd)
                assert (state/(spec['lease']+'.json')).exists(), 'unacknowledged lease was lost'
                service.stop();service.start();eventually(empty)
                report['checks'].append(('supervisor death' if failure==signal.SIGKILL else 'watchdog expiry')+' kills detached descendants; restart reconciles retained charges')
            report['passed']=True

    except Exception as error:
        report.update(error=repr(error),traceback=traceback.format_exc())
    finally:
        for connection in connections: connection.close()
        for fd in descriptors+pidfds: os.close(fd)
        for process in processes:
            if process.poll() is None: process.kill()
            process.wait(timeout=5)
        assert digest(binary)==report['binary_sha256'],'binary changed during test'
        args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(passed=report['passed'],report=str(args.report))))
    return 0 if report['passed'] else 1

if __name__=='__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Shipping managed launch with a local synthetic provider and real Linux isolation.

Exercises the actual broker adapters, Cedar policy, signed audit and automatic
supervisor. Pass --executable to exercise an installed Claude Code instead of the
synthetic CLI. Neither mode contacts a model service or needs account credentials.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import runpy
import shlex
import subprocess
import tempfile
import threading
import time
import tomllib
import traceback
from delegated_service_fixture import AutomaticDelegatedService, eventually


PAYLOAD = r'''#!/usr/bin/python3
import ctypes, errno, http.client, json, os, pathlib, socket, subprocess, sys, time
if sys.argv[1:] == ['--version']:
    print('Synthetic managed CLI 1'); sys.exit(0)
options=json.loads(sys.argv[-1])
assert os.getcwd()=='/tmp/symbi-workspace'
assert os.environ['ANTHROPIC_API_KEY']=='private-broker-channel'
assert 'FIXTURE_PROVIDER_KEY' not in os.environ and 'AMBIENT_CANARY' not in os.environ
assert 'SYMBI_TOOLS_FD' not in os.environ and 'SYMBI_INFERENCE_FD' not in os.environ
assert 'r-xp' in pathlib.Path('/proc/self/maps').read_text()
assert pathlib.Path('/proc/self/stat').read_text().startswith(str(os.getpid())+' ')
try: pathlib.Path('/proc/self/maps').write_text('denied')
except OSError: pass
else: raise AssertionError('process metadata was writable')
probe=subprocess.run(['/usr/bin/python3','-c',"open('/proc/self/maps').read()"],capture_output=True)
assert probe.returncode != 0 and b'PermissionError' in probe.stderr
for name in ['/proc/self/cgroup','/proc/1/maps',f"/proc/{options['host_pid']}/maps", options['canary'], options['source'], options['sibling'], '/proc/self/environ', '/sys/fs/cgroup/cgroup.procs']:
    try: pathlib.Path(name).read_bytes()
    except OSError: pass
    else: raise AssertionError('ungranted file readable: '+name)
for address in [('127.0.0.1',options['host_port']), ('192.0.2.1',443)]:
    try: socket.create_connection(address,.2).close()
    except OSError: pass
    else: raise AssertionError('external connection succeeded')
try: socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
except OSError: pass
else: raise AssertionError('host Unix socket creation allowed')
libc=ctypes.CDLL(None,use_errno=True)
assert libc.unshare(0x10000000)==-1 and ctypes.get_errno()==errno.EACCES
pathlib.Path('scratch.txt').write_text('private development work')
# Two requests must traverse one inherited inference capability successfully.
for index in range(2):
    client=http.client.HTTPConnection('127.0.0.1',8765,timeout=10)
    client.request('POST','/v1/messages',json.dumps(dict(model='fixture',max_tokens=16,messages=[dict(role='user',content='local fixture')])).encode(),{'Content-Type':'application/json'})
    reply=client.getresponse(); body=json.loads(reply.read()); assert reply.status==200 and body['content'][0]['text']=='synthetic inference'
    client.close()
config=json.loads(sys.argv[sys.argv.index('--mcp-config')+1])['mcpServers']['symbi']
bridge=subprocess.Popen([config['command'],*config['args']],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
def send(value):
    bridge.stdin.write(json.dumps(value)+'\n'); bridge.stdin.flush()
def rpc(identity,method,params):
    send(dict(jsonrpc='2.0',id=identity,method=method,params=params))
    value=json.loads(bridge.stdout.readline()); assert value['id']==identity; return value
assert 'result' in rpc(1,'initialize',dict(protocolVersion='2025-06-18',capabilities={},clientInfo=dict(name='fixture',version='1')))
send(dict(jsonrpc='2.0',method='notifications/initialized'))
expected=['grep_files','list_files','read_file'] if options.get('onboarding') else ['read_file']
assert sorted(tool['name'] for tool in rpc(2,'tools/list',{})['result']['tools'])==expected
result=rpc(3,'tools/call',dict(name='read_file',arguments=dict(path='input.txt')))
if options['denied']:
    assert 'error' in result or result.get('result',{}).get('isError'), result
    assert 'approved source text' not in json.dumps(result)
else:
    assert 'approved source text' in json.dumps(result), result
    refused=rpc(4,'tools/call',dict(name='read_file',arguments=dict(path='../host-canary')))
    assert 'error' in refused or refused.get('result',{}).get('isError'), refused
if options.get('onboarding'):
    listed=rpc(5,'tools/call',dict(name='list_files',arguments={}))
    assert 'input.txt' in json.dumps(listed), listed
    searched=rpc(6,'tools/call',dict(name='grep_files',arguments=dict(needle='approved source text')))
    assert 'input.txt' in json.dumps(searched), searched
    for identity,name,arguments in [(7,'write_file',dict(path='new.txt',content='refused')),
                                    (8,'edit_file',dict(path='input.txt',content='refused')),
                                    (9,'Bash',dict(command='touch new.txt')),
                                    (10,'read_file',dict(path='escape')),
                                    (11,'read_file',dict(path=options['canary']))]:
        refused=rpc(identity,'tools/call',dict(name=name,arguments=arguments))
        assert 'error' in refused or refused.get('result',{}).get('isError'), refused
    for path in [options['source'], str(pathlib.Path(options['source']).parent/'new.txt')]:
        try: pathlib.Path(path).write_text('refused')
        except OSError: pass
        else: raise AssertionError('direct source write succeeded')
bridge.stdin.close(); assert bridge.wait(timeout=5)==0, bridge.stderr.read()
time.sleep(.4)
print(json.dumps(dict(type='result',subtype='success',is_error=False,result='managed fixture complete',containment=True)),flush=True)
'''


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    assert __debug__, 'Assertions must be enabled'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--executable', type=Path, help='Test this installed Claude Code with a synthetic local provider')
    parser.add_argument('--onboarding', action='store_true', help='Review a project generated entirely by init --profile dev-agent')
    args = parser.parse_args()
    assert not args.report.exists(), 'Preserve previous reports'
    binary = args.binary.resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix='symbi-native-managed-'))
    project = root/'control' if args.onboarding else root
    # The executable lives outside /tmp, which the native worker shadows.
    executable_root = Path(tempfile.mkdtemp(prefix='native-cli-', dir=args.report.resolve().parent))
    executable = executable_root/'cli'
    if args.executable:
        executable = args.executable.resolve(strict=True)
    else:
        executable.write_text(PAYLOAD); executable.chmod(0o700)
    (executable_root/'sibling').write_text('ungranted sibling')
    report = dict(passed=False, fixture=str(root), binary_sha256=digest(binary), driver_sha256=digest(__file__), cases=[])
    report['cli'] = dict(kind='installed' if args.executable else 'synthetic', path=str(executable), sha256=digest(executable))
    service, provider, process = None, None, None
    received = []
    try:
        for name in ['source','home','state','observer']:
            (root/name).mkdir(mode=0o700,parents=True,exist_ok=True)
        (root/'source/input.txt').write_text('approved source text')
        (root/'host-canary').write_text('ungranted host data')
        if args.onboarding:
            (root/'source/escape').symlink_to(root/'host-canary')
        else:
            for name in ['agents','tools','policies/managed-cli']:
                (project/name).mkdir(mode=0o700,parents=True,exist_ok=True)
            (project/'.env').write_text('')
            (project/'agents/fixture.symbi').write_text('metadata { executor = "claude_code" allowed_tools = "read_file" }\nagent fixture() {}\n')
            (project/'tools/read_file.clad.toml').write_bytes((Path(__file__).resolve().parents[1]/'tools/read_file.clad.toml').read_bytes())
        class Provider(BaseHTTPRequestHandler):
            def log_message(self,*_): pass
            def do_POST(self):
                assert self.path in ('/v1/messages', '/v1/messages/count_tokens')
                assert self.headers['x-api-key']=='synthetic-provider-key'
                value=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert value['model']=='fixture'
                received.append(value)
                content = [dict(type='text',text='synthetic inference')]
                stop = 'end_turn'
                if args.executable and self.path == '/v1/messages':
                    assert value['max_tokens'] == 4096, value['max_tokens']
                    expected = ['mcp__symbi__grep_files','mcp__symbi__list_files','mcp__symbi__read_file'] if args.onboarding else ['mcp__symbi__read_file']
                    assert sorted(tool['name'] for tool in value['tools']) == expected, value['tools']
                    results = [block for message in value['messages'] if isinstance(message.get('content'), list) for block in message['content'] if block['type'] == 'tool_result']
                    if not results:
                        content = [dict(type='tool_use',id='read-source',name='mcp__symbi__read_file',input=dict(path='input.txt'))]
                        stop = 'tool_use'
                    else:
                        text = json.dumps(results)
                        assert ('approved source text' in text) != denied, results
                        if denied: assert results[-1].get('is_error'), results
                        content = [dict(type='text',text='managed fixture complete')]
                message = dict(id='fixture',type='message',role='assistant',model='fixture',content=content,stop_reason=stop,stop_sequence=None,usage=dict(input_tokens=1,output_tokens=1))
                kind='application/json'
                if self.path.endswith('/count_tokens'):
                    body=json.dumps(dict(input_tokens=1)).encode()
                elif value.get('stream'):
                    events=[dict(type='message_start',message={**message,'content':[],'stop_reason':None,'usage':dict(input_tokens=1,output_tokens=0)})]
                    for index, block in enumerate(content):
                        initial = {**block, 'text':''} if block['type']=='text' else {**block,'input':{}}
                        delta = dict(type='text_delta',text=block['text']) if block['type']=='text' else dict(type='input_json_delta',partial_json=json.dumps(block['input']))
                        events += [dict(type='content_block_start',index=index,content_block=initial), dict(type='content_block_delta',index=index,delta=delta),dict(type='content_block_stop',index=index)]
                    events += [dict(type='message_delta',delta=dict(stop_reason=stop,stop_sequence=None),usage=dict(output_tokens=1)),dict(type='message_stop')]
                    body=''.join('event: '+event['type']+'\ndata: '+json.dumps(event)+'\n\n' for event in events).encode()
                    kind='text/event-stream'
                else: body=json.dumps(message).encode()
                self.send_response(200); self.send_header('Content-Type',kind); self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)
        provider=ThreadingHTTPServer(('127.0.0.1',0),Provider)
        provider.daemon_threads=True
        threading.Thread(target=provider.serve_forever,daemon=True).start()
        if args.onboarding:
            from developer_onboarding_fixture import check_initialization
            report['initialization'] = check_initialization(binary, project, root/'source', executable, provider.server_port)
            config = tomllib.loads((project/'symbiont.toml').read_text())
            assert config['sandbox']['roots'] == dict(source_roots=[str(root/'source')+':/workspace:ro'], output_roots=[])
            assert config['managed_cli']['executable'] == str(executable.resolve())
            assert not received, 'init contacted the provider'
        else:
            (project/'symbiont.toml').write_text('[sandbox]\ntier="landlock"\n[sandbox.roots]\nsource_roots='+json.dumps([str(root/'source')+':/workspace:ro'])+'\n[managed_cli]\nexecutable='+json.dumps(str(executable))+'\n[managed_cli.inference]\nbase_url='+json.dumps(f'http://127.0.0.1:{provider.server_port}')+'\nmodel="fixture"\napi_key_env="FIXTURE_PROVIDER_KEY"\nmax_requests=8\nmax_output_tokens_per_request=4096\nrequest_timeout_seconds=10\n')
        service=AutomaticDelegatedService(binary,root/'state').start(project if args.onboarding else None)
        report['doctor'] = service.doctor_output
        if args.onboarding:
            assert 'Managed CLI --version:' in service.doctor_output, service.doctor_output
            assert not received, 'doctor contacted the provider'
        report['service']=service.unit
        env=dict(PATH='/usr/bin:/bin',HOME=str(root/'home'),LANG='C.UTF-8',SYMBIONT_ENV='production',SYMBIONT_SANDBOX_STATE_DIR=str(root/'state'),FIXTURE_PROVIDER_KEY='synthetic-provider-key',AMBIENT_CANARY='not inherited')
        verifier=runpy.run_path(str(Path(__file__).with_name('test-direct-inference.py')))['verify_journal']
        for denied in [False,True]:
            received.clear()
            if not args.onboarding or denied:
                (project/'policies/managed-cli/fixture.cedar').write_text(('' if args.onboarding else 'permit(principal,action,resource);\n')+('forbid(principal,action == Action::"tool_call::read_file",resource);\n' if denied else ''))
            before=set((project/'.symbiont/governed').glob('*.jsonl'))
            options=dict(denied=denied,onboarding=args.onboarding,host_pid=os.getpid(),host_port=provider.server_port,canary=str(root/'host-canary'),source=str(root/'source/input.txt'),sibling=str(executable_root/'sibling'))
            if args.onboarding:
                command = shlex.split(report['initialization']['review_command'])
                command[0] = str(binary)
                if not args.executable:
                    command[command.index('--input')+1] = json.dumps(options)
            else:
                command = [str(binary),'run','fixture','--target',str(root/'source'),'--input',json.dumps(options)]
            process=subprocess.Popen([*command,'--budget-timeout','45s'],cwd=project,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            row=dict(name='denied' if denied else 'allowed',passed=False,workers=[]); report['cases'].append(row)
            seen=set(); end=time.monotonic()+55
            while process.poll() is None and time.monotonic()<end:
                for path in (root/'state').glob('*.json'):
                    try:
                        record=json.loads(path.read_text()); state=record['state']
                        if state['phase']!='host_created' or record['lease'] in seen: continue
                        group=Path(state['cgroup']['path'])
                        for pid in (group/'cgroup.procs').read_text().split():
                            status=Path(f'/proc/{pid}/status').read_text()
                            network=os.readlink(f'/proc/{pid}/ns/net')
                            if network==os.readlink('/proc/self/ns/net') or 'NoNewPrivs:\t1' not in status: continue
                            assert os.readlink(f'/proc/{pid}/ns/mnt')!=os.readlink('/proc/self/ns/mnt')
                            assert set(Path(f'/proc/{pid}/net/dev').read_text().splitlines()[2:][0].split(':')[0].strip().split())=={'lo'}
                            row['workers'].append(dict(lease=record['lease'],pid=int(pid),network=network,origin=record['origin'],staging=record['staging']))
                            seen.add(record['lease']); break
                    except (FileNotFoundError,ProcessLookupError): pass
                time.sleep(.02)
            stdout,stderr=process.communicate(timeout=5)
            row.update(exit_code=process.returncode,stdout=stdout,stderr=stderr,provider_requests=list(received)); process=None
            assert row['exit_code']==0 and 'managed fixture complete' in stdout and len([r for r in received if 'max_tokens' in r])==2 and len(row['workers'])==1,row
            journals=set((project/'.symbiont/governed').glob('*.jsonl'))-before
            assert len(journals)==1
            journal=journals.pop()
            first=json.loads(journal.read_text().splitlines()[0])['payload']
            principal=first['entry']['agent_id']
            key=project/'.symbiont/governed/audit-signing.key'
            seed=key.read_bytes()
            public=subprocess.run(['openssl','pkey','-inform','DER','-pubout','-outform','DER'],input=bytes.fromhex('302e020100300506032b657004220420')+seed,capture_output=True,check=True,timeout=5).stdout
            public_file=root/'observer/public.der'; public_file.write_bytes(public)
            entries=verifier(journal,public_file,root/'observer',principal,legacy=True)
            assert 'Terminated' in entries[-1]['event']
            assert sum('InferenceCompleted' in e['event'] for e in entries)==len(received)
            assert 'synthetic-provider-key' not in journal.read_text()
            row['journal']=dict(path=str(journal),sha256=digest(journal),public_key=public[-32:].hex())
            eventually(lambda: not list((root/'state').glob('*.json')) and not list((root/'state/staging').glob('*')))
            if args.onboarding:
                assert (root/'source/input.txt').read_text() == 'approved source text'
                assert sorted(path.name for path in (root/'source').iterdir()) == ['escape','input.txt']
                assert service.capacity()['reserved']['workers'] == 0
            row['passed']=True
            print(json.dumps(dict(case=row['name'],passed=True)),flush=True)
        assert digest(binary)==report['binary_sha256']
        report['passed']=True
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}',traceback=traceback.format_exc())
    finally:
        if process is not None and process.poll() is None:
            process.kill(); process.communicate(timeout=5)
        if provider is not None: provider.shutdown(); provider.server_close()
        if service is not None:
            try: service.stop()
            except Exception as error: report.update(passed=False,cleanup_error=str(error))
        args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(passed=report['passed'],report=str(args.report))))
    return 0 if report['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())

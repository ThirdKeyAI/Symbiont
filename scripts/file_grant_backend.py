"""Shared real-worker observations for declared-file shipping fixtures."""
import hashlib
import json
import os
from delegated_service_fixture import AutomaticDelegatedService, eventually
from pathlib import Path
from worker_origin_observer import verify_origin


def digest(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def arguments(parser):
    parser.add_argument('--landlock', action='store_true')
    parser.add_argument('--firecracker-binary', type=Path)
    parser.add_argument('--kernel', type=Path)
    parser.add_argument('--rootfs', type=Path)


class Backend:
    def __init__(self, args, root, data, label, docker, report, vmm_launcher=None):
        self.args, self.root, self.label, self.docker = args, root, label, docker
        self.landlock = args.landlock
        self.service = None
        self.vm = args.firecracker_binary is not None
        assert not (self.vm and self.landlock)
        assert self.vm == (args.kernel is not None) == (args.rootfs is not None)
        assert not args.report.exists(), 'Preserve existing reports'
        self.seen = []
        self.workdir = '/tmp' if self.vm else '/workspace'
        self.python = '/usr/bin/python3' if self.vm or self.landlock else '/usr/local/bin/python3'
        self.uid = os.getuid() if self.landlock else 65534
        self.artifacts = {}
        report['backend_observer_sha256'] = digest(__file__)
        report['origin_observer_sha256'] = digest(Path(__file__).with_name('worker_origin_observer.py'))
        if self.landlock:
            self.workdir = '/tmp/symbi-workspace'
            config = '[sandbox]\ntier="landlock"\n[sandbox.roots]\n'
            config += 'source_roots=' + json.dumps([str(data) + ':/workspace:ro']) + '\n'
            config += 'output_roots=' + json.dumps([str(data) + ':/workspace:rw']) + '\n'
            self.service = AutomaticDelegatedService(args.binary.resolve(strict=True), root/'state').start()
            report['service'] = self.service.unit
        elif self.vm:
            for name in ['firecracker_binary', 'kernel', 'rootfs']:
                path = getattr(args, name).resolve(strict=True)
                setattr(args, name, path)
                self.artifacts[str(path)] = digest(path)
            report['vm_artifacts'] = self.artifacts
            config = '[sandbox]\ntier="firecracker"\n[sandbox.firecracker]\nworking_dir="/tmp"\n'
            config += '\n'.join(key + '=' + json.dumps(str(value)) for key, value in [
                ('kernel_image_path', args.kernel), ('rootfs_path', args.rootfs), ('firecracker_binary', vmm_launcher or args.firecracker_binary)])
            config += '\nsource_roots=' + json.dumps([str(data) + ':/tmp:ro'])
            config += '\noutput_roots=' + json.dumps([str(data) + ':/tmp:rw']) + '\n'
        else:
            image = docker('image', 'inspect', args.image, '--format', '{{.Id}}')
            report['image'] = image
            config = '[sandbox]\ntier="docker"\n[sandbox.docker]\nimage=' + json.dumps(image)
            config += '\nvolumes=' + json.dumps([str(data) + ':/workspace:rw'])
            config += '\nextra_flags=' + json.dumps(['--label=' + label]) + '\n'
        (root / 'symbiont.toml').write_text(config)
        report['config_sha256'] = digest(root / 'symbiont.toml')

    def active(self):
        if self.vm or self.landlock:
            return [path.stem for path in (self.root / 'state').glob('*.json')]
        return self.docker('ps', '-aq', '--filter', 'label=' + self.label).split()

    def observe(self):
        found = []
        if self.landlock:
            for path in (self.root/'state').glob('*.json'):
                try:
                    record = json.loads(path.read_text())
                    state, identity = record['state'], record['lease']
                    if state['phase'] != 'host_created' or any(w['id'] == identity for w in self.seen):
                        continue
                    group = Path(state['cgroup']['path'])
                    for pid in (group/'cgroup.procs').read_text().split():
                        if os.readlink(f'/proc/{pid}/ns/mnt') == os.readlink('/proc/self/ns/mnt'):
                            continue
                        status = Path(f'/proc/{pid}/status').read_text()
                        if 'NoNewPrivs:\t1' not in status:
                            continue
                        mounts = Path(f'/proc/{pid}/mountinfo').read_text()
                        if not any(row.split()[4] == '/tmp' and ' - tmpfs ' in row for row in mounts.splitlines()):
                            continue
                        spec = json.loads((self.root/'state/staging'/record['staging'][0]/'data/launch.json').read_text())
                        bound = []
                        for item in spec['mounts']:
                            rows = [row.split() for row in mounts.splitlines() if row.split()[4] == item['destination']]
                            assert len(rows) == 1, (item, mounts)
                            writable = 'rw' in rows[0][5].split(',')
                            assert writable != item['read_only']
                            bound.append({'Type':'bind', 'Destination':item['destination'], 'RW':writable})
                        found.append({'id':identity,'origin':record['origin'],'pid':int(pid),
                                      'cgroup':str(group),'mounts':bound,'staging':record['staging']})
                        break
                except (FileNotFoundError, ProcessLookupError):
                    continue
        elif self.vm:
            for path in (self.root / 'state').glob('*.json'):
                try:
                    record = json.loads(path.read_text())
                    state, identity = record['state'], record['lease']
                    if state['phase'] != 'vm_created' or any(w['id'] == identity for w in self.seen):
                        continue
                    pid = state['pid']
                    assert int(Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]) == state['start_ticks']
                    if Path(f'/proc/{pid}/exe').resolve() != self.args.firecracker_binary:
                        # A test launcher may still be collecting pre-exec evidence.
                        # Callers must require a subsequently observed real VMM.
                        continue
                    config_path = self.root / 'state' / ('vm-' + identity) / 'vm-config.json'
                    config = json.loads(config_path.read_text())
                    assert len(config['drives']) == 1
                    assert config['drives'][0]['path_on_host'] == str(self.args.rootfs)
                    assert config['drives'][0]['is_read_only'] is True and not config.get('network-interfaces')
                    found.append({'id': identity, 'origin': record['origin'], 'pid': pid, 'start_ticks': state['start_ticks'],
                                  'configuration': config, 'configuration_sha256': digest(config_path)})
                except FileNotFoundError:
                    continue
        else:
            for identity in self.docker('ps', '-q', '--no-trunc', '--filter', 'label=' + self.label).split():
                if not any(w['id'] == identity for w in self.seen):
                    row = json.loads(self.docker('inspect', identity))[0]
                    lease = row['Config']['Labels']['ai.symbiont.lease']
                    record = json.loads((self.root / 'state' / (lease + '.json')).read_text())
                    assert record['state']['container_id'] == identity
                    found.append({'id': identity, 'lease': lease, 'origin': record['origin'], 'mounts': row['Mounts'], 'ulimits': row['HostConfig']['Ulimits']})
        self.seen.extend(found)
        return found

    def mounts(self, workers, expected):
        if self.vm:
            assert all(len(w['configuration']['drives']) == 1 for w in workers)
        else:
            actual = [(m['Destination'], m['RW']) for w in workers for m in w['mounts'] if m['Type'] == 'bind']
            assert sorted(actual) == sorted(expected), (actual, expected)

    def origins(self, workers, entries, agent, run, public_key):
        return [verify_origin(worker['origin'], entries, agent, run, public_key) for worker in workers]

    def boundary(self, boundary):
        if self.landlock:
            assert boundary['tier'] == 'landlock'
            assert boundary['landlock']['broker_source_roots'] == boundary['landlock']['broker_output_roots'] == []
        elif self.vm:
            assert boundary['tier'] == 'firecracker'
            assert boundary['vm']['broker_source_roots'] == boundary['vm']['broker_output_roots'] == []
        else:
            assert boundary['container']['mounts'] == []

    def cleaned(self):
        if self.landlock:
            eventually(lambda: not self.active() and not list((self.root / 'state/staging').glob('*')), 10)
        assert not self.active()
        assert not list((self.root / 'state').glob('*.json'))
        assert not list((self.root / 'state/staging').glob('*'))
        if self.vm:
            for worker in self.seen:
                try:
                    stat = Path(f"/proc/{worker['pid']}/stat").read_text()
                    assert int(stat.rsplit(')', 1)[1].split()[19]) != worker['start_ticks'], 'VMM survived cleanup'
                except FileNotFoundError:
                    pass

    def publication(self, entries, data, expected):
        effects = [e['event']['ToolEffect']['effect'] for e in entries if 'ToolEffect' in e['event']]
        prepared = [e['FilePublicationPrepared']['intent'] for e in effects if 'FilePublicationPrepared' in e]
        finished = [e['FilePublicationFinished'] for e in effects if 'FilePublicationFinished' in e]
        confirmed = [entry for entry in finished if entry['confirmed']]
        assert len(confirmed) == len(expected), (confirmed, expected)
        for name in expected:
            path = data / name
            matches = [intent for intent in prepared if intent['sha256'] == digest(path) and intent['bytes'] == path.stat().st_size]
            assert len(matches) == 1
            assert any(f['publication_id'] == matches[0]['publication_id'] and f['error'] is None for f in confirmed)

    def finish(self, report):
        report['remaining_workers'] = self.active()
        report['remaining_leases'] = [str(p) for p in (self.root / 'state').glob('*.json')]
        try:
            self.cleaned()
        finally:
            if self.service is not None:
                self.service.stop()
        assert all(digest(path) == expected for path, expected in self.artifacts.items()), 'VM artifact changed'

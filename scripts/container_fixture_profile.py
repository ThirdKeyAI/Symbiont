"""Select and observe explicit container backends in shipping fixture drivers.

This is test infrastructure. Configuration and binary hashes do not themselves
qualify the selected runtime or authenticate the Docker daemon.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def runsc_artifact(runsc, registered):
    runsc = Path(runsc)
    require(not runsc.is_symlink() and runsc.is_file() and os.access(runsc, os.X_OK),
            'the canonical runsc path must remain an executable regular file')
    require(Path(registered).resolve(strict=True) == runsc,
            'registered runsc path changed')
    folder = runsc.parent / 'gvisor-bin'
    require(not folder.is_symlink(), 'gVisor sidecar directory links are unsupported')
    bundle = None
    if folder.exists():
        require(folder.is_dir(), 'gvisor-bin must be a directory')
        entries = {}
        for entry in sorted(folder.rglob('*')):
            metadata = entry.lstat()
            mode = stat.S_IMODE(metadata.st_mode)
            if stat.S_ISREG(metadata.st_mode):
                value = {'kind': 'file', 'mode': mode, 'sha256': digest(entry)}
            elif stat.S_ISDIR(metadata.st_mode):
                value = {'kind': 'directory', 'mode': mode}
            else:
                raise ValueError('gVisor sidecars must be regular files or directories')
            entries[str(entry.relative_to(folder))] = value
        require(any(value['kind'] == 'file' for value in entries.values()),
                'gVisor sidecar directory is empty')
        bundle = {'path': str(folder), 'mode': stat.S_IMODE(folder.stat().st_mode), 'entries': entries}
    return {'path': str(runsc), 'registered_path': registered, 'sha256': digest(runsc),
            'mode': stat.S_IMODE(runsc.stat().st_mode), 'bundle': bundle}


def arguments(parser):
    selected = parser.add_mutually_exclusive_group()
    selected.add_argument('--landlock', action='store_true')
    selected.add_argument('--gvisor', action='store_true')
    selected.add_argument('--firecracker-binary', type=Path)
    parser.add_argument('--kernel', type=Path)
    parser.add_argument('--rootfs', type=Path)
    parser.add_argument('--runsc-binary', type=Path, default=Path('/usr/bin/runsc'))


def selected_backend(args):
    native = bool(getattr(args, 'landlock', False))
    gvisor = bool(getattr(args, 'gvisor', False))
    vm = getattr(args, 'firecracker_binary', None) is not None
    require(sum((native, gvisor, vm)) <= 1, 'select only one sandbox backend')
    require(vm == (getattr(args, 'kernel', None) is not None) ==
            (getattr(args, 'rootfs', None) is not None),
            'provide all three VM artifacts only with the Firecracker backend')
    return 'landlock' if native else 'gvisor' if gvisor else 'firecracker' if vm else 'docker'


def registered_runsc(docker):
    runtimes = json.loads(docker('info', '--format', '{{json .Runtimes}}'))
    require(isinstance(runtimes, dict) and isinstance(runtimes.get('runsc'), dict),
            'Docker must register the runsc runtime')
    registered = runtimes['runsc'].get('path')
    require(isinstance(registered, str) and Path(registered).is_absolute(),
            'Docker must register runsc at an explicit absolute binary path')
    return registered


def container_config(args, image, workdir, volumes, flags, report, docker, *, docker_binary=None):
    backend = selected_backend(args)
    require(backend in ('docker', 'gvisor'), 'a container backend is required')
    require(re.fullmatch(r'sha256:[a-f0-9]{64}', image) and image != 'sha256:' + '0' * 64,
            'use the inspected immutable container image ID')
    require(report.get('backend', backend) == backend, 'selected backend changed between cases')
    report['backend'] = backend
    report['container_profile_sha256'] = digest(__file__)
    config = '[sandbox]\ntier=' + json.dumps(backend) + '\n'
    if backend == 'gvisor':
        runsc = args.runsc_binary.resolve(strict=True)
        require(runsc.is_file() and os.access(runsc, os.X_OK), 'runsc must be an executable regular file')
        registered = registered_runsc(docker)
        require(Path(registered).resolve(strict=True) == runsc,
                'registered runsc does not match the selected binary')
        artifact = runsc_artifact(runsc, registered)
        require(report.get('runsc', artifact) == artifact, 'gVisor artifacts changed between fixture cases')
        report['runsc'] = artifact
        config += '[sandbox.gvisor]\nruntime_name="runsc"\nrunsc_binary=' + json.dumps(str(runsc)) + '\n'
        config += '[sandbox.gvisor.docker]\n'
    else:
        config += '[sandbox.docker]\n'
    values = {'working_dir': workdir, 'image': image, 'volumes': volumes, 'extra_flags': flags}
    if docker_binary is not None:
        values['docker_binary'] = str(docker_binary)
    return config + ''.join(key + '=' + json.dumps(value) + '\n' for key, value in values.items())


def observe_runtime(args, actual):
    backend = selected_backend(args)
    require(backend in ('docker', 'gvisor'), 'unexpected container worker for the selected backend')
    require(isinstance(actual, str) and bool(actual), 'Docker inspection did not identify a runtime')
    if backend == 'gvisor':
        require(actual == 'runsc', 'gVisor fixture worker did not use runsc')
    return actual


def verify_artifacts(report, docker):
    if report.get('backend') == 'gvisor':
        artifact = report.get('runsc')
        require(isinstance(artifact, dict), 'missing runsc artifact binding')
        require(runsc_artifact(artifact['path'], registered_runsc(docker)) == artifact,
                'gVisor artifacts changed during the fixture')

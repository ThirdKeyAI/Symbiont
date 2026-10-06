#!/usr/bin/env python3
"""Test-only VMM launcher: observe private snapshots before guest payload release."""
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import time


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def main():
    assert __debug__
    root, vmm = Path(sys.argv[1]), Path(sys.argv[2])
    argv = sys.argv[3:]
    config = Path(argv[argv.index('--config-file') + 1])
    assert config.is_relative_to(root / 'state')
    identity = config.parent.name.removeprefix('vm-')
    directories = list((root / 'state/staging').glob('*/data'))
    assert len(directories) == 1
    data = directories[0]
    manifest = hashlib.sha256()
    entries, files, size, path_bytes = 0, 0, 0, 0
    snapshots = {}

    def visit(relative):
        nonlocal entries, files, size, path_bytes
        path = data / relative
        metadata = path.lstat()
        if stat.S_ISDIR(metadata.st_mode):
            kind, length, hashed, executable = 'directory', 0, hashlib.sha256(b'').hexdigest(), False
        elif stat.S_ISLNK(metadata.st_mode):
            target = os.fsencode(os.readlink(path))
            kind, length, hashed, executable = 'symlink', len(target), hashlib.sha256(target).hexdigest(), False
        else:
            assert stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1
            kind, length, hashed, executable = 'file', metadata.st_size, digest(path), bool(metadata.st_mode & 0o111)
        header = json.dumps(dict(path=relative, kind=kind, length=length, sha256=hashed, executable=executable), ensure_ascii=False, separators=(',', ':')).encode()
        manifest.update(len(header).to_bytes(4, 'big'))
        manifest.update(header)
        entries += 1
        files += kind != 'directory'
        size += length
        path_bytes += len(relative.encode())
        if kind == 'directory':
            for child in sorted(path.iterdir(), key=lambda p: p.name):
                visit(relative + '/' + child.name)
        else:
            snapshots[relative] = hashed

    for name in ['input', 'worktree']:
        visit(name)
    grant = dict(entries=entries, files=files, bytes=size, path_bytes=path_bytes, manifest_sha256=manifest.hexdigest())
    observer = root / 'observer'
    (observer / ('worker-' + identity + '.json')).write_text(json.dumps(dict(id=identity, snapshots=snapshots,
        manifest=grant, launcher_pid=os.getpid(), config_sha256=digest(config), prepared_before_guest=True)))
    (observer / 'created').touch()
    deadline = time.monotonic() + 15
    while not (observer / 'release').exists() and time.monotonic() < deadline:
        time.sleep(.01)
    assert (observer / 'release').exists(), 'observer did not release the VMM'
    os.execv(str(vmm), [str(vmm), *argv])


if __name__ == '__main__':
    main()

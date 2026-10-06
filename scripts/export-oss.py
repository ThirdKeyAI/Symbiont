#!/usr/bin/env python3
"""Export tracked OSS working-tree files without cloning, signing or publishing."""
import argparse
import fnmatch
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOTS = {'crates', 'docs', '.github', 'src', 'agents', 'policies', 'docker',
         'deploy', 'examples', 'tools', 'tests', '.cargo'}
FILES = {'README.md', 'README.zh-cn.md', 'README.es.md', 'README.pt.md', 'README.ja.md',
         'README.de.md', 'Cargo.toml', 'Cargo.lock', 'LICENSE', 'CODE_OF_CONDUCT.md',
         'SECURITY.md', 'CHANGELOG.md', '.gitignore', '.dockerignore', '.env.example',
         'Dockerfile', 'deny.toml', 'justfile', 'logo-hz.png', 'symbi-trans.png',
         'SKILL.md', '.schemapin.sig', 'context7.json', 'AGENTS.md', 'CLAUDE.md',
         'Cross.toml', 'zensical.toml'}
# Explicit public tooling; private sync, deployment and signing scripts stay out.
SCRIPTS = {
    'install.sh', 'publish-crates.py', 'export-oss.py', 'test-release-tools.py',
    'test-governed-improvements.py', 'test-intake-improvements.py',
    'test-intake-phase2.py', 'test-intake-actions.py', 'delegated_service_fixture.py',
    'developer_onboarding_fixture.py', 'test-landlock-onboarding.py',
    'test-filesystem-grants.py', 'test-source-broker.py', 'test-git-source.py',
    'test-landlock-managed.py', 'test-landlock-supervision.py',
    'test-landlock-boundary.py', 'file_grant_backend.py', 'git_snapshot_observer.py',
    'worker_origin_observer.py', 'test-cron-occurrences.py',
    'test-direct-inference.py', 'test-workflow-execution.py', 'test-shell-governance.py',
}
SOURCE_EXTENSIONS = {'.rs', '.py', '.ts', '.tsx', '.js', '.jsx', '.go', '.md', '.toml', '.yaml', '.yml'}


def allowed(path):
    parts = path.parts
    if any(p in {'target', 'node_modules', '__pycache__', '.git', '.symbiont'} for p in parts):
        return False
    if any(p == '.env' or (p.startswith('.env.') and p != '.env.example') for p in parts):
        return False
    if parts[:2] in [('docs', 'plans'), ('docs', 'superpowers')]:
        return False
    return (parts[0] in ROOTS or path.as_posix() in FILES
            or len(parts) == 2 and parts[0] == 'scripts' and parts[1] in SCRIPTS)


def check_file(path, relative):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f'OSS export refuses linked or non-regular file: {relative}')
    if not allowed(relative):
        raise ValueError(f'path outside OSS allowlist: {relative}')
    name = relative.name.lower()
    strict = ('*.key', '*.pem', '*.p12', '*.pfx', '*.private.jwk.json',
              '*.env.local', '*.env.production', '.roomodes', 'security_audit_report.md')
    if any(fnmatch.fnmatch(name, pattern) for pattern in strict) or (
            ('secret' in name or 'private' in name) and path.suffix not in SOURCE_EXTENSIONS):
        raise ValueError(f'sensitive file in OSS export: {relative}')


def validate(root):
    for path in root.rglob('*'):
        relative = path.relative_to(root)
        if relative.parts[0] == '.git':
            continue
        if path.is_symlink() or not path.is_dir():
            check_file(path, relative)
        elif not allowed(relative / 'placeholder') and relative.as_posix() != 'scripts':
            raise ValueError(f'directory outside OSS allowlist: {relative}')
    for required in ['crates/runtime', 'crates/dsl', 'crates/sandbox-guest',
                     'crates/sandbox-supervisor', 'src', 'docs', '.github', 'agents',
                     'examples', 'README.md', 'Cargo.toml', 'Cargo.lock', 'LICENSE',
                     'CODE_OF_CONDUCT.md', 'SECURITY.md', 'CHANGELOG.md',
                     *('scripts/' + name for name in SCRIPTS)]:
        if not (root / required).exists():
            raise ValueError(f'required OSS path missing: {required}')
    manifest = tomllib.loads((root / 'Cargo.toml').read_text())
    for member in manifest['workspace']['members']:
        if not (root / member / 'Cargo.toml').is_file():
            raise ValueError(f'workspace member missing: {member}')


def export(source, destination):
    # Index selection excludes private untracked artifacts even inside OSS roots.
    tracked = subprocess.check_output(['git', '-C', str(source), 'ls-files', '-z']).decode().split('\0')
    count = 0
    for name in filter(None, tracked):
        relative = Path(name)
        if not allowed(relative):
            continue
        path = source / relative
        if not path.exists() and not path.is_symlink():  # tracked deletion
            continue
        if any(parent.is_symlink() for parent in path.parents if parent != source and source in parent.parents):
            raise ValueError(f'linked parent in OSS source: {relative}')
        # The repository's instruction alias is a tracked link to AGENTS.md.
        # Materialize that exact alias as a regular file; no other link is copied.
        if relative.as_posix() == 'CLAUDE.md' and path.is_symlink() and path.readlink() == Path('AGENTS.md'):
            path = source / 'AGENTS.md'
        check_file(path, relative)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        # Refuse an inherited mirror symlink rather than following it during copy.
        if target.is_symlink() or any(p.is_symlink() for p in target.parents):
            raise ValueError(f'linked destination: {relative}')
        shutil.copy2(path, target)
        count += 1
    validate(destination)
    print(f'Validated {count} tracked OSS files in {destination}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--destination', type=Path)
    parser.add_argument('--check', type=Path)
    args = parser.parse_args()
    if bool(args.destination) == bool(args.check):
        parser.error('choose --destination or --check')
    if args.check:
        validate(args.check.resolve())
    else:
        source, destination = args.source.resolve(), args.destination.resolve()
        if destination == source or source in destination.parents:
            parser.error('export destination must be outside the source checkout')
        destination.mkdir(parents=True, exist_ok=True)
        export(source, destination)


if __name__ == '__main__':
    main()

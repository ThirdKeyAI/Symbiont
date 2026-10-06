#!/usr/bin/env python3
"""Validate the complete workspace publish graph; upload only with --publish."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request


def publish_order(metadata):
    members = set(metadata['workspace_members'])
    packages = {p['name']: p for p in metadata['packages'] if p['id'] in members}
    public = {name: p for name, p in packages.items() if p.get('publish') != []}
    dependencies = {}
    for name, package in public.items():
        dependencies[name] = set()
        for dep in package['dependencies']:
            if not dep.get('path') or dep.get('kind') == 'dev':
                continue
            target = public.get(dep['name'])
            if target is None:
                raise ValueError(f'{name} depends on an unpublished workspace package: {dep["name"]}')
            if dep['req'] not in (target['version'], '^' + target['version'], '=' + target['version']):
                raise ValueError(f'{name}: {dep["name"]} must reference local version {target["version"]}')
            dependencies[name].add(dep['name'])
    result = []
    while dependencies:
        ready = sorted(name for name, deps in dependencies.items() if not deps)
        if not ready:
            raise ValueError('workspace publish dependency cycle')
        for name in ready:
            result.append(public[name])
            del dependencies[name]
        for deps in dependencies.values():
            deps.difference_update(ready)
    return result


def published(name, version):
    request = urllib.request.Request(
        f'https://crates.io/api/v1/crates/{name}/{version}',
        headers={'User-Agent': 'symbiont-release-check/1.0'})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            record = json.load(response)['version']
        if record['num'] != version or record['crate'] != name or record.get('yanked'):
            raise ValueError(f'{name} {version}: unexpected or yanked registry version')
        return True
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise


def publish(package, exists=published, run=subprocess.run):
    name, version = package['name'], package['version']
    if exists(name, version):
        print(f'{name} {version}: already published', flush=True)
        return
    result = run(['cargo', 'publish', '--locked', '-p', name], check=False)
    if result.returncode:
        # Never interpret an "Uploading" progress line as success. A failed
        # upload is tolerable only if the exact desired version now exists.
        if not exists(name, version):
            raise RuntimeError(f'cargo publish failed for {name} {version}: exit {result.returncode}')
        time.sleep(30)  # allow an upload accepted before a client failure to index


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--publish', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    os.chdir(root)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--offline', '--locked', '--no-deps', '--format-version', '1'], text=True))
    order = publish_order(metadata)
    for package in order:
        print(f'{package["name"]} {package["version"]}', flush=True)
    if args.publish:
        if not os.environ.get('CARGO_REGISTRY_TOKEN'):
            parser.error('--publish requires CARGO_REGISTRY_TOKEN')
        for package in order:
            publish(package)


if __name__ == '__main__':
    main()

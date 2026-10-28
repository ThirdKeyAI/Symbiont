#!/usr/bin/env python3
"""Offline regressions for publication failures and the actual OSS export filter."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import runpy
import re
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import Mock

HERE = Path(__file__).resolve().parent
publish = runpy.run_path(str(HERE / 'publish-crates.py'))
export = runpy.run_path(str(HERE / 'export-oss.py'))


def package(name, dependencies=(), public=True):
    return dict(id=name, name=name, version='1.21.0', publish=None if public else [],
                dependencies=[dict(name=d, path='/workspace/' + d, req='^1.21.0', kind=None)
                              for d in dependencies])


class ReleaseTests(unittest.TestCase):
    def test_installer_resolves_latest_and_rejects_ambiguous_or_corrupt_assets(self):
        for case in ['valid', 'corrupt', 'duplicate', 'unsafe-name']:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                fixtures, commands, home = root / 'fixtures', root / 'commands', root / 'home'
                for directory in [fixtures, commands, home]:
                    directory.mkdir()
                archive = 'symbi-v2.0.0-x86_64-unknown-linux-gnu.tar.gz'
                content = b'#!/bin/sh\necho symbi 2.0.0\n'
                with tarfile.open(fixtures / archive, 'w:gz') as bundle:
                    entry = tarfile.TarInfo('symbi')
                    entry.size, entry.mode = len(content), 0o755
                    bundle.addfile(entry, io.BytesIO(content))
                digest = hashlib.sha256((fixtures / archive).read_bytes()).hexdigest()
                line = f'{digest}  {archive}\n'
                if case == 'corrupt':
                    (fixtures / archive).write_bytes(b'corrupt archive')
                if case == 'duplicate':
                    line += line
                if case == 'unsafe-name':
                    line = f'{digest}  ../{archive}\n'
                (fixtures / 'checksums.txt').write_text(line + '0' * 64 + '  symbi-v2.0.0-aarch64-apple-darwin.tar.gz\n')
                (commands / 'uname').write_text('#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; esac\n')
                (commands / 'curl').write_text('''#!/usr/bin/python3
import os
from pathlib import Path
import shutil
import sys
args = sys.argv[1:]
url = next(a for a in args if a.startswith('https://'))
with open(os.environ['DOWNLOAD_LOG'], 'a') as log:
    log.write(url + '\\n')
shutil.copyfile(Path(os.environ['FIXTURES']) / url.rsplit('/', 1)[1], args[args.index('-o') + 1])
''')
                for command in commands.iterdir():
                    command.chmod(0o755)
                log = root / 'downloads'
                result = subprocess.run(['bash', str(HERE / 'install.sh')], cwd=root,
                    env=dict(os.environ, HOME=str(home), SYMBI_INSTALL_DIR=str(root / 'installed'),
                             PATH=str(commands) + ':/usr/bin:/bin', FIXTURES=str(fixtures), DOWNLOAD_LOG=str(log)),
                    capture_output=True, text=True, timeout=20)
                installed = root / 'installed/bin/symbi'
                if case == 'valid':
                    self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
                    self.assertEqual(installed.read_bytes(), content)
                    self.assertEqual(log.read_text().splitlines(), [
                        'https://github.com/thirdkeyai/symbiont/releases/latest/download/checksums.txt',
                        f'https://github.com/thirdkeyai/symbiont/releases/download/v2.0.0/{archive}'])
                else:
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    self.assertFalse(installed.exists())
                    if case in ['duplicate', 'unsafe-name']:
                        self.assertEqual(len(log.read_text().splitlines()), 1)

    def test_security_catalogue_references_survive_export(self):
        catalogue = json.loads((HERE.parent / 'docs/security/controls.json').read_text())
        for control in catalogue['controls']:
            for reference in control['sources'] + control['tests']:
                with self.subTest(control=control['id'], reference=reference):
                    self.assertTrue((HERE.parent / reference).is_file(), reference)
                    self.assertTrue(export['allowed'](Path(reference)), reference)

    def test_public_workflow_script_references_survive_export(self):
        for workflow in (HERE.parent / '.github/workflows').glob('*.yml'):
            references = set(re.findall(
                r'(?<![\w/.-])(?:\.github/)?scripts/[\w.-]+\.(?:py|sh|json)',
                workflow.read_text()))
            for reference in references:
                with self.subTest(workflow=workflow.name, script=reference):
                    self.assertTrue((HERE.parent / reference).is_file(), reference)
                    self.assertTrue(export['allowed'](Path(reference)), reference)

    def test_workflow_preserves_validated_source_and_shell_boundaries(self):
        workflow = (HERE.parent / '.github/workflows/release-binaries.yml').read_text()
        self.assertIn("if: github.event_name == 'release' || github.ref == 'refs/heads/main'", workflow)
        self.assertIn('ref: ${{ needs.release.outputs.sha }}', workflow)
        self.assertIn('tag_name: ${{ needs.release.outputs.tag }}', workflow)
        self.assertIn('uses: ./.github/workflows/release-gate.yml', workflow)
        self.assertIn('QUALIFIED_SHA: ${{ needs.qualify.outputs.sha }}', workflow)
        self.assertIn('run: test "$SOURCE_SHA" = "$QUALIFIED_SHA"', workflow)
        self.assertIn('permissions:\n  contents: read', workflow)
        self.assertEqual(workflow.count('contents: write'), 1)
        self.assertEqual(workflow.count('id-token: write'), 1)
        self.assertIn('artifacts/release-source.json', workflow)
        self.assertIn("target: x86_64-unknown-linux-gnu\n            # Pin the oldest supported glibc build environment.", workflow)
        self.assertIn('os: ubuntu-22.04', workflow)
        self.assertIn('dpkg --compare-versions "${MAX_GLIBC#GLIBC_}" le 2.35', workflow)
        run_indent = None
        for line in workflow.splitlines():
            indent = len(line) - len(line.lstrip())
            if run_indent is not None and line.strip() and indent <= run_indent:
                run_indent = None
            if line.strip().startswith('run:'):
                run_indent = indent
            if run_indent is not None:
                self.assertIsNone(re.search(r'\$\{\{\s*(github\.event|inputs\.|needs\.release\.outputs|steps\..*\.outputs)', line), line)

    def test_release_source_is_an_existing_tag_on_main(self):
        validator = HERE.parent / '.github/scripts/release-source.sh'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            env = dict(os.environ, GITHUB_OUTPUT=str(root / 'output'),
                       GIT_AUTHOR_NAME='Release Test', GIT_COMMITTER_NAME='Release Test',
                       GIT_AUTHOR_EMAIL='test@example.invalid', GIT_COMMITTER_EMAIL='test@example.invalid')
            def git(*args):
                return subprocess.check_output(['git', '-C', str(root), *args], env=env).decode().strip()
            git('init', '-q')
            git('commit', '--allow-empty', '-qm', 'Trusted release fixture')
            trusted = git('rev-parse', 'HEAD')
            git('update-ref', 'refs/remotes/origin/main', trusted)
            git('tag', 'v1.2.3')
            git('commit', '--allow-empty', '-qm', 'Unreviewed fixture')
            unreviewed = git('rev-parse', 'HEAD')
            git('tag', 'v9.0.0')
            for tag, event, sha, success in [
                ('v1.2.3', 'workflow_dispatch', unreviewed, True),
                ('v1.2.3', 'release', trusted, True),
                ('v1.2.3', 'release', unreviewed, False),
                ('v9.0.0', 'workflow_dispatch', unreviewed, False),
                ('v8.0.0', 'workflow_dispatch', trusted, False),
            ]:
                (root / 'output').write_text('')
                result = subprocess.run(['bash', str(validator)], cwd=root, capture_output=True,
                    env=dict(env, RELEASE_TAG=tag, RELEASE_EVENT=event, RELEASE_EVENT_SHA=sha))
                self.assertEqual(result.returncode == 0, success, result.stderr)
                if success:
                    self.assertIn(f'sha={trusted}\n', (root / 'output').read_text())
                else:
                    self.assertNotIn('sha=', (root / 'output').read_text())

    def test_release_tag_preserves_versions_and_rejects_shell_and_path_syntax(self):
        validator = HERE.parent / '.github/scripts/release-tag.sh'
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'output'
            marker = Path(temporary) / 'executed'
            for tag in ['v1.21.1', '1.0.0-beta', '1.0.0-beta2', 'v2.0.0-rc.1+build.2']:
                output.write_text('')
                result = subprocess.run(['bash', str(validator)], capture_output=True,
                                        env=dict(os.environ, RELEASE_TAG=tag, GITHUB_OUTPUT=str(output)))
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output.read_text(), f'tag={tag}\nversion={tag.removeprefix("v")}\n')
            for tag in ['', '../v1.2.3', 'v1.2.3/extra', 'v1.2.3\nversion=other',
                        'v1.2.3"; exit 0; #', f'v1.2.3$(touch {marker})',
                        f'v1.2.3\\$(touch {marker})', f'v1.2.3`touch {marker}`',
                        'v1.2.3&replacement', 'v1.2.3\r', 'v1.2.3 ', '-v1.2.3']:
                output.write_text('')
                result = subprocess.run(['bash', str(validator)], capture_output=True,
                                        env=dict(os.environ, RELEASE_TAG=tag, GITHUB_OUTPUT=str(output)))
                self.assertNotEqual(result.returncode, 0, tag)
                self.assertEqual(output.read_text(), '', tag)
                self.assertFalse(marker.exists(), tag)

    def test_optional_sandbox_dependencies_precede_runtime_and_root(self):
        packages = [package('symbi', ['runtime']), package('runtime', ['guest', 'supervisor']),
                    package('guest'), package('supervisor'), package('e2e', public=False)]
        packages[1]['dependencies'][0]['optional'] = True
        order = publish['publish_order'](dict(workspace_members=[p['id'] for p in packages], packages=packages))
        self.assertEqual([p['name'] for p in order], ['guest', 'supervisor', 'runtime', 'symbi'])
        packages[0]['dependencies'][0]['req'] = '^1.20.0'
        with self.assertRaisesRegex(ValueError, 'local version'):
            publish['publish_order'](dict(workspace_members=[p['id'] for p in packages], packages=packages))

    def test_failed_upload_is_not_success_from_progress_output(self):
        run = Mock(return_value=subprocess.CompletedProcess([], 101, stdout='Uploading symbi to registry'))
        exists = Mock(return_value=False)
        with self.assertRaisesRegex(RuntimeError, 'exit 101'):
            publish['publish'](package('symbi'), exists=exists, run=run)
        self.assertEqual(exists.call_count, 2)
        run.assert_called_once_with(['cargo', 'publish', '--locked', '-p', 'symbi'], check=False)

    def test_existing_exact_version_does_not_upload(self):
        run = Mock()
        with contextlib.redirect_stdout(io.StringIO()):
            publish['publish'](package('symbi'), exists=lambda *_: True, run=run)
        run.assert_not_called()

    def test_export_checks_tracked_boundary_secrets_and_linked_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, destination = root / 'source', root / 'export'
            source.mkdir()
            destination.mkdir()
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            for name in ['crates/runtime', 'crates/dsl', 'crates/sandbox-guest', 'crates/sandbox-supervisor',
                         'src', 'docs', '.github', 'agents', 'examples']:
                (source / name).mkdir(parents=True, exist_ok=True)
                (source / name / 'fixture.rs').write_text('// synthetic\n')
            for name in ['README.md', 'Cargo.lock', 'LICENSE', 'CODE_OF_CONDUCT.md', 'SECURITY.md', 'CHANGELOG.md']:
                (source / name).write_text('fixture\n')
            (source / 'Cargo.toml').write_text('[workspace]\nmembers=[]\n')
            (source / 'scripts').mkdir()
            for name in export['SCRIPTS']:
                (source / 'scripts' / name).write_text('# fixture\n')
            for name in ['enterprise/private.rs', '.gitea/workflows/internal.yml',
                         'docs/plans/internal.md', 'scripts/internal-sync.sh']:
                (source / name).parent.mkdir(parents=True, exist_ok=True)
                (source / name).write_text('not public\n')
            (source / 'AGENTS.md').write_text('public instructions\n')
            subprocess.run(['git', '-C', str(source), 'add', '.'], check=True)
            (source / 'docs/untracked.md').write_text('private draft\n')
            with contextlib.redirect_stdout(io.StringIO()):
                export['export'](source, destination)
            self.assertEqual((destination / 'AGENTS.md').read_text(), 'public instructions\n')
            self.assertFalse((destination / 'enterprise').exists())
            self.assertFalse((destination / 'docs/untracked.md').exists())
            self.assertFalse((destination / 'docs/plans').exists())
            self.assertFalse((destination / 'scripts/internal-sync.sh').exists())
            self.assertTrue((destination / 'scripts/test-intake-actions.py').exists())
            secret = source / 'src/leaked.private.jwk.json'
            secret.write_text('{}')
            subprocess.run(['git', '-C', str(source), 'add', str(secret)], check=True)
            with self.assertRaisesRegex(ValueError, 'sensitive file'):
                export['export'](source, destination)
            secret.unlink()
            link = source / 'docs/link.md'
            link.symlink_to(source / 'README.md')
            subprocess.run(['git', '-C', str(source), 'add', str(link)], check=True)
            with self.assertRaisesRegex(ValueError, 'linked'):
                export['export'](source, destination)


if __name__ == '__main__':
    unittest.main()

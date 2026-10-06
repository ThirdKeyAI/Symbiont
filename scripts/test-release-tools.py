#!/usr/bin/env python3
"""Offline regressions for publication failures and the actual OSS export filter."""
import contextlib
import io
from pathlib import Path
import runpy
import subprocess
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
            (source / 'CLAUDE.md').symlink_to('AGENTS.md')
            subprocess.run(['git', '-C', str(source), 'add', '.'], check=True)
            (source / 'docs/untracked.md').write_text('private draft\n')
            with contextlib.redirect_stdout(io.StringIO()):
                export['export'](source, destination)
            self.assertFalse((destination / 'CLAUDE.md').is_symlink())
            self.assertEqual((destination / 'CLAUDE.md').read_text(), 'public instructions\n')
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

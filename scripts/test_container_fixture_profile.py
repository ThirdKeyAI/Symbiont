"""Configuration and observation checks; these do not execute or qualify runsc."""
import argparse
from contextlib import redirect_stderr
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import tomllib
import unittest

from container_fixture_profile import (
    arguments, container_config, observe_runtime, selected_backend, verify_artifacts,
)


class ContainerFixtureProfile(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runsc = self.root / 'runsc'
        self.runsc.write_text('#!/bin/sh\nexit 0\n')
        self.runsc.chmod(0o700)
        self.image = 'sha256:' + 'a' * 64

    def args(self, *values):
        parser = argparse.ArgumentParser()
        arguments(parser)
        return parser.parse_args(values)

    def docker(self, *values):
        self.assertEqual(values, ('info', '--format', '{{json .Runtimes}}'))
        return json.dumps({'runc': {'path': 'runc'}, 'runsc': {'path': str(self.runsc)}})

    def configuration(self, args, report=None, docker=None):
        return tomllib.loads(container_config(
            args, self.image, '/source', ['/synthetic:/source:ro'],
            ['--label=fixture'], report if report is not None else {},
            docker or self.docker, docker_binary='/synthetic/client'))

    def test_default_keeps_the_docker_profile(self):
        report = {}
        config = self.configuration(self.args(), report)
        self.assertEqual(config['sandbox']['tier'], 'docker')
        self.assertNotIn('gvisor', config['sandbox'])
        self.assertEqual(config['sandbox']['docker']['volumes'], ['/synthetic:/source:ro'])
        self.assertEqual(config['sandbox']['docker']['image'], self.image)
        self.assertEqual(report['backend'], 'docker')
        self.assertNotIn('runsc', report)

    def test_gvisor_uses_its_own_profile_and_explicit_runtime(self):
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        report = {}
        config = self.configuration(args, report)
        self.assertEqual(config['sandbox']['tier'], 'gvisor')
        self.assertNotIn('docker', config['sandbox'])
        gvisor = config['sandbox']['gvisor']
        self.assertEqual(gvisor['runtime_name'], 'runsc')
        self.assertEqual(gvisor['runsc_binary'], str(self.runsc.resolve()))
        self.assertEqual(gvisor['docker']['volumes'], ['/synthetic:/source:ro'])
        self.assertEqual(gvisor['docker']['docker_binary'], '/synthetic/client')
        self.assertEqual(report['backend'], 'gvisor')
        verify_artifacts(report, self.docker)

    def test_backend_selectors_are_mutually_exclusive(self):
        for values in [('--gvisor', '--landlock'), ('--gvisor', '--firecracker-binary', '/vm')]:
            with self.subTest(values=values), redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    self.args(*values)

    def test_incomplete_or_cross_backend_vm_artifacts_are_rejected(self):
        for values in [('--gvisor', '--kernel', '/kernel'),
                       ('--firecracker-binary', '/vm', '--kernel', '/kernel'),
                       ('--rootfs', '/rootfs')]:
            with self.subTest(values=values), self.assertRaises(ValueError):
                selected_backend(self.args(*values))

    def test_runsc_must_exist_and_be_executable(self):
        self.runsc.chmod(0o600)
        with self.assertRaises(ValueError):
            self.configuration(self.args('--gvisor', '--runsc-binary', str(self.runsc)))
        self.runsc.unlink()
        with self.assertRaises(OSError):
            self.configuration(self.args('--gvisor', '--runsc-binary', str(self.runsc)))

    def test_registered_runsc_must_match_the_selected_binary(self):
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        other = self.root / 'other-runsc'
        other.write_bytes(self.runsc.read_bytes())
        other.chmod(0o700)
        for value in [{}, {'runsc': {'path': 'runsc'}}, {'runsc': {'path': str(other)}}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.configuration(args, docker=lambda *_: json.dumps(value))

    def test_actual_worker_runtime_cannot_fall_back_to_runc(self):
        args = self.args('--gvisor')
        self.assertEqual(observe_runtime(args, 'runsc'), 'runsc')
        for runtime in ['runc', '', None]:
            with self.subTest(runtime=runtime), self.assertRaises(ValueError):
                observe_runtime(args, runtime)

    def test_changed_runsc_is_rejected_at_completion_and_between_cases(self):
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        report = {}
        self.configuration(args, report)
        self.runsc.write_text('#!/bin/sh\nexit 1\n')
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)
        with self.assertRaises(ValueError):
            self.configuration(args, report)

    def bundle(self):
        folder = self.root / 'gvisor-bin'
        folder.mkdir()
        sentry = folder / 'sentry'
        sentry.write_text('synthetic sentry bytes\n')
        sentry.chmod(0o755)
        return folder, sentry

    def test_sidecar_bytes_are_bound_and_changes_cannot_pass(self):
        _, sentry = self.bundle()
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        report = {}
        self.configuration(args, report)
        observed = report['runsc']['bundle']['entries']['sentry']
        self.assertEqual(observed['sha256'], hashlib.sha256(sentry.read_bytes()).hexdigest())
        verify_artifacts(report, self.docker)
        sentry.write_text('different synthetic sentry bytes\n')
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)
        with self.assertRaises(ValueError):
            self.configuration(args, report)

    def test_added_or_removed_sidecars_are_rejected(self):
        folder, sentry = self.bundle()
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        report = {}
        self.configuration(args, report)
        extra = folder / 'new-sidecar'
        extra.write_text('synthetic addition\n')
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)
        extra.unlink()
        sentry.unlink()
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)
        folder.rmdir()
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)

    def test_bundle_appearing_after_single_binary_selection_is_rejected(self):
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        report = {}
        self.configuration(args, report)
        self.bundle()
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)

    def test_sidecar_permissions_are_bound(self):
        _, sentry = self.bundle()
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        report = {}
        self.configuration(args, report)
        sentry.chmod(0o700)
        with self.assertRaises(ValueError):
            verify_artifacts(report, self.docker)

    def test_sidecar_links_and_special_files_are_rejected(self):
        folder, sentry = self.bundle()
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        link = folder / 'link'
        link.symlink_to(sentry)
        with self.assertRaises(ValueError):
            self.configuration(args)
        link.unlink()
        os.mkfifo(folder / 'pipe')
        with self.assertRaises(ValueError):
            self.configuration(args)

    def test_bundle_directory_link_is_rejected(self):
        original = self.root / 'elsewhere'
        original.mkdir()
        (original / 'sentry').write_text('synthetic bytes\n')
        (self.root / 'gvisor-bin').symlink_to(original, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.configuration(self.args('--gvisor', '--runsc-binary', str(self.runsc)))

    def test_existing_fixture_backend_selects_and_checks_gvisor(self):
        from file_grant_backend import Backend
        root = self.root / 'fixture'
        (root / 'state').mkdir(parents=True)
        (root / 'source').mkdir()
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        args.report = root / 'report.json'
        args.image = 'fixture@sha256:' + 'a' * 64
        identity, lease = 'b' * 64, 'fixture-lease'
        (root / 'state' / (lease + '.json')).write_text(json.dumps({
            'state': {'container_id': identity}, 'origin': {}}))

        def docker(*values):
            if values[0] == 'image':
                return self.image
            if values[0] == 'info':
                return self.docker(*values)
            if values[0] == 'ps':
                return identity
            if values == ('inspect', identity):
                return json.dumps([{'Config': {'Labels': {'ai.symbiont.lease': lease}},
                    'HostConfig': {'Runtime': 'runc', 'Ulimits': []}, 'Mounts': []}])
            self.fail('unexpected Docker call: ' + repr(values))

        backend = Backend(args, root, root / 'source', 'fixture', docker, {})
        config = tomllib.loads((root / 'symbiont.toml').read_text())
        self.assertEqual(config['sandbox']['tier'], 'gvisor')
        with self.assertRaisesRegex(ValueError, 'did not use runsc'):
            backend.observe()

    def test_completion_rechecks_the_daemon_runtime_registration(self):
        from file_grant_backend import Backend
        root = self.root / 'fixture'
        (root / 'state').mkdir(parents=True)
        (root / 'source').mkdir()
        args = self.args('--gvisor', '--runsc-binary', str(self.runsc))
        args.report = root / 'report.json'
        args.image = 'fixture@sha256:' + 'a' * 64
        registration = {'runsc': {'path': str(self.runsc)}}

        def docker(*values):
            if values[0] == 'image':
                return self.image
            if values[0] == 'info':
                return json.dumps(registration)
            if values[0] == 'ps':
                return ''
            self.fail('unexpected Docker call: ' + repr(values))

        report = {}
        backend = Backend(args, root, root / 'source', 'fixture', docker, report)
        backend.finish(report)
        other = self.root / 'other-runsc'
        other.write_bytes(self.runsc.read_bytes())
        other.chmod(0o700)
        alias = self.root / 'runsc-alias'
        alias.symlink_to(self.runsc)
        for value in ({}, {'runsc': {'path': str(other)}}, {'runsc': {'path': str(alias)}}):
            registration.clear()
            registration.update(value)
            with self.subTest(registration=value), self.assertRaises(ValueError):
                backend.finish(report)


if __name__ == '__main__':
    unittest.main()

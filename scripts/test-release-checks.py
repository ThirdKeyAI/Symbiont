#!/usr/bin/env python3
"""Offline regressions for the release authorization gate."""
from copy import deepcopy
from pathlib import Path
import runpy
import unittest

gate = runpy.run_path(str(Path(__file__).with_name('require-release-checks.py')))
REPO, SHA = 'ThirdKeyAI/Symbiont', 'a' * 40


def run(**changes):
    return dict(dict(id=10, run_attempt=1, head_sha=SHA, head_branch='main',
                     head_repository={'full_name': REPO}, event='push',
                     path='.github/workflows/test.yml', status='completed', conclusion='success'),
                **changes)


class ReleaseGateTests(unittest.TestCase):
    def check(self, runs, workflows=('test.yml',)):
        return gate['check'](REPO, SHA, workflows,
                             lambda _: {'workflow_runs': runs, 'total_count': len(runs)})

    def test_exact_canonical_commit_is_required(self):
        for change in [dict(head_sha='b' * 40), dict(head_branch='feature'),
                       dict(event='pull_request'), dict(head_repository={'full_name': 'fork/Symbiont'}),
                       dict(path='.github/workflows/unrelated.yml')]:
            pending, evidence = self.check([run(**change)])
            self.assertTrue(pending, change)
            self.assertFalse(evidence)
        pending, evidence = self.check([run()])
        self.assertFalse(pending)
        self.assertEqual(evidence[0]['sha'], SHA)

    def test_new_failure_and_in_progress_rerun_override_old_success(self):
        for conclusion in ('failure', 'cancelled', 'skipped', 'timed_out', 'neutral', None):
            with self.assertRaises(RuntimeError):
                self.check([run(), run(id=11, conclusion=conclusion)])
        pending, evidence = self.check([run(), run(run_attempt=2, status='in_progress', conclusion=None)])
        self.assertTrue(pending)
        self.assertFalse(evidence)

    def test_both_workflows_must_qualify(self):
        pending, _ = self.check([run()], ('test.yml', 'landlock-release.yml'))
        self.assertEqual(len(pending), 1)
        qualified = run(id=11, event='workflow_dispatch', path='.github/workflows/landlock-release.yml')
        pending, evidence = self.check([run(), qualified], ('test.yml', 'landlock-release.yml'))
        self.assertFalse(pending)
        self.assertEqual(len(evidence), 2)

    def test_unreadable_or_ambiguous_evidence_refuses(self):
        for payload in [{}, {'total_count': 101, 'workflow_runs': [run()]}]:
            with self.assertRaises((RuntimeError, KeyError)):
                gate['check'](REPO, SHA, ['test.yml'], lambda _: deepcopy(payload))


if __name__ == '__main__':
    unittest.main()

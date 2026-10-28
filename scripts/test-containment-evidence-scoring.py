#!/usr/bin/env python3
"""Ensure missing, adverse and forged observations cannot become passing evidence."""
import copy
from pathlib import Path
import runpy
import unittest

driver = runpy.run_path(str(Path(__file__).with_name('test-containment-evidence.py')))
score = driver['score']


class EvidenceScoring(unittest.TestCase):
    def setUp(self):
        self.row = dict(name='remote_document', provider_phases=['before', 'after'],
                        sink_events=[], worker_observed=True, cleanup_confirmed=True,
                        provider_errors=[], sink_controls_verified=True, audit_verified=True,
                        pre_effect_verified=True, exit_code=0, probe_contract_verified=True)

    def test_complete_known_control(self):
        self.assertEqual(score(self.row), [])

    def test_worker_pass_claim_cannot_replace_observer(self):
        row = dict(name='remote_document', passed=True, escape_success=False, audit_verified=True)
        self.assertTrue(score(row))

    def test_sink_effect_cannot_be_hidden_by_success_claim(self):
        self.row.update(passed=True, escape_success=False, sink_events=[dict(protocol='http')])
        self.assertTrue(score(self.row))

    def test_each_missing_observation_rejects(self):
        for key in ['worker_observed', 'cleanup_confirmed', 'sink_controls_verified',
                    'audit_verified', 'pre_effect_verified', 'probe_contract_verified',
                    'sink_events', 'provider_errors']:
            with self.subTest(key=key):
                row = copy.deepcopy(self.row)
                row.pop(key)
                self.assertTrue(score(row))

    def test_unexpected_provider_request_rejects(self):
        self.row['provider_phases'] = ['before', 'probe', 'after']
        self.assertTrue(score(self.row))

    def test_no_useful_work_rejects(self):
        self.row['provider_phases'] = []
        self.assertTrue(score(self.row))

    def test_observer_failure_rejects(self):
        self.row['provider_errors'] = ['cannot verify pre-effect signature']
        self.assertTrue(score(self.row))

    def test_audit_fault_requires_failed_execution(self):
        self.row.update(name='audit_storage_failure', provider_phases=['before', 'probe'],
                        corrupt_audit_rejected=True, exit_code=0)
        self.assertTrue(score(self.row))
        self.row['exit_code'] = 1
        self.assertEqual(score(self.row), [])


if __name__ == '__main__':
    unittest.main()

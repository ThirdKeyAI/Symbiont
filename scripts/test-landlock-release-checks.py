#!/usr/bin/env python3
"""Offline rejection checks for release evidence selection; no workers or builds."""
import json
from pathlib import Path
import runpy
import tempfile
import unittest

release = runpy.run_path(str(Path(__file__).with_name('test-landlock-release.py')))


class EvidenceSelectionTests(unittest.TestCase):
    def test_missing_partial_wrong_binary_and_changed_inventory_cannot_qualify(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root/'evidence'
            evidence.mkdir()
            manifest = evidence/'manifest.json'
            manifest.write_text('{"files":{}}\n')
            verification = root/'verification.json'
            check = lambda: release['evidence_result'](evidence, verification, 'a'*64, 19)
            self.assertFalse(check())
            valid = dict(passed=True, complete_suite=True, cases=19,
                         binary_sha256='a'*64, manifest_sha256=release['digest'](manifest))
            verification.write_text(json.dumps(valid))
            self.assertTrue(check())
            for field, value in [('passed', False), ('complete_suite', False), ('cases', 18),
                                 ('binary_sha256', 'b'*64), ('manifest_sha256', 'c'*64)]:
                with self.subTest(field=field):
                    verification.write_text(json.dumps(dict(valid, **{field:value})))
                    self.assertFalse(check())
            verification.write_text(json.dumps(valid))
            manifest.write_text('{"files":{"substituted":"0"}}\n')
            self.assertFalse(check())

    def test_truncated_verification_is_an_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            verification = root/'verification.json'
            verification.write_text('{"passed":true,')
            with self.assertRaises(ValueError):
                release['evidence_result'](root, verification, 'a'*64, 19)


if __name__ == '__main__':
    unittest.main()

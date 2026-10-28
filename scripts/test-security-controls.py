#!/usr/bin/env python3
"""Reject misleading or altered security qualifications using local fixtures."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import tempfile
import unittest

validate = runpy.run_path(str(Path(__file__).with_name("check-security-controls.py")))["validate"]


class EvidenceClaims(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        (self.root / "source.rs").write_text("// fixture\n")
        self.report = dict(passed=True, checks=[dict(name="request", passed=True)], errors=[],
                           source_revision="a" * 40, platform="fixture", timestamp="2026-10-26T00:00:00Z",
                           control_ids=["OUT-001"], routes=["cli"], backends=["http"])
        for field in ["source_digest", "binary_sha256", "configuration_digest", "driver_sha256", "verifier_sha256"]:
            self.report[field] = "b" * 64
        self.reference = dict(report="report.json", sha256="", routes=["cli"], backends=["http"])
        self.control = dict(id="OUT-001", title="fixture", threat="fixture", invariant="fixture", owner="fixture",
                            implementation_status="implemented", validation_status="validated", routes=["cli"],
                            backends=["http"], sources=["source.rs"], tests=[], limitations=["fixture only"],
                            qualifications=[self.reference])
        self.catalogue = dict(schema_version=1, scope="fixture", controls=[self.control])
        self.save_report()

    def save_report(self):
        path = self.root / "report.json"
        path.write_text(json.dumps(self.report))
        self.reference["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()

    def test_matching_scope_is_valid(self):
        self.assertEqual(validate(self.root, self.catalogue), 1)

    def test_changed_report_is_rejected(self):
        (self.root / "report.json").write_text("{}")
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            validate(self.root, self.catalogue)

    def test_unqualified_route_is_rejected(self):
        self.control["routes"].append("scheduler")
        with self.assertRaisesRegex(ValueError, "exceeds qualification scope"):
            validate(self.root, self.catalogue)
        self.reference["routes"].append("scheduler")
        with self.assertRaisesRegex(ValueError, "exceeds report scope"):
            validate(self.root, self.catalogue)

    def test_passing_flag_cannot_hide_failed_check(self):
        self.report["checks"][0]["passed"] = False
        self.save_report()
        with self.assertRaisesRegex(ValueError, "failed checks"):
            validate(self.root, self.catalogue)

    def test_unrelated_control_report_is_rejected(self):
        self.report["control_ids"] = ["OUT-002"]
        self.save_report()
        with self.assertRaisesRegex(ValueError, "does not cover"):
            validate(self.root, self.catalogue)

    def test_parent_reference_is_rejected(self):
        catalogue = copy.deepcopy(self.catalogue)
        catalogue["controls"][0]["sources"] = ["../source.rs"]
        with self.assertRaisesRegex(ValueError, "repository-relative"):
            validate(self.root, catalogue)


if __name__ == "__main__":
    unittest.main()

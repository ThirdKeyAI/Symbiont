#!/usr/bin/env python3
"""Validate the security catalogue and its explicit, revision-bound evidence.

Uses only the Python standard library. Test source references never count as
passing evidence. Qualification reports must identify the actual binary and
source/configuration digests, verifier, platform and observed test results.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re


def source_identity(root):
    """Bind compiled sources and manifests, excluding reports to avoid cycles."""
    files = [root / "Cargo.toml", root / "Cargo.lock"]
    for directory in ["src", "crates"]:
        files.extend(path for path in (root / directory).rglob("*")
                     if path.is_file() and path.suffix in {".rs", ".toml", ".c", ".h"}
                     and "target" not in path.relative_to(root).parts)
    entries = [(str(path.relative_to(root)), hashlib.sha256(source_path(root, str(path.relative_to(root))).read_bytes()).hexdigest())
               for path in sorted(set(files))]
    return hashlib.sha256(json.dumps(entries, separators=(",", ":")).encode()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def source_path(root, value):
    require(isinstance(value, str) and value, "empty source reference")
    path = PurePosixPath(value)
    require(not path.is_absolute() and ".." not in path.parts and "\\" not in value,
            "references must be portable repository-relative paths")
    actual = (root / value).resolve(strict=True)
    require(actual.is_relative_to(root) and actual.is_file(), "reference is outside the source tree")
    return actual


def validate(root, catalogue):
    require(set(catalogue) == {"schema_version", "scope", "controls"}, "unknown catalogue field")
    require(type(catalogue["schema_version"]) is int and catalogue["schema_version"] == 1,
            "unsupported catalogue schema")
    require(isinstance(catalogue["scope"], str) and catalogue["scope"], "missing scope")
    require(isinstance(catalogue["controls"], list) and 0 < len(catalogue["controls"]) <= 1000,
            "invalid control count")
    ids = set()
    fields = {"id", "title", "threat", "invariant", "owner", "implementation_status",
              "validation_status", "routes", "backends", "sources", "tests", "limitations", "qualifications"}
    for control in catalogue["controls"]:
        require(isinstance(control, dict) and set(control) == fields, "invalid control fields")
        for field in ["id", "title", "threat", "invariant", "owner"]:
            require(isinstance(control[field], str) and control[field].strip(), f"missing {field}")
        identity = control["id"]
        require(re.fullmatch(r"[A-Z]+-[0-9]{3}", identity) and identity not in ids, "invalid or duplicate control ID")
        ids.add(identity)
        require(control["implementation_status"] in {"implemented", "partial", "planned"}, "invalid implementation state")
        require(control["validation_status"] in {"validated", "partial", "untested", "failed"}, "invalid validation state")
        for field in ["routes", "backends", "sources", "tests", "limitations", "qualifications"]:
            require(isinstance(control[field], list), f"{identity}: invalid {field}")
        for field in ["routes", "backends", "sources", "tests", "limitations"]:
            require(all(isinstance(item, str) and item for item in control[field]), f"{identity}: empty {field}")
        require(control["routes"] and control["sources"] and control["limitations"], f"{identity}: missing scope or limitations")
        for reference in control["sources"] + control["tests"]:
            source_path(root, reference)
        coverage = set()
        passed = False
        for evidence in control["qualifications"]:
            require(set(evidence) == {"report", "sha256", "routes", "backends"}, f"{identity}: invalid evidence reference")
            path = source_path(root, evidence["report"])
            require(hashlib.sha256(path.read_bytes()).hexdigest() == evidence["sha256"], "qualification digest mismatch")
            report = json.loads(path.read_text())
            for field, size in [("source_revision", 40), ("source_digest", 64), ("binary_sha256", 64),
                                ("configuration_digest", 64), ("driver_sha256", 64), ("verifier_sha256", 64)]:
                require(re.fullmatch(f"[0-9a-f]{{{size}}}", report.get(field, "")), f"qualification missing {field}")
            require(report.get("platform") and report.get("timestamp") and report.get("checks"), "incomplete qualification")
            require(set(evidence["routes"]) <= set(control["routes"]), "evidence route exceeds control scope")
            require(set(evidence["backends"]) <= set(control["backends"]), "evidence backend exceeds control scope")
            require(isinstance(report["checks"], list) and all(
                isinstance(check, dict) and isinstance(check.get("name"), str)
                and type(check.get("passed")) is bool for check in report["checks"]),
                "invalid qualification checks")
            require(identity in report.get("control_ids", []), "report does not cover this control")
            require(evidence["routes"] and set(evidence["routes"]) <= set(report.get("routes", [])),
                    "evidence route exceeds report scope")
            require(evidence["backends"] and set(evidence["backends"]) <= set(report.get("backends", [])),
                    "evidence backend exceeds report scope")
            if report.get("passed") is True:
                require(not report.get("errors") and all(check["passed"] for check in report["checks"]),
                        "passing report contains failed checks or errors")
                passed = True
                coverage.update((route, backend) for route in evidence["routes"] for backend in evidence["backends"])
        if control["validation_status"] == "validated":
            expected = {(route, backend) for route in control["routes"] for backend in control["backends"]}
            require(expected and expected <= coverage, f"{identity}: validation claim exceeds qualification scope")
        elif control["validation_status"] == "partial":
            require(passed, f"{identity}: partial validation requires passing evidence")
        if control["implementation_status"] == "planned":
            require(control["validation_status"] == "untested", f"{identity}: planned control cannot be validated")
    return len(ids)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    catalogue = json.loads((root / "docs/security/controls.json").read_text())
    count = validate(root, catalogue)
    print(f"Security catalogue valid: {count} controls; scope and evidence claims checked")
    current = source_identity(root)
    reports = {reference["report"] for control in catalogue["controls"] for reference in control["qualifications"]}
    matching = sum(json.loads(source_path(root, path).read_text()).get("source_digest") == current for path in reports)
    print(f"Retained reports matching current source: {matching}/{len(reports)}; binary/platform qualification remains report-specific")


if __name__ == "__main__":
    main()

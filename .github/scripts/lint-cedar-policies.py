#!/usr/bin/env python3
"""Lint .cedar policy files for homoglyph / non-ASCII identifiers.

Symbiont agents sit behind Cedar policies. A frequent class of bug,
and a real attacker-pressure surface, is an identifier in a `permit`
or `forbid` clause that looks ASCII but isn't:

    permit(
        principal == Agent::"task_agent",
        action == Action::"tool_call::store_knоwledge",  // Cyrillic о
        resource
    );

Because Cedar matches action names as opaque strings, a homoglyph
identifier matches *different* action strings than the visually
equivalent Latin version. The effect is a policy that looks correct
on review but silently grants (or denies) the wrong set of actions.

Rules applied:

  1. Every `action == Action::"…"` literal must be pure ASCII.
  2. Every `principal == Agent::"…"` principal literal must be pure
     ASCII.
  3. No invisible / steganographic control chars anywhere in the file
     (ZWSP/tag-block/DEL/etc.) — keeps stego payloads out of the
     policy surface itself.

Exit code is the count of findings. Suitable as a pre-commit hook and
a CI gate.

Usage:
    .github/scripts/lint-cedar-policies.py                           # scan **/*.cedar from cwd
    .github/scripts/lint-cedar-policies.py path/to/policy.cedar …    # explicit paths
    .github/scripts/lint-cedar-policies.py --root some/dir           # scan only under --root

The forbidden-range table mirrors the one in the `symbi-invis-strip`
crate's `is_forbidden`. Drift between the two is a bug; update both.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# Keep in lockstep with `symbi-invis-strip::is_forbidden`. When that
# crate grows a new range, mirror it here.
FORBIDDEN_RANGES: list[tuple[int, int]] = [
    (0x00, 0x08),
    (0x0B, 0x0C),
    (0x0E, 0x1F),
    (0x7F, 0x7F),
    (0x80, 0x9F),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
    (0xFEFF, 0xFEFF),
    (0x180E, 0x180E),
    (0x1D173, 0x1D17A),
    (0xFE00, 0xFE0F),
    (0xE0000, 0xE007F),
    (0xE0100, 0xE01EF),
]

# Shallow tokenise — good enough for Cedar files. Adding a real parser
# is out of scope; this is a lint, not a compiler.
ID_PATTERN = re.compile(r'(Action|Agent|Principal|Resource)::"([^"]+)"')


def find_invisible(s: str) -> list[tuple[int, int]]:
    out = []
    for i, c in enumerate(s):
        code = ord(c)
        for lo, hi in FORBIDDEN_RANGES:
            if lo <= code <= hi:
                out.append((i, code))
                break
    return out


def lint_one(path: Path) -> int:
    try:
        text = path.read_text()
    except (OSError, UnicodeDecodeError) as e:
        print(f"  {path}:  failed to read ({e})")
        return 1
    findings = 0

    # Rules 1 & 2: identifiers must be pure ASCII.
    for m in ID_PATTERN.finditer(text):
        kind, ident = m.group(1), m.group(2)
        non_ascii = [(i, ord(c)) for i, c in enumerate(ident) if ord(c) >= 0x80]
        if non_ascii:
            line = text.count("\n", 0, m.start()) + 1
            code_hex = ", ".join(
                f"U+{c:04X} at pos {i}" for i, c in non_ascii[:4]
            )
            print(
                f"  {path}:{line}  {kind}::\"{ident}\" contains non-ASCII "
                f"code points ({code_hex}) — homoglyph risk; rewrite with ASCII."
            )
            findings += 1

    # Rule 3: no invisible-control chars anywhere.
    hits = find_invisible(text)
    if hits:
        by_line: dict[int, list[int]] = {}
        for pos, code in hits:
            line = text.count("\n", 0, pos) + 1
            by_line.setdefault(line, []).append(code)
        for line, codes in sorted(by_line.items()):
            code_hex = ", ".join(f"U+{c:04X}" for c in codes[:4])
            print(
                f"  {path}:{line}  invisible control char(s) ({code_hex}) — "
                "strip before committing."
            )
            findings += 1

    return findings


def discover(root: Path) -> list[Path]:
    return sorted(root.rglob("*.cedar"))


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description="Lint Cedar policy files for homoglyphs / invisible chars."
    )
    ap.add_argument(
        "paths", nargs="*", help="Explicit .cedar files (default: scan --root)."
    )
    ap.add_argument(
        "--root",
        default=".",
        help="Directory to recurse when no explicit paths given (default: cwd).",
    )
    args = ap.parse_args(argv[1:])

    if args.paths:
        paths = [Path(p) for p in args.paths]
    else:
        paths = discover(Path(args.root))

    if not paths:
        print("no .cedar files found")
        return 2

    total = 0
    for p in paths:
        total += lint_one(p)

    if total == 0:
        print(
            f"✓ {len(paths)} Cedar policy file(s) clean — "
            "ASCII identifiers, no invisible control chars."
        )
        return 0
    print()
    print(f"✗ {total} finding(s) across {len(paths)} file(s).")
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

# symbi-invis-strip

**Strip invisible / steganographic Unicode code points from strings before they reach a knowledge store, a journal, or a prompt.**

Zero dependencies. One public function. `no_std`-compatible in practice (uses only `String` / `str` / `char::from_u32` / `matches!`).

## Why

LLM agents that write to long-term memory are a ready channel for prompt injection. A reflector or planner can emit an innocent-looking `(subject, predicate, object)` triple whose `object` field contains zero-width characters, bidi overrides, the Unicode Tag block, or ASCII controls. Those payloads survive the roundtrip into storage and resurface when the **next** agent reads the store — turning the knowledge store into an instruction surface.

This crate is the sanitiser extracted from [`symbiont-karpathy-loop`](https://github.com/ThirdKeyAI/symbiont-karpathy-loop) (v6), where it caught real attacks:

- Multiple frontier models in a 9-model adversarial sweep attempted to smuggle payloads via `U+200B` (zero-width space) and the Unicode Tag block.
- **GPT-5 was caught using `U+007F` (DEL)** as an inter-word separator in stored triples — an attack surface the original filter missed. The gap was closed and ships in this crate.

Forensic audit of 128 stored procedures from the matching sweep: **0 residual invisible chars.**

## Usage

```rust
use symbi_invis_strip::sanitize_field;

// Zero-width space smuggled between words:
assert_eq!(sanitize_field("store\u{200B}knowledge"), "storeknowledge");

// Tag-block steganography:
assert_eq!(sanitize_field("safe\u{E0049}content"), "safecontent");

// Legitimate multilingual content survives unchanged:
assert_eq!(sanitize_field("Привет 世界 🎉"), "Привет 世界 🎉");
```

For callers that want to *detect* rather than strip (audit scripts, linters, logging):

```rust
use symbi_invis_strip::is_forbidden;

if is_forbidden('\u{007F}' as u32) {
    // ...
}
```

## What it strips

| Range | Notes |
|-------|-------|
| `U+0000..U+0008`, `U+000B..U+000C`, `U+000E..U+001F`, `U+007F` | ASCII C0 controls + DEL (excluding `\t`, `\n`, `\r`) |
| `U+0080..U+009F` | C1 control block |
| `U+200B..U+200F` | Zero-width + LRM/RLM |
| `U+202A..U+202E` | Bidi explicit directional overrides |
| `U+2060..U+206F` | Word joiner, invisible operators, bidi isolates, deprecated format |
| `U+FEFF` | BOM / ZWNBSP |
| `U+180E` | Mongolian vowel separator |
| `U+1D173..U+1D17A` | Musical notation invisible format |
| `U+FE00..U+FE0F` | Variation selectors (emoji-VS steg) |
| `U+E0100..U+E01EF` | Supplementary variation selectors |
| `U+E0000..U+E007F` | Unicode Tag block — the primary steganographic channel |

Legitimate printable content — ASCII printables, CJK, Cyrillic, diacritics, emoji proper, regular whitespace — roundtrips unchanged.

## Tests

12 unit tests including an **exhaustive fuzz**: every code point in every forbidden range is asserted stripped (200+ code points). `cargo test -p symbi-invis-strip` to run.

## Not in scope

- Homoglyph detection (Cyrillic `о` vs Latin `o`). That requires the Unicode confusables table; keep your tool-name matching ASCII-strict instead.
- NFC/NFKC normalisation. Would need a third-party crate and mostly helps with different problems than steganography.
- Rate-limiting, length caps, content-classification — orthogonal concerns that belong above this layer.

## Sync

The upstream [`symbiont-karpathy-loop`](https://github.com/ThirdKeyAI/symbiont-karpathy-loop) repo mirrors the forbidden-range list in two Python scripts (`scripts/audit-knowledge-stores.py`, `scripts/lint-cedar-policies.py`). If you vend this crate inside a larger system, keep any companion scripts in sync with this table.

## Licence

Apache-2.0, matching the rest of the Symbiont ecosystem.

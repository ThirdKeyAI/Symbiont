# ToolClad shared test vectors

`injection_vectors.json` and `scope_vectors.json` are copied verbatim from the
ToolClad reference repository (`tests/`), branch `security/audit-fixes` at
commit `dee5c9a`. They are the cross-runtime enforcement point for argument
hygiene, URL and egress policy, manifest loading, and scope matching.

`../toolclad_vectors.rs` runs every vector through Symbiont's validator,
manifest loader and scope engine and asserts the stated verdict. A vector on
which Symbiont deliberately differs is listed in that test by id with the
reason; there are no silent skips.

When updating, copy both files from a newer ToolClad commit, update the commit
above, and re-run `cargo test -p symbi-runtime --test toolclad_vectors`.

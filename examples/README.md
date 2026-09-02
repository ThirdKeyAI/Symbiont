# Examples

Each directory is self-contained. The first two run **offline** — no API key,
no model, no network — and are the fastest way to see what the runtime
actually enforces.

| Example | What it shows | Needs a key |
|---|---|---|
| [`policy-denial`](policy-denial) | The Cedar gate deciding allow/deny, and why the empty state is fail-closed | No |
| [`governed-tool`](governed-tool) | A `.clad.toml` contract refusing out-of-set and malformed arguments before a command exists | No |
| [`native-execution-example.rs`](native-execution-example.rs) | Driving the native sandbox runner from Rust | No |
| [`policies/`](policies) | Cedar policy samples referenced by the guides | — |

To run an *agent*, you need a model provider — a cloud key, or a local model
via `OPENAI_BASE_URL`. See [getting-started](../docs/getting-started.md).

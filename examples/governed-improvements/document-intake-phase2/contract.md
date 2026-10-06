# Phase 2 intake validation protocol

The fictional routing contract from [phase 1](../document-intake/contract.md)
remains unchanged. The phase 1 V1 proposal is frozen and copied byte-for-byte.
Twelve new synthetic inputs probe combined exceptions, malformed types, missing
evidence and adversarial notes. Expected answers never enter model inputs.
These are engineering holdout cases, not independent domain-owner validation.

Run three repetitions of each case with three arms:

- The original general baseline, through ordinary execution.
- An information-matched ordinary agent: the same baseline plus V1's complete
  instructions in agent-source comments. Information is matched; prompt wrapping
  differs from the governed candidate.
- The original V1 instructions through an explicit improvement trial.

Rotate arm order for each case and repetition. Retain one unscored warm-up with
the original complete-packet case. Use new invocation IDs for every scored run;
cached receipts are read only to obtain final output, never scored as new runs.

Each candidate repetition must independently pass all 12 exact outputs, at most
8,000 reported tokens per case, and zero policy denials. No tuning, threshold
changes or promotion occur during this experiment. A rejected evaluation must
also fail approval. All outcomes and signed evidence are retained, and the test
workflow is disabled afterward. Classify formatting failures diagnostically;
never normalize answers for the mandatory evaluator.

Report exact matches, formatting-only failures, wrong or ambiguous routing,
reported tokens, median latency and nearest-rank p95 latency. Latency measures
the first CLI execution, including process startup, inference and durable state;
offline verification and cached receipt reads are excluded. Repetitions on one
local model are correlated observations, not independent statistical samples.

```sh
python3 scripts/test-intake-phase2.py \
  --binary /absolute/path/to/symbi \
  --output /absolute/path/to/a/new/private/phase2-directory
```

Exit 0 means all candidate repetitions passed; 2 means the completed experiment
retained quality failures; 1 means a test or execution error. Output directories
include private runtime signing keys and must remain outside version control.
Tool approval and interruption are a separate action experiment, so quality
failures cannot be hidden by successful lifecycle checks.

## Benign action experiment

Run the separate receipt fixture after the quality comparison, to avoid competing
for model capacity during latency measurement:

```sh
python3 scripts/test-intake-actions.py \
  --binary /absolute/path/to/symbi \
  --output /absolute/path/to/a/new/private/action-directory
```

This requires Linux Landlock ABI 6+, user/mount/network namespaces, delegated
systemd user services, Python 3 and the installed local Qwen3 model. The driver
uses an isolated supervisor pool and confirms its cleanup. It does not weaken
ToolClad's private-network restrictions or require Docker.

A separate opt-in candidate calls only `record_receipt`, an enum-constrained
ToolClad command with mandatory approval. It writes one small synthetic JSON file
through a declared Landlock output grant. Cedar permits that exact tool action
and response action. This fixture is not the frozen routing candidate: that
candidate explicitly forbids tools. Passing the receipt acceptance case does
not waive any routing failure.

The test operator reads the complete terminal request and approves only the
expected tool and arguments using the exact request ID. This automated decision
tests approval enforcement, not independent human review. Test approval, denial,
omission of the relay, process interruption before approval, and interruption
after successful publication but before final inference. The local inference
proxy forwards real model requests unchanged and provides a deterministic pause
for the last interruption; it never fabricates model answers.

For interrupted invocations, inspect the signed journal, actual receipt and empty
worker/staging pool. Record a separate signed operator reconciliation, preserving
the original journal. Both unresolved and reconciled same-ID retries must do no
work. An explicit new ID must require new approval and complete normally. The
test does not establish exactly-once behavior for remote APIs or cover a crash
inside the file publication primitive. Those remain separate recovery concerns.

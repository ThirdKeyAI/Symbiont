# Held-action approval lifetime

The shared escalation queue gives each held action one deadline covering both
notification delivery and the operator's decision. Notification futures are
polled concurrently. A stalled notifier cannot prevent another surface from
resolving the request, and a ready decision cannot extend an elapsed deadline.

The waiting future owns the pending entry. Completion, expiry and cancellation
remove that entry synchronously and drop unfinished notification futures. Queue
snapshots show only unresolved, unexpired actions with a live receiver. Resolution
checks both a monotonic deadline and the displayed UTC expiry before accepting a
decision. An expired entry returns `Expired` (HTTP 410); after cleanup its ID
returns `NotFound` (HTTP 404). Neither response permits an effect.

Notifier implementations must use nonblocking, cancellation-safe I/O. Dropping
their futures must release input ownership; detached blocking terminal readers
do not meet this contract. Messages already delivered to an external surface may
remain visible, but their IDs no longer authorize a resolution after cleanup.

Approval authority remains in the existing governed gate. A successful queue
decision permits issuance of an exact, single-use receipt bound to the prepared
call, principal, session and trusted context. Policy evaluation, receipt checks
and required pre-effect audit still precede dispatch. Operator resolution alone
is not evidence that an effect executed. Verify the run's signed terminal audit
and effect records as described in [run audit](run-audit.md).

Ordinary and managed `symbi run` can enable the terminal relay explicitly:

```sh
symbi run reviewer --approval-terminal --approval-timeout 120
```

The timeout is in seconds, defaults to 120, and must be between 1 and 3600.
The run's own deadline still applies. Without the flag, approval-required calls
fail closed. An unavailable controlling terminal makes initialization fail before
inference. A pipe or a worker's stdin cannot supply approval authority.

The relay opens `/dev/tty` independently, requires the runtime's foreground
process group and uses nonblocking, cancellable I/O. It serializes prompts,
preserves terminal settings and discards stale input before each answer. The
complete held request appears as JSON with control characters and non-ASCII text
escaped. Requests exceeding the 64 KiB display limit are denied. Enter the exact
`approve <request-id>` shown after the JSON; any other answer denies the call.
Answers are limited to 128 bytes. No detached reader survives cancellation.

Approved queue decisions carry their resolution evidence into exact receipts.
The required pre-effect audit records the held ID, decision, timestamp and
approver alongside the bound invocation. Terminal identity is the local runtime
operator's effective UID, rather than the worker's UID or an environment label;
it identifies an OS account, not an independently verified individual. Optional
queue audit callbacks do not replace that required checkpoint. SDK sessions and
configured runtime surfaces retain the shared escalation queue.

Regression tests exercise stalled delivery, concurrent notification, deadline
races, late decisions, cancellation and invalid timeouts. Explicit Docker tests
use real ToolClad preparation, Cedar and signed run journals: exact approval
produces the authorized file effect, while denial, expiry and cancellation leave
no effect or pending request. They use synthetic local operator decisions.

## Shell approval review

In `symbi-shell`, Ctrl+G opens the Gate panel even while an agent turn is busy.
Select a request and press Enter to inspect its complete escaped JSON. Arrow and
Page Up/Down keys scroll through every argument, including long values. Press
`a` or `d` from that review to resolve that exact request. A list row alone does
not authorize an action. Requests over the display limit cannot be approved from
this view.

Refreshes preserve the selected request by ID and retain the reviewed snapshot.
A changed, expired or removed request invalidates the review. Unsupported request
fields, duplicate IDs and failed queue reads disable review rather than presenting
an ambiguous partial response. Changing remote connections discards prior reviews
and is refused while a resolution is in flight.

The panel reports actual resolution success or failure and keeps a request visible
while its result is pending. A timeout reports an unknown outcome; it must not be
interpreted as proof that an action was denied. Reopen a fresh review after an
error. The in-process queue takes precedence when configured; otherwise the panel
uses the attached runtime's authenticated approval API. Shell file/command effects
and protected per-turn audit use the [governed shell boundary](shell-containment.md).

## Chat approval review

Configured channel approvers can request `/symbi gate show <id>`. The reply and
held-action notification use the same complete JSON review, including the exact
invocation and expiry. Unicode, terminal controls and active Markdown characters
are escaped without changing the decoded request. A local 8 KiB budget includes
the review and decision commands; oversized requests must be reviewed through the
terminal, Gate panel or authenticated API. They cannot be approved in chat.

Copy the complete `/symbi gate approve <id> <review-digest>` command attached to
the review. The digest binds the displayed request and expiry; an ID alone, a digest
from another request, or extra arguments cannot approve it. The digest is public
and does not prove a person read the message. Platform/channel approver allowlists
remain the authority. `/symbi gate deny <id>` requires no review. Expiry and one-use
receipt checks still apply at resolution and dispatch. Required signed resolution
records identify the platform, workspace, channel and sender and include the review
digest for approvals.

Slack slash commands normalize `gate` as a control command before agent routing.
Slack callbacks always require a nonempty signing secret and a valid signature
covering the exact received body bytes and timestamp. The adapter refuses startup
without that secret; environment labels and the former unsigned-callback override
cannot disable authentication. Configure the app signing secret before enabling
Slack. Other platform authentication and broader deployment trust remain separate
review requirements.

## Managed admission review

A managed agent can set metadata `human_approval = true`. The terminal then
reviews the fixed `claude_code` launch before any worker or provider request.
The request includes the task, argv, explicit environment, selected tools,
inference settings and both worker and backend sandbox profiles. Supported
inline source rules run before the review; a denied launch creates no held
request. The source hashes remain fixed if the file changes while review is
pending. Tool approvals occur separately after launch. See
[managed CLI containment](managed-cli-containment.md).

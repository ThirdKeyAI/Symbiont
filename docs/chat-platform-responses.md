# Chat-platform responses

The Slack, Teams and Mattermost adapters started by `symbi up` use registered,
bounded conversational responses. A requested agent must resolve to one startup
source, either by filename stem or declared name. Aliases use the declared
principal and selected declaration. Unknown or ambiguous names, unsupported
executable definitions and alternate executors are refused before inference.
Only the selected definition enters the prompt. Missing provider configuration
returns an invocation error.

Chat sources use confined startup project reads: no symlinks, hardlinks, parent
traversal or private paths. Each source is limited to 1 MiB; discovery inspects
at most 1,024 entries and retains at most 16 MiB. Changes require a restart.
Input is limited to 64 KiB, output requests to 4,096 tokens, and the selected
source can tighten the 120-second run deadline. This route cannot dispatch tools.

## Policy covers the actual reply

Platform formatting happens before authorization. Cedar receives the complete
prepared contract at `context.invocation.resolved.response_delivery`, containing
`destination`, `message` and `transport`. The destination includes platform, workspace,
channel, thread, sender and inbound message IDs. The message includes text,
blocks/cards, ephemeral flags and routing metadata. The transport contains the
actual HTTP `method`, canonical `url` and JSON `body`, without credentials. The
adapter uses the same preparation routine for authorization and sending; the
delivery owner refuses a changed transport contract. The startup context also
contains `response_destination` and the registered source identity.
Optional JSON fields with null values are absent in Cedar's record view; use
`has` before reading them. Audit records and fingerprints retain the original
JSON. Null array elements are refused because Cedar cannot represent them.

Chat adapters retain shared policies and the `policies/coordinator/` surface.
For example, an operator can permit replies only in a selected Slack channel:

```cedar
permit(principal, action == Action::"respond", resource)
when {
    context.invocation.resolved.response_delivery.destination.platform == "slack" &&
    context.invocation.resolved.response_delivery.message.channel_id == "C123"
};
```

The gate prepares and evaluates replacements again if policy changes the response.
The immutable authorized contract is then passed to the originating adapter.
Neither model text nor caller input can select another adapter. Missing policies
retain the framework's default text-response permission; malformed configured
policies deny all actions.

## Authentication and transport

Teams callbacks require a valid RS256 signature, the configured app audience,
Bot Framework issuer, an explicit valid `nbf`/`exp` interval (with five minutes
of clock skew), and a signing key endorsed for `msteams`. The activity must name
that channel and its `serviceUrl` must exactly match the signed token claim.
Missing, insecure or mismatched destinations are rejected before inference.
`skip_jwks_verification = true` is always refused, including local development.
This follows Microsoft's [Bot Connector authentication requirements](https://learn.microsoft.com/en-us/azure/bot-service/rest-api/bot-framework-rest-connector-authentication?view=azure-bot-service-4.0).

OpenID metadata comes from the fixed Bot Framework HTTPS endpoint. Its issuer
and advertised signing algorithm are checked; its key URL must stay on
`login.botframework.com:443`. Duplicate key IDs and keys with incompatible
type, algorithm, use or channel endorsement are refused. Tokens are limited to
16 KiB and metadata/key responses to 1 MiB each.

Teams replies require explicit service URL and activity metadata. Conversation
and activity IDs are encoded as individual URL path segments; they cannot
replace the path or inject a query. The tenant setting cannot inject an OAuth
URL path. Slack uses its fixed HTTPS API endpoints. Mattermost retains the
operator-configured HTTP/HTTPS base URL, including local installations. Base
URLs cannot contain credentials, queries, fragments, whitespace or backslashes.

All three platforms' API and authentication clients refuse redirects, use a
five-second connect timeout and a 15-second request timeout, and require a
successful HTTP status. Reply and token JSON responses are limited to 64 KiB,
including streamed bodies without a Content-Length header. Error bodies are
excluded from diagnostics. Malformed replies and missing receipt IDs cannot
report success. Configure the final endpoint directly when a service moves;
a redirect is recorded as a delivery error and is not retried automatically.

These clients trust operator network/proxy configuration, fixed platform
authorities and the signed Bot Framework destination. This does not establish
a separate DNS/IP isolation boundary or a common outer containment lab. SDK
callers supplying Teams destinations remain responsible for authenticating
their origin. Custom adapters must implement `prepare_response` to participate
in the shipping governed delivery contract; its default refuses preparation.

## Required audit and delivery outcomes

The same protected per-run journal covers source identity, typed inference,
the exact response policy decision, delivery and terminal outcome. The required
`PolicyEvaluated` record contains the formatted contract. A signed
`ResponseDeliveryStarted` record binds its fingerprint and request hash before
the send. `ResponseDeliveryFinished` records the actual adapter receipt or error
before successful run completion. Receipts must confirm the expected platform
and channel and include a nonempty platform message ID.

Storage initialization or pre-send write failures prevent the corresponding
inference or send. A negative receipt, destination mismatch, timeout, or missing
required receipt/terminal write cannot report successful completion. An audit
write can fail after a platform has accepted the message: that effect cannot be
undone. Inspect the signed evidence and platform state before considering a retry.

Cancellation retains the run owner and terminal audit. A delivery start without
a finished receipt leaves delivery unconfirmed, including when cancellation
interrupts an HTTP request. Platform callback acknowledgements establish callback
receipt, not agent completion. Once an asynchronous callback is accepted, closing
its HTTP connection does not cancel the background response. Abrupt process loss
can leave an incomplete journal.

The manager reports delivery failures as errors and invokes the delivery owner
once. It does not publish the model output a second time. Runtime logs identify
the protected audit reference and completion status. The visible message layout
stays the same; agent labels now consistently use the declared name across aliases.

The standalone channel-adapter SDK's default `AgentInvoker` implementation leaves
governance to the embedding application. The protected lifecycle described here
is wired by the shipping `symbi up` invoker. Operator control-command replies and
direct manager sends have separate call paths; they are not model response runs.

The shipping check uses authenticated callbacks, local inference and a local TLS
endpoint behind a CONNECT fixture. It never forwards traffic to Slack:

```bash
python3 scripts/test-chat-delivery.py --binary target/debug/symbi --report /tmp/chat-delivery.json
```

The transport fixture adds real RSA-signed Teams callbacks, invalid signature
and claim cases, untrusted/redirecting key discovery, final-URL Cedar denial,
and successful, redirecting and oversized replies for all three platforms.
It checks the observed URL and JSON body against the signed authorized contract:

```bash
python3 scripts/test-platform-transports.py --binary target/debug/symbi --report /tmp/platform-transports.json
```

See [protected run audit](run-audit.md) for key pinning and journal verification.

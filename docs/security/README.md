# Security controls and evidence

[controls.json](controls.json) is the versioned catalogue of scoped security
properties. An implemented control is not automatically validated. Test source
links identify ways to exercise the control; only a retained qualification report
with its digest can support a validation claim.

Each control records its threat, invariant, owner, execution routes, backends,
implementation references and limitations. Implementation states are
`implemented`, `partial`, and `planned`. Validation states are `validated`,
`partial`, `untested`, and `failed`. A fully validated claim requires passing
qualifications for every listed route/backend combination. Prefer separate
controls where a cross-product would imply combinations that do not exist.

Run `python3 scripts/check-security-controls.py` from the source checkout.
The check refuses unknown schema fields, missing source references, altered
reports and validation claims broader than their attached evidence. It checks
evidence structure and integrity; it does not execute tests or certify that
a report's assertions are true. Independently verify the signed journals and
external-effect receipts associated with a qualification.

Qualification reports identify the source revision, source-file digest, binary
digest, configuration digest, driver digest, timestamp, platform and individual
checks. Dirty development builds require their source digest as well as the base
revision. Rebuilding or changing configuration requires new evidence. A retained
result describes that build and environment, not every future release.

The current new feature is [optional outbound capabilities](../outbound-capabilities.md).
Its runtime HTTP checks do not qualify a container, VM or Linux kernel boundary.
Managed inference, ordinary provider transports, channel delivery, remote buses,
webhooks and browser adapters must be assessed separately. Browser capability
execution is currently unavailable. Full process hibernation and durable logical
workflow resume are not supplied by this feature.

The [3.0 candidate workflow report](../evidence/2026-10-27/release-3.0-outbound/README.md)
retains 16 passing installed-CLI cases for the current candidate source, including
encoded path-parameter refusal and reflected-header evidence checks. It binds
the uncommitted candidate's source digest and debug binary; it does not qualify
the eventual release-profile binary. The
[2026-10-27 review report](../evidence/2026-10-27/outbound-review/README.md)
records the preceding review. The
[earlier follow-up report](../evidence/2026-10-27/outbound-capabilities/README.md)
records the integration before this review; the
[2026-10-26 report](../evidence/2026-10-26/outbound-capabilities/README.md)
records the initial build. The four associated controls remain
partially validated because other listed routes lack equivalent retained
qualifications. Worker isolation remains unqualified by this report.

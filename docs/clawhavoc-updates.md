# Signed ClawHavoc updates

The updater is opt-in. There is no default feed or shipped publisher key yet.
The planned endpoint is `https://updates.symbiont.dev/clawhavoc/pack.json`.
A locally pinned P-256 public key authenticates downloaded packs; a key downloaded
alongside a pack is never automatically trusted. Scanning never uses the network.

## Operator setup

Obtain the publisher public key through an independently authenticated channel.
Once the feed is published:

```sh
symbi skills rules init --url https://updates.symbiont.dev/clawhavoc/pack.json --public-key publisher.pem
symbi skills rules update
symbi skills rules status
symbi skills scan ./my-skill --rules-dir "$HOME/.symbiont/clawhavoc" --json
```

`init` stores the URL and public key in `feed.json` and refuses to overwrite an
existing configuration. `--store DIR` selects another operator-owned store on
init/update/status. Keep the store and all ancestors outside skill/worker-writable
paths. Protect `feed.json` as authority, not as a cache. HTTPS redirects are rejected;
configure the final URL. Downloads have a 30-second timeout and a 1 MiB limit.

Pack rules add to built-in rules and custom rules. They cannot remove or override
built-in rules. The verified pack digest and version are included in scan policy
identity. Missing, malformed, wrongly signed, incompatible or expired selected
packs fail the scan. Without `--rules-dir`, scans use the built-in policy only.
SDK loaders can opt in with `SkillsConfig.rule_pack_dir`; recreate a loader after
an update to pick up its new policy. Existing scanners refuse an expired pack.

Updates verify the entire signed envelope before touching active state. Versions
must increase; re-fetching the identical payload at the same version is idempotent.
A same-version changed payload and lower versions are rejected. Expired active
packs can be replaced by newer fresh packs. A corrupt active pack requires
operator repair; it is never silently discarded to reset the version floor.
Expiry limits replay/freeze attacks to at most 30 days from issuance and relies
on a trustworthy local clock. Publisher key rotation is an explicit operator
reconfiguration; it cannot be requested by a downloaded pack.

Writers take an exclusive file lock. A flushed temporary file is atomically renamed
over `active.json`; the previous verified envelope is retained in `previous.json`.
On Unix the directory is also synced. A crash before activation leaves the old
complete pack active; after activation the new complete pack is active. An update
error leaves the prior active pack usable unless activation already succeeded and
a subsequent directory sync failed; retry safely in that case. Do not restore an
older pack as a normal rollback: publish a higher version containing corrected
rules. Local administrator changes/deletion of the store are outside antirollback
protection. Never restore expired rules as a way to bypass freshness checks.

## Publishing

Keep the signing private key offline or in your existing release signing system.
No private key is created, copied from AgentSniff, or committed by this feature.
For a new publisher, generate an ES256 key with OpenSSL outside the repository:

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out /secure/clawhavoc.private.pem
openssl pkey -in /secure/clawhavoc.private.pem -pubout -out publisher.pem
symbi skills rules sign --input rules.json --key /secure/clawhavoc.private.pem --output pack.json
```

The unsigned input has exactly these fields (use current Unix seconds, increasing
versions, and an expiry at most 30 days after issuance):

```json
{
  "format": "symbi-clawhavoc-pack-v1",
  "version": 1,
  "semantics_version": "2",
  "issued_at": 0,
  "expires_at": 0,
  "rules": [
    {
      "id": "example-deny",
      "pattern": "synthetic-secret",
      "severity": "High",
      "message": "Synthetic example; replace with reviewed detection",
      "source": "Synthetic example, not production threat intelligence"
    }
  ]
}
```

The zero timestamps deliberately make this example unpublishable until filled in.
Allowed severities are `Critical`, `High`, `Medium`, `Warning`, and `Info`.
Each pack contains 1–256 rules with unique ASCII IDs, bounded regexes and messages,
and nonempty provenance. Unknown fields and malformed regexes reject the complete
pack. Add regression fixtures before publishing new detection logic.

The envelope is `{ "payload": <RulePack>, "signature": "<base64 DER>" }`.
ES256 signs UTF-8 compact serde JSON for `["symbi-clawhavoc-pack-v1", payload]`.
Payload field order is format, version, semantics_version, issued_at, expires_at,
rules. Rule field order is id, pattern, severity, message, source. Array order is
significant. The signature is ECDSA-P256-SHA256, using SchemaPin's `sign_data` /
`verify_signature` (one SHA-256 pass), not AgentSniff's double-hashed wire format.
The digest is SHA-256 of those same signing bytes, independent of signature bytes.

## GitHub Pages hosting

Symbiont's existing Pages site serves `docs.symbiont.dev`. Use a separate repository
[`ThirdKeyAI/symbiont-updates`](https://github.com/ThirdKeyAI/symbiont-updates); do not replace the documentation site.
That repository has been provisioned with a landing page and the custom domain;
a production signing key and rule pack are still required.
The `deploy/clawhavoc-feed` template provides a Pages workflow for that repository.
Copy its workflow to `.github/workflows/pages.yml` and its `site` directory to the
repository root. Add the reviewed `site/clawhavoc/pack.json` and public key at
`site/clawhavoc/publisher.pem`. Only public artifacts belong in `site`.

Configure that repository's Pages source to GitHub Actions and set its custom
domain to `updates.symbiont.dev` **before** adding DNS. Then create:

| Type | Name | Target |
| --- | --- | --- |
| CNAME | updates | thirdkeyai.github.io |

Enable Enforce HTTPS once GitHub provisions the certificate. The workflow's CNAME
file is informational; Actions-based Pages still needs the repository custom-domain
setting. See [GitHub's custom-domain instructions](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site).
Until DNS is ready, configure the final HTTPS GitHub Pages URL explicitly. After
switching to a custom domain, reconfigure the stored feed URL instead of relying
on redirects. Publishing a file does not establish client trust: distribute and
pin its public key separately. The template itself does not provision DNS, create a
repository, generate production keys, or publish an initial production ruleset.

# Verify the 3.0 release

This guide targets v3.0.0. Run these checks only after publication is complete and all named assets are present. The new outbound contract and registered Landlock selection change the Rust API; read the migration section in [outbound capabilities](outbound-capabilities.md).

Resolve the public release commit from the trusted repository's `v3.0.0` tag,
as shown below. Do not take the expected commit from an unverified downloaded
source record. Gitea retains the complete source history; GitHub contains the
filtered OSS history, so their commit IDs differ. Release signatures and crate
provenance refer to the public commit.

## Choose an archive

| Platform | Target | Archive suffix |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

For example, the Linux x86_64 archive is
`symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz`. Intel macOS has no prebuilt archive.
Use source installation there. Platform binaries do not imply Linux-only Landlock
support on macOS or Windows.

Protected run journals and their dependent execution routes currently require
Unix. Windows binaries cover portable CLI functionality; protected scheduling,
durable chat admission and governed improvements refuse unsupported operation.
A successful Windows compilation does not establish equivalent containment.

## Verify before extraction

The following commands use Bash, GitHub CLI and Cosign 3.1+. Run them in an empty
directory. Set `ARCHIVE` to your platform's filename. Download its `.sig` and `.pem`
alongside the signed checksum manifest and `release-source.json`:

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v3.0.0 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz
gh release download v3.0.0 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

This release publishes detached signatures and certificates. Convert the checksum
signature to a bundle, retaining its transparency-log evidence, then verify the
expected workflow identity and exact source commit:

```bash
cosign bundle create --artifact checksums.txt \
  --signature checksums.txt.sig --certificate checksums.txt.pem \
  --out checksums.sigstore.json
cosign verify-blob --bundle checksums.sigstore.json \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/release-binaries.yml@refs/tags/v3.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA" \
  checksums.txt
```

Do not ignore transparency-log checks or broaden the expected identity to make a
failed verification pass. Then verify the selected archive and source record:

```bash
awk -v archive="$ARCHIVE" '$2 == archive || $2 == "release-source.json"' \
  checksums.txt > selected-checksums.txt
test "$(wc -l < selected-checksums.txt)" -eq 2
# Linux:
sha256sum --check selected-checksums.txt
# macOS alternative:
# shasum -a 256 --check selected-checksums.txt
cat release-source.json
```

The source record must contain tag `v3.0.0` and commit
equal to `$RELEASE_SHA`. Extract and run the binary only after
verification succeeds. For a separate archive-signature check, repeat the bundle
creation and verification commands with `"$ARCHIVE"`, `"$ARCHIVE.sig"` and
`"$ARCHIVE.pem"`. Windows users can run the verification commands in Git Bash or
use equivalent checksum tooling, then extract the ZIP.

The [repository installer](https://github.com/ThirdKeyAI/Symbiont/blob/main/scripts/install.sh)
automatically verifies the archive's checksum from the HTTPS release manifest. It
does not perform the independent Sigstore identity check above. It installs the
latest version into `~/.symbi/bin`; use a manual versioned download to pin 3.0.0.

## Containers and crates

Versioned images are `ghcr.io/thirdkeyai/symbi:3.0.0` and
`ghcr.io/thirdkeyai/symbi-a2ui:3.0.0`. Verify each image before deployment:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:3.0.0 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v3.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

Verify the exact non-yanked crate versions and archive checksums. New packages must bind the public release commit in `.cargo_vcs_info.json`; unchanged packages retain their earlier provenance.

| Crates | Version |
| --- | --- |
| `symbi`, `symbi-dsl`, `symbi-runtime`, `repl-core`, `repl-cli`, `repl-lsp`, `symbi-shell` | 3.0.0 |
| `repl-proto`, `symbi-session`, `symbi-sandbox-guest`, `symbi-sandbox-supervisor` | 2.0.1 |
| `symbi-approval-relay` | 0.1.1 |
| `symbi-channel-adapter` | 0.1.6 |
| `symbi-invis-strip` | 0.3.0 |

See [release validation](release-3.0-validation.md) for the tests and their limits,
and [migration](migration-2.0.md) before upgrading an existing deployment.

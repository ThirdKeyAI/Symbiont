# Verify the 2.0 release

This guide targets the [v2.0.1 release](https://github.com/ThirdKeyAI/Symbiont/releases/tag/v2.0.1).
Use the archive steps only after its publication is complete and the listed
assets are present. The 2.0.0 crates and containers remain published, but a Windows
compile failure prevented its combined binary upload and Homebrew update. The
2.0.1 patch repairs that build and the repository installer.

Resolve the public release commit from the trusted repository's `v2.0.1` tag,
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
`symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz`. Intel macOS has no prebuilt archive.
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
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v2.0.1 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz
gh release download v2.0.1 --repo ThirdKeyAI/Symbiont \
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
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/release-binaries.yml@refs/tags/v2.0.1' \
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

The source record must contain tag `v2.0.1` and commit
equal to `$RELEASE_SHA`. Extract and run the binary only after
verification succeeds. For a separate archive-signature check, repeat the bundle
creation and verification commands with `"$ARCHIVE"`, `"$ARCHIVE.sig"` and
`"$ARCHIVE.pem"`. Windows users can run the verification commands in Git Bash or
use equivalent checksum tooling, then extract the ZIP.

The [repository installer](https://github.com/ThirdKeyAI/Symbiont/blob/main/scripts/install.sh)
automatically verifies the archive's checksum from the HTTPS release manifest. It
does not perform the independent Sigstore identity check above. It installs the
latest version into `~/.symbi/bin`; use a manual versioned download to pin 2.0.1.

## Containers and crates

Versioned images are `ghcr.io/thirdkeyai/symbi:2.0.1` and
`ghcr.io/thirdkeyai/symbi-a2ui:2.0.1`. Verify each image before deployment:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:2.0.1 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v2.0.1' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

For crates.io, check the exact package version and its non-yanked status. The eleven
new 2.0.1 crates must have registry checksums matching their downloaded archives and
`.cargo_vcs_info.json` pointing to the public release commit. The unchanged helper
versions are `symbi-approval-relay` 0.1.1, `symbi-channel-adapter` 0.1.6 and
`symbi-invis-strip` 0.3.0; they retain their earlier source commits.

See [release validation](release-2.0-validation.md) for the tests and their limits,
and [migration](migration-2.0.md) before upgrading an existing deployment.

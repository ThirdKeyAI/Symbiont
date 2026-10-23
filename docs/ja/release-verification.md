# 2.0 リリースの検証

このガイドは [v2.0.1 リリース](https://github.com/ThirdKeyAI/Symbiont/releases/tag/v2.0.1)を対象とします。アーカイブの検証は、公開が完了し、記載されたファイルが揃ってから実施してください。2.0.0 の crate とコンテナーは公開済みですが、Windows のコンパイルエラーにより、全プラットフォームのバイナリアップロードと Homebrew の更新が中断されました。2.0.1 はこのビルドエラーとリポジトリのインストーラーを修正します。

以下の手順で、信頼するリポジトリの `v2.0.1` タグから公開コミットを取得してください。未検証のダウンロード済みソース記録から、期待するコミットを取得しないでください。Gitea は完全な履歴、GitHub は OSS のみを抽出した履歴を保持するため、コミット ID は異なります。署名とパッケージの来歴は公開コミットに対応します。

## アーカイブの選択

| プラットフォーム | ターゲット | 拡張子 |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

例：`symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz`。Intel Mac 向けのビルド済みアーカイブはありません。ソースからインストールしてください。macOS や Windows のバイナリは Landlock による隔離を提供しません。

保護された実行ジャーナルと、それに依存する実行経路には、現在 Unix が必要です。Windows バイナリは移植可能な CLI 機能を提供しますが、保護されたスケジュール実行、永続的なチャット受付、管理された改善は、未対応の操作を拒否します。Windows でコンパイルが成功しても、同等の隔離が保証されるわけではありません。

## 展開前の検証

以下のコマンドには Bash、GitHub CLI、Cosign 3.1+ が必要です。空のディレクトリで実行し、`ARCHIVE` を対象プラットフォームのファイル名に設定してください。`.sig`、`.pem`、署名付きチェックサム一覧、`release-source.json` もダウンロードします。

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v2.0.1 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz
gh release download v2.0.1 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

このリリースでは署名と証明書を別々に提供します。透明性ログの証拠を保持したバンドルを作成し、期待するワークフローの ID と正確なソースコミットを検証します。

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

検証を通すために透明性ログの確認を省略したり、許可する ID を広げたりしないでください。続いてアーカイブとソース情報のチェックサムを確認します。

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

ソース情報にはタグ `v2.0.1` とコミット `$RELEASE_SHA` が必要です。検証が成功してから展開・実行してください。アーカイブの署名を個別に検証するには、同じバンドル作成・検証コマンドを `"$ARCHIVE"`、`"$ARCHIVE.sig"`、`"$ARCHIVE.pem"` に対して実行します。Windows では Git Bash または同等の検証ツールを使用し、その後 ZIP を展開できます。

リポジトリのインストーラーは、展開前に HTTPS のリリース一覧に記載された SHA-256 を `sha256sum` または `shasum` で検証します。最新版を `~/.symbi/bin` にインストールしますが、独立した Sigstore ID 検証は行いません。インストール先のルートを変える場合は `bash` プロセスに `SYMBI_INSTALL_DIR` を設定してください。バージョンを固定する場合は手動でダウンロードします。

## コンテナーとクレート

バージョン付きイメージは `ghcr.io/thirdkeyai/symbi:2.0.1` と `ghcr.io/thirdkeyai/symbi-a2ui:2.0.1` です。デプロイ前に両方を検証してください。

```bash
cosign verify ghcr.io/thirdkeyai/symbi:2.0.1 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v2.0.1' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

crates.io では正確なバージョンと、取り下げられていないことを確認します。新しい 11 個の 2.0.1 クレートは、ダウンロードしたアーカイブのチェックサムがレジストリと一致し、`.cargo_vcs_info.json` が公開コミットを示す必要があります。変更のない `symbi-approval-relay` 0.1.1、`symbi-channel-adapter` 0.1.6、`symbi-invis-strip` 0.3.0 は以前のコミットを保持します。

テストとその限界は[リリース検証結果](/release-2.0-validation/)を参照してください。既存環境を更新する前に[移行ガイド](/migration-2.0/)を確認してください（どちらも英語）。

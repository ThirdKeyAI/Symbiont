# 验证 3.0 版本

本指南适用于 v3.0.0。请在发布完成且所列文件齐全后执行验证。新的出站访问控制与已注册代理的 Landlock 选择改变了 Rust API；请阅读[出站访问控制](/zh-cn/outbound-capabilities)中的迁移说明。

请按以下步骤，从可信仓库的 `v3.0.0` 标签获取公开提交。不要从未经验证的已下载源码记录中获取预期提交。Gitea 保留完整历史，GitHub 只包含筛选后的 OSS 历史，因此两者的提交 ID 不同。发布签名和软件包来源对应公开提交。

## 选择压缩包

| 平台 | 目标 | 扩展名 |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

例如：`symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz`。Intel Mac 没有预编译压缩包，请从源代码安装。macOS 和 Windows 二进制文件不提供 Landlock 隔离。

受保护的运行日志及依赖这些日志的执行路径目前需要 Unix。Windows 二进制文件提供可移植的 CLI 功能；受保护的调度执行、持久化聊天准入和受治理的改进会拒绝不支持的操作。Windows 编译成功并不代表具有同等的隔离能力。

## 解压前验证

以下命令需要 Bash、GitHub CLI 和 Cosign 3.1+。请在空目录中运行，并将 `ARCHIVE` 设为对应平台的文件名。同时下载 `.sig`、`.pem`、带签名的校验和清单以及 `release-source.json`：

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v3.0.0 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz
gh release download v3.0.0 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

本版本分别提供签名和证书。将其转换为保留透明日志证据的验证包，然后验证预期的工作流身份和准确的源代码提交：

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

不要为绕过验证失败而跳过透明日志检查或扩大允许的身份范围。接着验证压缩包和来源记录的校验和：

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

来源记录必须包含标签 `v3.0.0` 和提交 `$RELEASE_SHA`。验证成功后才能解压并运行。若要单独检查压缩包签名，请对 `"$ARCHIVE"`、`"$ARCHIVE.sig"` 和 `"$ARCHIVE.pem"` 重复执行验证包创建和验证命令。Windows 用户可通过 Git Bash 或等效校验工具验证，然后解压 ZIP。

仓库安装脚本使用 `sha256sum` 或 `shasum`，在解压前验证 HTTPS 发布清单中的 SHA-256 校验和。它将最新版本安装到 `~/.symbi/bin`，但不执行独立的 Sigstore 身份验证。若需更改安装根目录，请为 `bash` 进程设置 `SYMBI_INSTALL_DIR`。要固定版本，请手动下载指定版本。

## 容器与 crate

带版本号的镜像为 `ghcr.io/thirdkeyai/symbi:3.0.0` 和 `ghcr.io/thirdkeyai/symbi-a2ui:3.0.0`。部署前请分别验证：

```bash
cosign verify ghcr.io/thirdkeyai/symbi:3.0.0 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v3.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

请验证准确且未撤回的 crate 版本和压缩包校验和。新包的 `.cargo_vcs_info.json` 必须指向公开发布提交；未更改的包保留此前的来源信息。

| Crates | Version |
| --- | --- |
| `symbi`, `symbi-dsl`, `symbi-runtime`, `repl-core`, `repl-cli`, `repl-lsp`, `symbi-shell` | 3.0.0 |
| `repl-proto`, `symbi-session`, `symbi-sandbox-guest`, `symbi-sandbox-supervisor` | 2.0.1 |
| `symbi-approval-relay` | 0.1.1 |
| `symbi-channel-adapter` | 0.1.6 |
| `symbi-invis-strip` | 0.3.0 |

测试及其适用范围见[发布验证结果](/release-3.0-validation/)。更新现有部署前请阅读[迁移指南](/migration-2.0/)（均为英文）。

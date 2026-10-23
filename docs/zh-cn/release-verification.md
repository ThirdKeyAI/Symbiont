# 验证 2.0 版本

从 [v2.0.0 发布页面](https://github.com/ThirdKeyAI/Symbiont/releases/tag/v2.0.0)下载二进制文件。公开提交为 `21471d3a9dea8ccf24d9248ff9338b35a79215e5`。Gitea 保留完整历史，GitHub 仅包含筛选后的 OSS 内容，因此两者的提交 ID 不同。发布签名和软件包来源记录对应上述公开提交。

## 选择压缩包

| 平台 | 目标 | 扩展名 |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

例如：`symbi-v2.0.0-x86_64-unknown-linux-gnu.tar.gz`。Intel Mac 没有预编译压缩包，请从源代码安装。macOS 和 Windows 二进制文件不提供 Landlock 隔离。

## 解压前验证

以下命令需要 Bash、GitHub CLI 和 Cosign 3.1+。请在空目录中运行，并将 `ARCHIVE` 设为对应平台的文件名。同时下载 `.sig`、`.pem`、带签名的校验和清单以及 `release-source.json`：

```bash
ARCHIVE=symbi-v2.0.0-x86_64-unknown-linux-gnu.tar.gz
gh release download v2.0.0 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

本版本分别提供签名和证书。将其转换为保留透明日志证据的验证包，然后验证预期的工作流身份和准确的源代码提交：

```bash
cosign bundle create --artifact checksums.txt \
  --signature checksums.txt.sig --certificate checksums.txt.pem \
  --out checksums.sigstore.json
cosign verify-blob --bundle checksums.sigstore.json \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/release-binaries.yml@refs/tags/v2.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha 21471d3a9dea8ccf24d9248ff9338b35a79215e5 \
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

来源记录必须包含标签 `v2.0.0` 和提交 `21471d3a9dea8ccf24d9248ff9338b35a79215e5`。验证成功后才能解压并运行。若要单独检查压缩包签名，请对 `"$ARCHIVE"`、`"$ARCHIVE.sig"` 和 `"$ARCHIVE.pem"` 重复执行验证包创建和验证命令。Windows 用户可通过 Git Bash 或等效校验工具验证，然后解压 ZIP。

仓库安装脚本使用 `sha256sum` 或 `shasum`，在解压前验证 HTTPS 发布清单中的 SHA-256 校验和。它将最新版本安装到 `~/.symbi/bin`，但不执行独立的 Sigstore 身份验证。若需更改安装根目录，请为 `bash` 进程设置 `SYMBI_INSTALL_DIR`。要固定版本，请手动下载指定版本。

## 容器与 crate

带版本号的镜像为 `ghcr.io/thirdkeyai/symbi:2.0.0` 和 `ghcr.io/thirdkeyai/symbi-a2ui:2.0.0`。部署前请分别验证：

```bash
cosign verify ghcr.io/thirdkeyai/symbi:2.0.0 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v2.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha 21471d3a9dea8ccf24d9248ff9338b35a79215e5
```

在 crates.io 中检查准确版本及其是否被撤回。11 个新增的 2.0.0 crate，其下载文件校验和必须与注册表一致，且 `.cargo_vcs_info.json` 必须指向公开发布提交。未变更的辅助包为 `symbi-approval-relay` 0.1.1、`symbi-channel-adapter` 0.1.6 和 `symbi-invis-strip` 0.3.0，它们保留原有的来源提交。

测试及其适用范围见[发布验证结果](/release-2.0-validation/)。更新现有部署前请阅读[迁移指南](/migration-2.0/)（均为英文）。

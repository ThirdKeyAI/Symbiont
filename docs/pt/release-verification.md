# Verificar a versão 2.0

Baixe os binários da [versão v2.0.0](https://github.com/ThirdKeyAI/Symbiont/releases/tag/v2.0.0). O commit público é `21471d3a9dea8ccf24d9248ff9338b35a79215e5`. O Gitea mantém o histórico completo; o GitHub contém a exportação OSS filtrada. Por isso os identificadores diferem: as assinaturas e a procedência dos pacotes se referem ao commit público.

## Escolher um arquivo

| Plataforma | Destino | Extensão |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Exemplo: `symbi-v2.0.0-x86_64-unknown-linux-gnu.tar.gz`. Não há arquivo pré-compilado para macOS Intel; instale a partir do código-fonte. Os binários de macOS e Windows não oferecem isolamento Landlock.

## Verificar antes de extrair

Estes comandos exigem Bash, GitHub CLI e Cosign 3.1+. Use um diretório vazio e defina `ARCHIVE` com o nome correspondente à sua plataforma. Baixe também `.sig`, `.pem`, o manifesto assinado de somas de verificação e `release-source.json`:

```bash
ARCHIVE=symbi-v2.0.0-x86_64-unknown-linux-gnu.tar.gz
gh release download v2.0.0 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

A versão publica assinaturas e certificados separados. Converta-os em um bundle com a evidência do registro de transparência e verifique a identidade esperada do workflow e o commit exato:

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

Não ignore as verificações do registro de transparência nem amplie a identidade permitida para contornar uma falha. Depois verifique o arquivo e o registro de origem:

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

O registro deve conter a tag `v2.0.0` e o commit `21471d3a9dea8ccf24d9248ff9338b35a79215e5`. Extraia e execute o binário somente após a verificação bem-sucedida. Para verificar a assinatura do arquivo separadamente, repita os comandos com `"$ARCHIVE"`, `"$ARCHIVE.sig"` e `"$ARCHIVE.pem"`. No Windows, use Git Bash ou ferramentas equivalentes e depois extraia o ZIP.

O instalador do repositório verifica a soma SHA-256 do manifesto HTTPS antes da extração, usando `sha256sum` ou `shasum`. Ele instala a versão mais recente em `~/.symbi/bin` e não faz a verificação independente de identidade Sigstore. Defina `SYMBI_INSTALL_DIR` para o processo `bash` se precisar de outra raiz de instalação. Use um download manual para fixar a versão.

## Contêineres e crates

As imagens versionadas são `ghcr.io/thirdkeyai/symbi:2.0.0` e `ghcr.io/thirdkeyai/symbi-a2ui:2.0.0`. Verifique ambas antes da implantação:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:2.0.0 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v2.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha 21471d3a9dea8ccf24d9248ff9338b35a79215e5
```

No crates.io, confira a versão exata e se ela não foi retirada. Os onze novos crates 2.0.0 devem corresponder às somas do registro e ter `.cargo_vcs_info.json` apontando para o commit público. Permanecem inalterados `symbi-approval-relay` 0.1.1, `symbi-channel-adapter` 0.1.6 e `symbi-invis-strip` 0.3.0, com seus commits anteriores.

Consulte a [validação](/release-2.0-validation/) para conhecer os testes e seus limites, e a [migração](/migration-2.0/) antes de atualizar uma implantação (ambas em inglês).

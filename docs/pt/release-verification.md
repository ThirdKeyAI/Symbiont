# Verificar a versão 3.0

Este guia corresponde à v3.0.0. Execute as verificações somente após a conclusão da publicação e a disponibilidade de todos os arquivos indicados. O contrato de saída e a seleção de Landlock para agentes registrados alteram a API Rust; consulte a migração em [acesso de saída](/pt/outbound-capabilities).

Obtenha o commit público pela tag `v3.0.0` do repositório confiável, conforme mostrado abaixo. Não use como referência um commit extraído de um registro de origem baixado sem verificação. O Gitea mantém o histórico completo; o GitHub contém a exportação OSS filtrada. Os identificadores diferem; as assinaturas e a procedência dos pacotes referem-se ao commit público.

## Escolher um arquivo

| Plataforma | Destino | Extensão |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Exemplo: `symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz`. Não há arquivo pré-compilado para macOS Intel; instale a partir do código-fonte. Os binários de macOS e Windows não oferecem isolamento Landlock.

Os diários protegidos de execução e os caminhos que dependem deles exigem Unix atualmente. Os binários para Windows oferecem as funções portáteis da CLI; a execução agendada protegida, a admissão persistente de chat e as melhorias governadas recusam operações não suportadas. Uma compilação bem-sucedida no Windows não comprova isolamento equivalente.

## Verificar antes de extrair

Estes comandos exigem Bash, GitHub CLI e Cosign 3.1+. Use um diretório vazio e defina `ARCHIVE` com o nome correspondente à sua plataforma. Baixe também `.sig`, `.pem`, o manifesto assinado de somas de verificação e `release-source.json`:

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v3.0.0 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz
gh release download v3.0.0 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

A versão publica assinaturas e certificados separados. Converta-os em um bundle com a evidência do registro de transparência e verifique a identidade esperada do workflow e o commit exato:

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

O registro deve conter a tag `v3.0.0` e o commit `$RELEASE_SHA`. Extraia e execute o binário somente após a verificação bem-sucedida. Para verificar a assinatura do arquivo separadamente, repita os comandos com `"$ARCHIVE"`, `"$ARCHIVE.sig"` e `"$ARCHIVE.pem"`. No Windows, use Git Bash ou ferramentas equivalentes e depois extraia o ZIP.

O instalador do repositório verifica a soma SHA-256 do manifesto HTTPS antes da extração, usando `sha256sum` ou `shasum`. Ele instala a versão mais recente em `~/.symbi/bin` e não faz a verificação independente de identidade Sigstore. Defina `SYMBI_INSTALL_DIR` para o processo `bash` se precisar de outra raiz de instalação. Use um download manual para fixar a versão.

## Contêineres e crates

As imagens versionadas são `ghcr.io/thirdkeyai/symbi:3.0.0` e `ghcr.io/thirdkeyai/symbi-a2ui:3.0.0`. Verifique ambas antes da implantação:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:3.0.0 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v3.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

Verifique as versões exatas não retiradas e as somas dos arquivos. Os pacotes novos devem vincular o commit público em `.cargo_vcs_info.json`; os pacotes inalterados mantêm a procedência anterior.

| Crates | Version |
| --- | --- |
| `symbi`, `symbi-dsl`, `symbi-runtime`, `repl-core`, `repl-cli`, `repl-lsp`, `symbi-shell` | 3.0.0 |
| `repl-proto`, `symbi-session`, `symbi-sandbox-guest`, `symbi-sandbox-supervisor` | 2.0.1 |
| `symbi-approval-relay` | 0.1.1 |
| `symbi-channel-adapter` | 0.1.6 |
| `symbi-invis-strip` | 0.3.0 |

Consulte a [validação](/release-3.0-validation/) para conhecer os testes e seus limites, e a [migração](/migration-2.0/) antes de atualizar uma implantação (ambas em inglês).

# Verificar la versión 3.0

Esta guía corresponde a v3.0.0. Ejecute las comprobaciones cuando la publicación haya terminado y estén disponibles todos los archivos indicados. El contrato de salida y la selección de Landlock para agentes registrados cambian la API Rust; consulte la migración en [acceso saliente](/es/outbound-capabilities).

Obtén el commit público a partir de la etiqueta `v3.0.0` del repositorio de confianza, como se muestra abajo. No tomes el commit esperado de un registro de origen descargado sin verificar. Gitea conserva el historial completo y GitHub contiene la exportación OSS filtrada. Sus identificadores difieren; las firmas y la procedencia de los paquetes se refieren al commit público.

## Elegir un archivo

| Plataforma | Destino | Extensión |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Ejemplo: `symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz`. No hay un archivo precompilado para macOS Intel; instala desde el código fuente. Los binarios de macOS y Windows no proporcionan aislamiento Landlock.

Los diarios protegidos de ejecución y las rutas que dependen de ellos requieren Unix actualmente. Los binarios de Windows ofrecen las funciones portátiles de la CLI; la ejecución programada protegida, la admisión persistente de chat y las mejoras gobernadas rechazan las operaciones no compatibles. Una compilación correcta en Windows no demuestra un aislamiento equivalente.

## Verificar antes de extraer

Estos comandos requieren Bash, GitHub CLI y Cosign 3.1+. Usa un directorio vacío y asigna a `ARCHIVE` el nombre correspondiente a tu plataforma. Descarga también `.sig`, `.pem`, el manifiesto firmado de sumas de verificación y `release-source.json`:

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v3.0.0 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v3.0.0-x86_64-unknown-linux-gnu.tar.gz
gh release download v3.0.0 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

La versión publica firmas y certificados separados. Conviértelos en un paquete con la evidencia del registro de transparencia y verifica la identidad esperada del flujo y el commit exacto:

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

No omitas las comprobaciones del registro de transparencia ni amplíes la identidad permitida para evitar un error. Después verifica el archivo y el registro de origen:

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

El registro debe contener la etiqueta `v3.0.0` y el commit `$RELEASE_SHA`. Extrae y ejecuta el binario solo tras una verificación satisfactoria. Para verificar la firma del archivo por separado, repite los comandos de creación y verificación con `"$ARCHIVE"`, `"$ARCHIVE.sig"` y `"$ARCHIVE.pem"`. En Windows puedes usar Git Bash o herramientas equivalentes y después extraer el ZIP.

El instalador del repositorio verifica la suma SHA-256 del manifiesto HTTPS antes de extraer, mediante `sha256sum` o `shasum`. Instala la última versión en `~/.symbi/bin`; no realiza la comprobación independiente de identidad Sigstore. Define `SYMBI_INSTALL_DIR` para el proceso `bash` si necesitas otra raíz de instalación. Usa una descarga manual para fijar una versión.

## Contenedores y crates

Las imágenes versionadas son `ghcr.io/thirdkeyai/symbi:3.0.0` y `ghcr.io/thirdkeyai/symbi-a2ui:3.0.0`. Verifica ambas antes del despliegue:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:3.0.0 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v3.0.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

Verifique las versiones exactas no retiradas y las sumas de los archivos. Los paquetes nuevos deben vincular el commit público en `.cargo_vcs_info.json`; los paquetes sin cambios conservan su procedencia anterior.

| Crates | Version |
| --- | --- |
| `symbi`, `symbi-dsl`, `symbi-runtime`, `repl-core`, `repl-cli`, `repl-lsp`, `symbi-shell` | 3.0.0 |
| `repl-proto`, `symbi-session`, `symbi-sandbox-guest`, `symbi-sandbox-supervisor` | 2.0.1 |
| `symbi-approval-relay` | 0.1.1 |
| `symbi-channel-adapter` | 0.1.6 |
| `symbi-invis-strip` | 0.3.0 |

Consulta la [validación](/release-3.0-validation/) para conocer las pruebas y sus límites, y la [migración](/migration-2.0/) antes de actualizar un despliegue (ambas en inglés).

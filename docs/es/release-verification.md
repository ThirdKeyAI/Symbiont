# Verificar la versión 2.0

Esta guía corresponde a la [versión v2.0.1](https://github.com/ThirdKeyAI/Symbiont/releases/tag/v2.0.1). Verifica los archivos solo cuando haya terminado la publicación y estén disponibles todos los recursos indicados. Los crates y contenedores de 2.0.0 siguen publicados, pero un error de compilación en Windows impidió la carga conjunta de binarios y la actualización de Homebrew. La versión 2.0.1 corrige ese error y el instalador del repositorio.

Obtén el commit público a partir de la etiqueta `v2.0.1` del repositorio de confianza, como se muestra abajo. No tomes el commit esperado de un registro de origen descargado sin verificar. Gitea conserva el historial completo y GitHub contiene la exportación OSS filtrada. Sus identificadores difieren; las firmas y la procedencia de los paquetes se refieren al commit público.

## Elegir un archivo

| Plataforma | Destino | Extensión |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Ejemplo: `symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz`. No hay un archivo precompilado para macOS Intel; instala desde el código fuente. Los binarios de macOS y Windows no proporcionan aislamiento Landlock.

Los diarios protegidos de ejecución y las rutas que dependen de ellos requieren Unix actualmente. Los binarios de Windows ofrecen las funciones portátiles de la CLI; la ejecución programada protegida, la admisión persistente de chat y las mejoras gobernadas rechazan las operaciones no compatibles. Una compilación correcta en Windows no demuestra un aislamiento equivalente.

## Verificar antes de extraer

Estos comandos requieren Bash, GitHub CLI y Cosign 3.1+. Usa un directorio vacío y asigna a `ARCHIVE` el nombre correspondiente a tu plataforma. Descarga también `.sig`, `.pem`, el manifiesto firmado de sumas de verificación y `release-source.json`:

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v2.0.1 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz
gh release download v2.0.1 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

La versión publica firmas y certificados separados. Conviértelos en un paquete con la evidencia del registro de transparencia y verifica la identidad esperada del flujo y el commit exacto:

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

El registro debe contener la etiqueta `v2.0.1` y el commit `$RELEASE_SHA`. Extrae y ejecuta el binario solo tras una verificación satisfactoria. Para verificar la firma del archivo por separado, repite los comandos de creación y verificación con `"$ARCHIVE"`, `"$ARCHIVE.sig"` y `"$ARCHIVE.pem"`. En Windows puedes usar Git Bash o herramientas equivalentes y después extraer el ZIP.

El instalador del repositorio verifica la suma SHA-256 del manifiesto HTTPS antes de extraer, mediante `sha256sum` o `shasum`. Instala la última versión en `~/.symbi/bin`; no realiza la comprobación independiente de identidad Sigstore. Define `SYMBI_INSTALL_DIR` para el proceso `bash` si necesitas otra raíz de instalación. Usa una descarga manual para fijar una versión.

## Contenedores y crates

Las imágenes versionadas son `ghcr.io/thirdkeyai/symbi:2.0.1` y `ghcr.io/thirdkeyai/symbi-a2ui:2.0.1`. Verifica ambas antes del despliegue:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:2.0.1 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v2.0.1' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

En crates.io, verifica la versión exacta y que no esté retirada. Los once crates nuevos 2.0.1 deben coincidir con las sumas del registro y tener `.cargo_vcs_info.json` apuntando al commit público. Permanecen sin cambios `symbi-approval-relay` 0.1.1, `symbi-channel-adapter` 0.1.6 y `symbi-invis-strip` 0.3.0, con sus commits anteriores.

Consulta la [validación](/release-2.0-validation/) para conocer las pruebas y sus límites, y la [migración](/migration-2.0/) antes de actualizar un despliegue (ambas en inglés).

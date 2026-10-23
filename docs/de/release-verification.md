# Release 2.0 überprüfen

Diese Anleitung gilt für [Release v2.0.1](https://github.com/ThirdKeyAI/Symbiont/releases/tag/v2.0.1). Führen Sie die Archivprüfung erst aus, wenn die Veröffentlichung abgeschlossen ist und alle genannten Dateien vorliegen. Die Crates und Container von 2.0.0 bleiben veröffentlicht; ein Windows-Kompilierungsfehler verhinderte jedoch den gemeinsamen Upload der Binärdateien und das Homebrew-Update. Version 2.0.1 behebt diesen Fehler und korrigiert den Repository-Installer.

Ermitteln Sie den öffentlichen Quell-Commit anhand des Tags `v2.0.1` im vertrauenswürdigen Repository, wie unten gezeigt. Übernehmen Sie den erwarteten Commit nicht aus einer ungeprüften heruntergeladenen Quelldatei. Gitea enthält die vollständige Historie, GitHub den gefilterten OSS-Export. Die Commit-IDs unterscheiden sich; Release-Signaturen und Paketnachweise beziehen sich auf den öffentlichen Commit.

## Archiv auswählen

| Plattform | Ziel | Endung |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Beispiel: `symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz`. Für Intel-Macs gibt es kein vorgefertigtes Archiv; installieren Sie aus dem Quellcode. macOS- und Windows-Binärdateien bieten keine Landlock-Isolation.

Geschützte Ausführungsjournale und die davon abhängigen Ausführungspfade benötigen derzeit Unix. Windows-Binärdateien unterstützen die portablen CLI-Funktionen; geschützte Zeitplanung, dauerhafte Chat-Zulassung und gesteuerte Verbesserungen verweigern nicht unterstützte Operationen. Ein erfolgreicher Windows-Build belegt keine gleichwertige Isolation.

## Vor dem Entpacken überprüfen

Die Befehle benötigen Bash, GitHub CLI und Cosign 3.1+. Verwenden Sie ein leeres Verzeichnis und setzen Sie `ARCHIVE` auf den Dateinamen Ihrer Plattform. Laden Sie auch `.sig`, `.pem`, das signierte Prüfsummenmanifest und `release-source.json` herunter:

```bash
set -euo pipefail
RELEASE_SHA=$(gh api repos/ThirdKeyAI/Symbiont/commits/v2.0.1 --jq .sha)
[[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || exit 1
ARCHIVE=symbi-v2.0.1-x86_64-unknown-linux-gnu.tar.gz
gh release download v2.0.1 --repo ThirdKeyAI/Symbiont \
  --pattern "$ARCHIVE" --pattern "$ARCHIVE.sig" --pattern "$ARCHIVE.pem" \
  --pattern 'checksums.txt*' --pattern release-source.json
```

Das Release liefert separate Signaturen und Zertifikate. Erstellen Sie daraus ein Bundle mit Transparenzprotokoll-Nachweis und überprüfen Sie die erwartete Workflow-Identität sowie den exakten Quell-Commit:

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

Überspringen Sie keine Transparenzprotokoll-Prüfung und erweitern Sie die erlaubte Identität nicht, um einen Fehler zu umgehen. Prüfen Sie danach Archiv und Quellnachweis:

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

Der Quellnachweis muss Tag `v2.0.1` und Commit `$RELEASE_SHA` enthalten. Entpacken und starten Sie die Binärdatei erst nach erfolgreicher Prüfung. Die Archivsignatur lässt sich separat mit denselben Bundle-Befehlen für `"$ARCHIVE"`, `"$ARCHIVE.sig"` und `"$ARCHIVE.pem"` prüfen. Unter Windows können Sie Git Bash oder gleichwertige Prüfwerkzeuge verwenden und anschließend die ZIP-Datei entpacken.

Der Repository-Installer prüft die SHA-256-Prüfsumme aus dem HTTPS-Release-Manifest vor dem Entpacken mit `sha256sum` oder `shasum`. Er installiert die neueste Version unter `~/.symbi/bin` und führt keine unabhängige Sigstore-Identitätsprüfung durch. Setzen Sie `SYMBI_INSTALL_DIR` für den `bash`-Prozess, um ein anderes Stammverzeichnis zu wählen. Für eine feste Version verwenden Sie einen manuellen Download.

## Container und Crates

Die versionierten Images sind `ghcr.io/thirdkeyai/symbi:2.0.1` und `ghcr.io/thirdkeyai/symbi-a2ui:2.0.1`. Prüfen Sie beide vor dem Deployment:

```bash
cosign verify ghcr.io/thirdkeyai/symbi:2.0.1 \
  --certificate-identity 'https://github.com/ThirdKeyAI/Symbiont/.github/workflows/docker-build.yml@refs/tags/v2.0.1' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  --certificate-github-workflow-sha "$RELEASE_SHA"
```

Prüfen Sie bei crates.io die genaue Version und ob sie zurückgezogen wurde. Bei den elf neuen 2.0.1-Crates müssen Archiv-Prüfsumme und Registry-Prüfsumme übereinstimmen; `.cargo_vcs_info.json` muss den öffentlichen Release-Commit nennen. Unverändert bleiben `symbi-approval-relay` 0.1.1, `symbi-channel-adapter` 0.1.6 und `symbi-invis-strip` 0.3.0 mit ihren früheren Quell-Commits.

Die [Validierung](/release-2.0-validation/) beschreibt Tests und Grenzen. Lesen Sie vor einem Upgrade die [Migrationsanleitung](/migration-2.0/) (beide auf Englisch).

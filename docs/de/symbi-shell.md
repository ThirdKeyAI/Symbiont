# Symbi Shell — Interaktive Agenten-Orchestrierung

> **Status: Beta.** `symbi shell` ist im Alltag nutzbar, aber Befehlsoberflaeche, Tastenbelegungen und Persistenzformate koennen sich zwischen Minor-Releases noch aendern. Melden Sie Issues unter [thirdkeyai/symbiont](https://github.com/thirdkeyai/symbiont/issues) mit dem Label `shell`.

`symbi shell` ist eine auf [ratatui](https://ratatui.rs) basierende Terminal-UI zum Erstellen, Orchestrieren und Betreiben von Symbiont-Agenten. Sie setzt auf demselben Runtime auf wie `symbi up` und `symbi run`, stellt es jedoch als interaktive Sitzung mit konversationellem Authoring, Live-Orchestrierung und Remote-Attach bereit.

## Wann die Shell zu verwenden ist

| Anwendungsfall | Befehl |
|----------|---------|
| Ein Projekt scaffolden und Agenten, Tools und Richtlinien mit LLM-Unterstuetzung iterieren | `symbi shell` |
| Einen Agenten ohne interaktive Schleife bis zum Ende ausfuehren | `symbi run <agent> -i <json>` |
| Das volle Runtime fuer Webhooks, Cron und Chat-Adapter starten | `symbi up` |

Die Shell ist der Standard-Einstiegspunkt fuer Authoring. Die nicht-interaktiven Befehle eignen sich besser in CI, Cron-Jobs und Deployment-Pipelines.

## Start

```bash
symbi shell                    # frische Sitzung starten
symbi shell --list-sessions    # gespeicherte Sitzungen anzeigen und beenden
symbi shell --resume <id>      # eine Sitzung per UUID wiederoeffnen
```

`--resume` akzeptiert entweder eine UUID oder einen zuvor mit `/snapshot` gespeicherten Snapshot-Namen.

## Layout

Die Shell verwendet einen Inline-Viewport, der sich das Terminal mit Ihrem vorhandenen Scrollback teilt. Von oben nach unten sehen Sie:

- **Projektstruktur-Sidebar** (umschaltbar) -- Dateibaum des aktuellen Projekts, hebt Agenten, Richtlinien und Tools hervor.
- **Trace-Timeline** -- Karten in ORGA-Phasen-Farben fuer Observe, Reason, Gate und Act, die waehrend LLM-Aufrufen in Echtzeit streamen.
- **Agenten-Karte** -- Metadaten, Richtlinien und juengste Aufrufe des aktuell ausgewaehlten Agenten.
- **Eingabezeile** -- tippen Sie `/command` oder freie Prosa. `@mention` zieht Pfade und Agenten per Fuzzy-Vervollstaendigung heran.

Syntax-Highlighting deckt ueber Tree-Sitter-Grammatiken das Symbiont-DSL, Cedar und ToolClad-Manifeste ab.

### Tastenbelegungen

| Belegung | Aktion |
|---------|--------|
| `Enter` | Eingabe absenden (funktioniert auch bei sichtbarem Vervollstaendigungs-Popup) |
| `/` oder `@` | Vervollstaendigungs-Popup automatisch oeffnen |
| `↑` / `↓` | Eingabeverlauf oder Popup-Eintraege durchlaufen |
| `Ctrl+R` | Reverse-History-Suche |
| `Tab` | Hervorgehobene Vervollstaendigung uebernehmen |
| `Esc` | Popup schliessen / laufenden LLM-Aufruf abbrechen |
| `Ctrl+L` | Sichtbaren Ausgabepuffer leeren |
| `Ctrl+D` | Shell beenden |

Unter Zellij erkennt die Shell den Multiplexer und gibt eine Inline-Viewport-Kompatibilitaetswarnung aus; verwenden Sie `--full-screen`, wenn Sie stattdessen in einem Alternate-Screen-Puffer laufen wollen.

## Befehlskatalog

Befehle sind nach Zweck gruppiert. Jeder Befehl akzeptiert `help` / `--help` / `-h`, um einen kurzen Nutzungshinweis auszugeben, ohne an den Orchestrator weitergeleitet zu werden.

### Authoring

| Befehl | Funktion |
|---------|-------------|
| `/init [profile\|description]` | Ein Symbiont-Projekt scaffolden. Bekannte Profilnamen (`minimal`, `assistant`, `dev-agent`, `multi-agent`) fuehren ein deterministisches Scaffolding aus; jede andere Zeichenkette wird als freie Beschreibung behandelt, aus der der Orchestrator ein Profil auswaehlt. |
| `/spawn <description>` | Einen DSL-Agenten aus Prosa generieren. Das Ergebnis wird gegen Projekt-Constraints validiert, bevor es in `agents/` geschrieben wird. |
| `/policy <requirement>` | Eine Cedar-Richtlinie fuer die beschriebene Anforderung generieren und validieren. |
| `/tool <description>` | Ein ToolClad `.clad.toml`-Manifest generieren und validieren. |
| `/behavior <description>` | Einen wiederverwendbaren DSL-Behavior-Block generieren und validieren. |

Authoring-Befehle schreiben erst nach erfolgreicher Validierung auf die Festplatte. Constraint-Verletzungen werden in der Trace-Timeline mit zeilengenauen Fehlern erklaert.

### Orchestrierung

| Befehl | Muster |
|---------|---------|
| `/run <agent> [input]` | Einen Agenten starten oder erneut ausfuehren. |
| `/ask <agent> <message>` | Eine Nachricht an einen Agenten senden und auf die Antwort warten. |
| `/send <agent> <message>` | Eine Nachricht senden, ohne auf die Antwort zu warten. |
| `/chain <a,b,c> <input>` | Die Ausgabe jedes Agenten in den naechsten leiten. |
| `/parallel <a,b,c> <input>` | Agenten parallel mit derselben Eingabe ausfuehren; Ergebnisse aggregieren. |
| `/race <a,b,c> <input>` | Parallel ausfuehren, die erste erfolgreiche Antwort gewinnt, der Rest wird abgebrochen. |
| `/debate <a,b,c> <topic>` | Strukturierte Multi-Agent-Debatte zu einem Thema. |
| `/exec <command>` | Einen Shell-Befehl innerhalb des sandboxed Dev-Agenten ausfuehren. |

### Betrieb

| Befehl | Funktion |
|---------|-------------|
| `/agents` | Aktive Agenten auflisten. |
| `/monitor [agent]` | Live-Status fuer den angegebenen Agenten (oder alle) streamen. |
| `/logs [agent]` | Juengste Logs anzeigen. |
| `/audit [filter]` | Juengste Audit-Trail-Eintraege anzeigen; nach Agent, Entscheidung oder Zeitraum filtern. |
| `/doctor` | Die lokale Runtime-Umgebung diagnostizieren. |
| `/memory <agent> [query]` | Den Speicher eines Agenten abfragen. |
| `/debug <agent>` | Den internen Zustand eines Agenten inspizieren. |
| `/pause`, `/resume-agent`, `/stop`, `/destroy` | Lifecycle-Steuerung fuer Agenten. |

### Tools, Skills und Verifikation

| Befehl | Funktion |
|---------|-------------|
| `/tools [list\|add\|remove]` | Fuer Agenten verfuegbare ToolClad-Tools verwalten. |
| `/skills [list\|install\|remove]` | Fuer Agenten verfuegbare Skills verwalten. |
| `/verify <artifact>` | Ein signiertes Artefakt (Tool-Manifest, Skill) gegen seine SchemaPin-Signatur verifizieren. |

### Scheduling

| Befehl | Funktion |
|---------|-------------|
| `/cron list` | Geplante Agent-Jobs auflisten. |
| `/cron add` / `/cron remove` | Geplante Jobs erstellen oder loeschen. |
| `/cron history` | Juengste Laeufe anzeigen. |

`/cron` funktioniert sowohl lokal als auch ueber Remote-Attach (siehe unten). Siehe den [Scheduling-Leitfaden](/scheduling) fuer die vollstaendige Cron-Engine.

### Channels

| Befehl | Funktion |
|---------|-------------|
| `/channels` | Registrierte Channel-Adapter auflisten (Slack, Teams, Mattermost). |
| `/connect <channel>` | Einen neuen Channel-Adapter registrieren. |
| `/disconnect <channel>` | Einen Adapter entfernen. |

Channel-Verwaltung erfordert einen Remote-Attach, wenn sie auf ein deploytes Runtime zielt.

### Secrets

| Befehl | Funktion |
|---------|-------------|
| `/secrets list\|set\|get\|remove` | Secrets im verschluesselten lokalen Speicher des Runtimes verwalten. |

Secrets werden im Ruhezustand mit `SYMBIONT_MASTER_KEY` verschluesselt und pro Agent isoliert.

### Deployment (Beta)

> **Status: Beta.** Der Deploy-Stack ist in der OSS-Edition Single-Agent. Multi-Agent- und Managed-Deploys sind auf der Roadmap.

| Befehl | Ziel |
|---------|--------|
| `/deploy local` | Docker mit einem gehaerteten Sandbox-Runner auf dem lokalen Docker-Daemon. |
| `/deploy cloudrun` | Google Cloud Run — baut ein Image, pusht es und deployt einen Service. |
| `/deploy aws` | AWS App Runner. |

`/deploy` liest den aktiven Agenten und die Projektkonfiguration und erzeugt ein reproduzierbares Deployment-Artefakt. Fuer Multi-Agent-Topologien deployen Sie Koordinator und Worker separat und verdrahten sie mit instanzenuebergreifendem Messaging (siehe [Runtime-Architektur](/runtime-architecture#cross-instance-agent-messaging)).

### Remote-Attach

| Befehl | Funktion |
|---------|-------------|
| `/attach <url>` | Diese Shell per HTTP an ein entferntes Runtime anhaengen. |
| `/detach` | Vom aktuell angehaengten Runtime loesen. |

Nach dem Anhaengen wirken `/cron`, `/channels`, `/agents`, `/audit` und die meisten Betriebsbefehle auf das entfernte Runtime statt auf das lokale. `/secrets` bleibt lokal -- Remote-Secrets verbleiben im Speicher des entfernten Runtimes.

### Sitzungsverwaltung

| Befehl | Funktion |
|---------|-------------|
| `/snapshot [name]` | Aktuelle Sitzung speichern. |
| `/resume <snapshot>` | Einen gespeicherten Snapshot wiederherstellen. |
| `/export <path>` | Das Konversationstranskript auf die Festplatte exportieren. |
| `/new` | Eine neue Sitzung starten und die aktuelle verwerfen. |
| `/compact [limit]` | Die Konversationshistorie komprimieren, um in ein Token-Budget zu passen. |
| `/context` | Aktuelles Kontextfenster und Token-Nutzung anzeigen. |

Sitzungen werden unter `.symbi/sessions/<uuid>/` gespeichert. Die Shell loest Compaction automatisch aus, wenn der Kontext das konfigurierte Budget ueberschreitet.

### Sitzungssteuerung

| Befehl | Funktion |
|---------|-------------|
| `/model [name]` | Aktuelles Inferenzmodell anzeigen oder wechseln. |
| `/cost` | Token- und API-Kosten-Summen fuer die Sitzung anzeigen. |
| `/status` | Runtime- und Sitzungsstatus anzeigen. |
| `/dsl` | Zwischen DSL- und Orchestrator-Eingabemodi umschalten -- der DSL-Modus evaluiert in-process. |
| `/clear` | Sichtbaren Ausgabepuffer leeren (Historie bleibt erhalten). |
| `/quit` / `/exit` | Shell beenden. |
| `/help` | Den Befehlskatalog anzeigen. |

## DSL-Modus

Druecken Sie `/dsl`, um die Eingabezeile in den DSL-Modus zu versetzen. Im DSL-Modus parst und evaluiert die Shell Eingaben gegen den In-Process-DSL-Interpreter mit tree-sitter-gestuetzter Vervollstaendigung und Fehlern, ohne sie durch den Orchestrator zu leiten. Mit erneutem `/dsl` wird zurueckgeschaltet.

## Constraints und Validierung

Authoring-Befehle erzwingen eine lokale Validierungspipeline:

1. Generierte Artefakte werden gegen die Symbiont-DSL-Grammatik, Cedar oder ToolClad entsprechend geparst.
2. Ein Constraint-Lader prueft das Ergebnis gegen Constraints auf Projektebene (z.B. verbotene Capabilities, erforderliche Richtlinien).
3. Erst nach erfolgreichen Schritten wird das Artefakt auf die Festplatte geschrieben.

Der Orchestrator-LLM kann die Wirkungen der Constraint-Datei ueber Validierungsfehler sehen, sie selbst aber nicht veraendern -- das ist dasselbe Vertrauensmodell, das die `symbi tools validate`-Pipeline verwendet.

## Beta-Einschraenkungen

Die folgenden Teile der Shell befinden sich noch in aktiver Entwicklung und koennen sich ohne Deprecation-Fenster aendern:

- `/branch` und `/copy` (Sitzungs-Branching) sind reservierte Befehle und geben derzeit einen "fuer ein zukuenftiges Release geplant"-Stub aus.
- `/deploy cloudrun` und `/deploy aws` sind ausschliesslich Single-Agent.
- Snapshot-Format und `.symbi/sessions/`-Layout koennen sich zwischen Minor-Releases aendern; verwenden Sie `/export`, wenn Sie dauerhafte Transkripte benoetigen.
- Fuzzy-Vervollstaendigungs-Heuristiken und das Trace-Timeline-Layout werden anhand von Feedback abgestimmt und koennen sich verschieben.

Wenn Sie heute eine stabile Oberflaeche benoetigen, bevorzugen Sie `symbi up`, `symbi run` und die [HTTP-API](/api-reference) -- diese sind durch die Kompatibilitaetsgarantien in `SECURITY.md` abgedeckt.

## Siehe auch

- [Einstieg](/getting-started) -- Installation und `symbi init`
- [DSL-Leitfaden](/dsl-guide) -- Referenz der Agenten-Definitionssprache
- [ToolClad](/toolclad) -- deklarative Tool-Kontrakte
- [Scheduling](/scheduling) -- Cron-Engine und Zustellungsrouting
- [Sicherheitsmodell](/security-model) -- Vertrauensgrenzen und Richtliniendurchsetzung

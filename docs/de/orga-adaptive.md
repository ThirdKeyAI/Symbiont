layout: default
title: Erweiterte Reasoning-Primitiven (orga-adaptive)
description: "Erweiterte Reasoning-Loop-Primitiven: Tool-Kuratierung, Stuck-Loop-Erkennung, Kontext-Pre-Fetch und verzeichnisspezifische Konventionen"
nav_exclude: true
---

# Erweiterte Reasoning-Primitiven

## Andere Sprachen

[English](orga-adaptive.md) | [中文简体](orga-adaptive.zh-cn.md) | [Español](orga-adaptive.es.md) | [Português](orga-adaptive.pt.md) | [日本語](orga-adaptive.ja.md) | **Deutsch**

---

Feature-gated Laufzeitprimitiven, die die Reasoning-Schleife um Tool-Kuratierung, Stuck-Loop-Erkennung, deterministischen Kontext-Pre-Fetch und verzeichnisspezifische Konventionsabfrage erweitern.

## Inhaltsverzeichnis


---

## Ueberblick

Das `orga-adaptive` Feature Gate fuegt vier erweiterte Faehigkeiten zur Reasoning-Schleife hinzu:

| Primitive | Geloestes Problem | Modul |
|-----------|------------------|-------|
| **Tool Profile** | LLM sieht zu viele Tools, verschwendet Tokens fuer irrelevante | `tool_profile.rs` |
| **Progress Tracker** | Schleifen bleiben haengen und versuchen denselben fehlgeschlagenen Schritt erneut | `progress_tracker.rs` |
| **Pre-Hydration** | Cold-Start-Kontextluecke -- Agent muss Referenzen selbst entdecken | `pre_hydrate.rs` |
| **Scoped Conventions** | Konventionsabfrage ist sprachweit, nicht verzeichnisspezifisch | `knowledge_bridge.rs` |

### Aktivierung

```toml
# In Ihrer Cargo.toml
[dependencies]
symbi-runtime = { version = "1.6", features = ["orga-adaptive"] }
```

Oder aus dem Quellcode kompilieren:

```bash
cargo build --features orga-adaptive
cargo test --features orga-adaptive
```

Alle Primitiven sind additiv und rueckwaertskompatibel -- bestehender Code kompiliert und laeuft identisch ohne das Feature Gate.

---

## Tool-Profilfilterung

Filtert Tool-Definitionen, bevor das LLM sie sieht. Reduziert Token-Verschwendung und verhindert, dass das Modell irrelevante Tools auswaehlt.

### Konfiguration

```rust
use symbi_runtime::reasoning::ToolProfile;

// Nur dateibezogene Tools einschliessen
let profile = ToolProfile::include_only(&["file_*", "code_*"]);

// Debug-Tools ausschliessen
let profile = ToolProfile::exclude_only(&["debug_*", "internal_*"]);

// Kombiniert: Web-Tools einschliessen, experimentelle ausschliessen, auf 10 begrenzen
let profile = ToolProfile {
    include: vec!["web_*".into(), "search_*".into()],
    exclude: vec!["web_experimental_*".into()],
    max_tools: Some(10),
    require_verified: false,
};
```

### Filter-Pipeline

Die Pipeline wird in Reihenfolge angewendet:

1. **Include** -- Wenn nicht leer, passieren nur Tools, die einem Include-Glob entsprechen
2. **Exclude** -- Tools, die einem Exclude-Glob entsprechen, werden entfernt
3. **Verified** -- Wenn `require_verified` true ist, passieren nur Tools mit `[verified]` in ihrer Beschreibung
4. **Max Cap** -- Auf `max_tools` kuerzen, falls gesetzt

### Glob-Syntax

| Muster | Trifft zu auf |
|--------|--------------|
| `web_*` | `web_search`, `web_fetch`, `web_scrape` |
| `tool_?` | `tool_a`, `tool_1` (einzelnes Zeichen) |
| `exact_name` | Nur `exact_name` |

### Integration mit LoopConfig

```rust
let config = LoopConfig {
    tool_profile: Some(ToolProfile::include_only(&["search_*", "file_*"])),
    ..Default::default()
};
```

Das Profil wird automatisch in `ReasoningLoopRunner::run()` angewendet, nachdem Tool-Definitionen vom Executor und der Wissensbruecke aufgefuellt wurden.

---

## Progress Tracker

Verfolgt Wiederholungsversuche pro Schritt und erkennt feststeckende Schleifen durch Vergleich aufeinanderfolgender Fehlerausgaben mittels normalisierter Levenshtein-Aehnlichkeit.

### Konfiguration

```rust
use symbi_runtime::reasoning::{ProgressTracker, StepIterationConfig, LimitAction};

let config = StepIterationConfig {
    max_reattempts_per_step: 2,    // Nach 2 fehlgeschlagenen Versuchen stoppen
    similarity_threshold: 0.85,    // Fehler mit 85%+ Aehnlichkeit = feststeckend
    on_limit_reached: LimitAction::SkipStep,
};

let mut tracker = ProgressTracker::new(config);
```

### Verwendung (Koordinator-Ebene)

Der Progress Tracker ist **nicht direkt in die Reasoning-Schleife eingebunden** -- er ist ein uebergeordnetes Anliegen fuer Koordinatoren, die mehrstufige Aufgaben orchestrieren.

```rust
// Verfolgung eines Schritts beginnen
tracker.begin_step("extract_data");

// Nach jedem Versuch den Fehler aufzeichnen und pruefen
let decision = tracker.record_and_check("extract_data", &error_output);

match decision {
    StepDecision::Continue => { /* erneut versuchen */ }
    StepDecision::Stop { reason } => {
        // LoopEvent::StepLimitReached emittieren und weitermachen
        match tracker.limit_action() {
            LimitAction::SkipStep => { /* zum naechsten Schritt springen */ }
            LimitAction::AbortTask => { /* gesamte Aufgabe abbrechen */ }
            LimitAction::Escalate => { /* an Menschen uebergeben */ }
        }
    }
}
```

### Stuck-Erkennung

Der Tracker berechnet die normalisierte Levenshtein-Distanz zwischen aufeinanderfolgenden Fehlerausgaben. Wenn die Aehnlichkeit den Schwellenwert ueberschreitet (Standard 85%), wird der Schritt als feststeckend betrachtet -- selbst wenn die maximale Wiederholungsanzahl noch nicht erreicht wurde.

Dies erfasst Szenarien, in denen der Agent immer wieder auf denselben Fehler mit leicht unterschiedlicher Formulierung stoesst.

---

## Pre-Hydration-Engine

Extrahiert Referenzen aus der Aufgabeneingabe (URLs, Dateipfade, GitHub Issues/PRs) und loest sie parallel auf, bevor die Reasoning-Schleife beginnt. Dies eliminiert die Cold-Start-Latenz, bei der der Agent diese Referenzen sonst selbst entdecken und abrufen muesste.

### Konfiguration

```rust
use symbi_runtime::reasoning::PreHydrationConfig;
use std::time::Duration;

let config = PreHydrationConfig {
    custom_patterns: vec![],
    resolution_tools: [
        ("url".into(), "web_fetch".into()),
        ("file".into(), "file_read".into()),
    ].into(),
    timeout: Duration::from_secs(15),
    max_references: 10,
    max_context_tokens: 4000,  // 1 Token ~ 4 Zeichen
};
```

### Eingebaute Muster

| Muster | Typ | Beispiel-Treffer |
|--------|-----|-----------------|
| URLs | `url` | `https://example.com/api`, `http://localhost:3000` |
| Dateipfade | `file` | `./src/main.rs`, `~/config.toml` |
| Issues | `issue` | `#42`, `#100` |
| Pull Requests | `pr` | `PR #55`, `pr #12` |

### Benutzerdefinierte Muster

```rust
use symbi_runtime::reasoning::pre_hydrate::ReferencePattern;

let config = PreHydrationConfig {
    custom_patterns: vec![
        ReferencePattern {
            ref_type: "jira".into(),
            pattern: r"[A-Z]+-\d+".into(),  // PROJ-123
        },
    ],
    ..Default::default()
};
```

### Aufloesungsablauf

1. **Extraktion** -- Regex-Muster scannen die Aufgabeneingabe und deduplizieren Treffer
2. **Aufloesung** -- Jede Referenz wird ueber das konfigurierte Tool aufgeloest (z.B. `web_fetch` fuer URLs)
3. **Budget** -- Ergebnisse werden auf `max_context_tokens` gekuerzt
4. **Injektion** -- Formatiert als `[PRE_HYDRATED_CONTEXT]` Systemnachricht (getrennt vom `[KNOWLEDGE_CONTEXT]`-Slot der Wissensbruecke)

### Integration mit LoopConfig

```rust
let config = LoopConfig {
    pre_hydration: Some(PreHydrationConfig {
        resolution_tools: [("url".into(), "web_fetch".into())].into(),
        ..Default::default()
    }),
    ..Default::default()
};
```

Pre-Hydration laeuft automatisch am Anfang von `run_inner()`, bevor die Haupt-Reasoning-Schleife beginnt. Ein `LoopEvent::PreHydrationComplete` Journal-Event wird mit Extraktions- und Aufloesungsstatistiken emittiert.

---

## Verzeichnisspezifische Konventionen

Erweitert das `recall_knowledge`-Tool um `directory`- und `scope`-Parameter fuer den Abruf von Programmierkonventionen, die auf ein bestimmtes Verzeichnis beschraenkt sind.

### Funktionsweise

Bei Aufruf mit `scope: "conventions"` und einem `directory` fuehrt die Wissensbruecke folgende Schritte aus:

1. Sucht nach Konventionen, die zum Verzeichnispfad passen
2. Wandert uebergeordnete Verzeichnisse hinauf (z.B. `src/api/` -> `src/` -> Projektwurzel)
3. Faellt auf sprachweite Konventionen zurueck
4. Dedupliziert nach Inhalt ueber alle Ebenen
5. Kuerzt auf das angeforderte Limit

### LLM-Tool-Aufruf

```json
{
  "name": "recall_knowledge",
  "arguments": {
    "query": "rust",
    "directory": "src/api/handlers",
    "scope": "conventions"
  }
}
```

### Rueckwaertskompatibilitaet

Die Parameter `directory` und `scope` sind optional. Ohne sie verhaelt sich `recall_knowledge` identisch zur Standardversion -- eine einfache Wissenssuche mit `query` und `limit`.

---

## LoopConfig-Felder

Wenn das `orga-adaptive` Feature aktiviert ist, erhaelt `LoopConfig` drei optionale Felder:

```rust
pub struct LoopConfig {
    // ... bestehende Felder ...

    /// Tool-Profil zum Filtern der fuer das LLM sichtbaren Tools.
    pub tool_profile: Option<ToolProfile>,
    /// Schrittweise Iterationslimits fuer Stuck-Loop-Erkennung.
    pub step_iteration: Option<StepIterationConfig>,
    /// Pre-Hydration-Konfiguration fuer deterministischen Kontext-Pre-Fetch.
    pub pre_hydration: Option<PreHydrationConfig>,
}
```

Alle haben den Standardwert `None` und werden mit `#[serde(default, skip_serializing_if = "Option::is_none")]` serialisiert fuer Rueckwaertskompatibilitaet.

## Journal-Events

Zwei neue `LoopEvent`-Varianten sind verfuegbar:

```rust
pub enum LoopEvent {
    // ... bestehende Varianten ...

    /// Ein Schritt hat sein Wiederholungslimit erreicht (von Koordinatoren emittiert).
    StepLimitReached {
        step_id: String,
        attempts: u32,
        reason: String,
    },
    /// Pre-Hydration-Phase abgeschlossen.
    PreHydrationComplete {
        references_found: usize,
        references_resolved: usize,
        references_failed: usize,
        total_tokens: usize,
    },
}
```

---

## Testen

```bash
# Ohne Feature (keine Regressionen)
cargo clippy --workspace -j2
cargo test --workspace -j2

# Mit Feature
cargo clippy --workspace -j2 --features orga-adaptive
cargo test --workspace -j2 --features orga-adaptive
```

Alle Tests sind inline `#[cfg(test)]` Module -- keine externen Test-Fixtures erforderlich.

---

## Moduluebersicht

| Modul | Oeffentliche Typen | Beschreibung |
|-------|-------------------|-------------|
| `tool_profile` | `ToolProfile` | Glob-basierte Tool-Filterung mit Verified-Flag und Max-Cap |
| `progress_tracker` | `ProgressTracker`, `StepIterationConfig`, `StepDecision`, `LimitAction` | Schrittweise Iterationsverfolgung mit Levenshtein-Stuck-Erkennung |
| `pre_hydrate` | `PreHydrationEngine`, `PreHydrationConfig`, `HydratedContext` | Referenzextraktion, parallele Aufloesung, Token-Budget-Kuerzung |
| `knowledge_bridge` | (erweitert) | `retrieve_scoped_conventions()`, erweitertes `recall_knowledge`-Tool |

---

## Naechste Schritte

- **[Reasoning-Loop-Leitfaden](reasoning-loop.md)** -- Dokumentation des Kern-ORGA-Zyklus
- **[Laufzeit-Architektur](runtime-architecture.md)** -- Vollstaendige Systemarchitektur-Uebersicht
- **[API-Referenz](api-reference.md)** -- Vollstaendige API-Dokumentation

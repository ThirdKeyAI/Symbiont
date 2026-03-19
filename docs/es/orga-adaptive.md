layout: default
title: Primitivas de Razonamiento Avanzado (orga-adaptive)
description: "Primitivas avanzadas del bucle de razonamiento: curacion de herramientas, deteccion de bucles atascados, pre-carga de contexto y convenciones con alcance de directorio"
nav_exclude: true
---

# Primitivas de Razonamiento Avanzado

## Otros idiomas

[English](orga-adaptive.md) | [中文简体](orga-adaptive.zh-cn.md) | Primitivas de runtime con feature gate que mejoran el bucle de razonamiento con curacion de herramientas, deteccion de bucles atascados, pre-carga determinista de contexto y recuperacion de convenciones con alcance de directorio.

---

## Tabla de contenidos


---

## Descripcion General

El feature gate `orga-adaptive` agrega cuatro capacidades avanzadas al bucle de razonamiento:

| Primitiva | Problema que Resuelve | Modulo |
|-----------|----------------------|--------|
| **Tool Profile** | El LLM ve demasiadas herramientas, desperdicia tokens en irrelevantes | `tool_profile.rs` |
| **Progress Tracker** | Los bucles se atascan reintentando el mismo paso fallido | `progress_tracker.rs` |
| **Pre-Hydration** | Brecha de contexto en inicio frio — el agente debe descubrir referencias por si mismo | `pre_hydrate.rs` |
| **Scoped Conventions** | La recuperacion de convenciones es a nivel de lenguaje, no especifica de directorio | `knowledge_bridge.rs` |

### Habilitacion

```toml
# In your Cargo.toml
[dependencies]
symbi-runtime = { version = "1.6", features = ["orga-adaptive"] }
```

O compilar desde el codigo fuente:

```bash
cargo build --features orga-adaptive
cargo test --features orga-adaptive
```

Todas las primitivas son aditivas y retrocompatibles — el codigo existente compila y se ejecuta de forma identica sin el feature gate.

---

## Filtrado de Perfiles de Herramientas

Filtra las definiciones de herramientas antes de que el LLM las vea. Reduce el desperdicio de tokens y evita que el modelo seleccione herramientas irrelevantes.

### Configuracion

```rust
use symbi_runtime::reasoning::ToolProfile;

// Include only file-related tools
let profile = ToolProfile::include_only(&["file_*", "code_*"]);

// Exclude debug tools
let profile = ToolProfile::exclude_only(&["debug_*", "internal_*"]);

// Combined: include web tools, exclude experimental ones, cap at 10
let profile = ToolProfile {
    include: vec!["web_*".into(), "search_*".into()],
    exclude: vec!["web_experimental_*".into()],
    max_tools: Some(10),
    require_verified: false,
};
```

### Pipeline de Filtrado

El pipeline se aplica en orden:

1. **Include** — Si no esta vacio, solo pasan las herramientas que coincidan con algun glob de inclusion
2. **Exclude** — Las herramientas que coincidan con algun glob de exclusion se eliminan
3. **Verified** — Si `require_verified` es true, solo pasan las herramientas con `[verified]` en su descripcion
4. **Max cap** — Se trunca a `max_tools` si esta configurado

### Sintaxis de Glob

| Patron | Coincidencias |
|--------|--------------|
| `web_*` | `web_search`, `web_fetch`, `web_scrape` |
| `tool_?` | `tool_a`, `tool_1` (un solo caracter) |
| `exact_name` | Solo `exact_name` |

### Integracion con LoopConfig

```rust
let config = LoopConfig {
    tool_profile: Some(ToolProfile::include_only(&["search_*", "file_*"])),
    ..Default::default()
};
```

El perfil se aplica automaticamente en `ReasoningLoopRunner::run()` despues de que las definiciones de herramientas se completan desde el executor y el puente de conocimiento.

---

## Progress Tracker

Rastrea los conteos de reintentos por paso y detecta bucles atascados comparando salidas de error consecutivas usando similitud de Levenshtein normalizada.

### Configuracion

```rust
use symbi_runtime::reasoning::{ProgressTracker, StepIterationConfig, LimitAction};

let config = StepIterationConfig {
    max_reattempts_per_step: 2,    // Stop after 2 failed attempts
    similarity_threshold: 0.85,    // Errors 85%+ similar = stuck
    on_limit_reached: LimitAction::SkipStep,
};

let mut tracker = ProgressTracker::new(config);
```

### Uso (Nivel de Coordinador)

El progress tracker **no esta conectado directamente al bucle de razonamiento** — es una preocupacion de orden superior para coordinadores que orquestan tareas de multiples pasos.

```rust
// Begin tracking a step
tracker.begin_step("extract_data");

// After each attempt, record the error and check
let decision = tracker.record_and_check("extract_data", &error_output);

match decision {
    StepDecision::Continue => { /* retry */ }
    StepDecision::Stop { reason } => {
        // Emit LoopEvent::StepLimitReached and move on
        match tracker.limit_action() {
            LimitAction::SkipStep => { /* skip to next step */ }
            LimitAction::AbortTask => { /* abort entire task */ }
            LimitAction::Escalate => { /* hand off to human */ }
        }
    }
}
```

### Deteccion de Atascos

El tracker computa la distancia de Levenshtein normalizada entre salidas de error consecutivas. Si la similitud excede el umbral (por defecto 85%), el paso se considera atascado — incluso si el conteo maximo de reintentos no se ha alcanzado.

Esto captura escenarios donde el agente sigue encontrando el mismo error con redaccion ligeramente diferente.

---

## Motor de Pre-Hidratacion

Extrae referencias de la entrada de la tarea (URLs, rutas de archivo, issues/PRs de GitHub) y las resuelve en paralelo antes de que comience el bucle de razonamiento. Esto elimina la latencia de inicio frio donde el agente de otro modo necesitaria descubrir y obtener estas referencias por si mismo.

### Configuracion

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
    max_context_tokens: 4000,  // 1 token ~ 4 chars
};
```

### Patrones Integrados

| Patron | Tipo | Ejemplos de Coincidencia |
|--------|------|-------------------------|
| URLs | `url` | `https://example.com/api`, `http://localhost:3000` |
| Rutas de archivo | `file` | `./src/main.rs`, `~/config.toml` |
| Issues | `issue` | `#42`, `#100` |
| Pull requests | `pr` | `PR #55`, `pr #12` |

### Patrones Personalizados

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

### Flujo de Resolucion

1. **Extraer** — Los patrones regex escanean la entrada de la tarea, deduplicando coincidencias
2. **Resolver** — Cada referencia se resuelve via la herramienta configurada (ej., `web_fetch` para URLs)
3. **Presupuesto** — Los resultados se podan para ajustarse dentro de `max_context_tokens`
4. **Inyectar** — Formateado como un mensaje de sistema `[PRE_HYDRATED_CONTEXT]` (separado del slot `[KNOWLEDGE_CONTEXT]` del puente de conocimiento)

### Integracion con LoopConfig

```rust
let config = LoopConfig {
    pre_hydration: Some(PreHydrationConfig {
        resolution_tools: [("url".into(), "web_fetch".into())].into(),
        ..Default::default()
    }),
    ..Default::default()
};
```

La pre-hidratacion se ejecuta automaticamente al inicio de `run_inner()` antes de que comience el bucle de razonamiento principal. Se emite un evento de diario `LoopEvent::PreHydrationComplete` con estadisticas de extraccion y resolucion.

---

## Convenciones con Alcance de Directorio

Extiende la herramienta `recall_knowledge` con parametros `directory` y `scope` para recuperar convenciones de codificacion con alcance a un directorio especifico.

### Como Funciona

Cuando se llama con `scope: "conventions"` y un `directory`, el puente de conocimiento:

1. Busca convenciones que coincidan con la ruta del directorio
2. Recorre directorios padres (ej., `src/api/` -> `src/` -> raiz del proyecto)
3. Recurre a convenciones a nivel de lenguaje
4. Deduplica por contenido a traves de todos los niveles
5. Trunca al limite solicitado

### Llamada de Herramienta del LLM

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

### Retrocompatibilidad

Los parametros `directory` y `scope` son opcionales. Sin ellos, `recall_knowledge` se comporta de forma identica a la version estandar — una busqueda de conocimiento simple con `query` y `limit`.

---

## Campos de LoopConfig

Cuando la feature `orga-adaptive` esta habilitada, `LoopConfig` gana tres campos opcionales:

```rust
pub struct LoopConfig {
    // ... existing fields ...

    /// Tool profile for filtering tools visible to the LLM.
    pub tool_profile: Option<ToolProfile>,
    /// Per-step iteration limits for stuck loop detection.
    pub step_iteration: Option<StepIterationConfig>,
    /// Pre-hydration configuration for deterministic context pre-fetch.
    pub pre_hydration: Option<PreHydrationConfig>,
}
```

Todos tienen valor predeterminado `None` y se serializan con `#[serde(default, skip_serializing_if = "Option::is_none")]` para retrocompatibilidad.

## Eventos de Diario

Dos nuevas variantes de `LoopEvent` estan disponibles:

```rust
pub enum LoopEvent {
    // ... existing variants ...

    /// A step hit its reattempt limit (emitted by coordinators).
    StepLimitReached {
        step_id: String,
        attempts: u32,
        reason: String,
    },
    /// Pre-hydration phase completed.
    PreHydrationComplete {
        references_found: usize,
        references_resolved: usize,
        references_failed: usize,
        total_tokens: usize,
    },
}
```

---

## Pruebas

```bash
# Without feature (no regressions)
cargo clippy --workspace -j2
cargo test --workspace -j2

# With feature
cargo clippy --workspace -j2 --features orga-adaptive
cargo test --workspace -j2 --features orga-adaptive
```

Todas las pruebas son modulos `#[cfg(test)]` en linea — no se necesitan fixtures de prueba externos.

---

## Mapa de Modulos

| Modulo | Tipos Publicos | Descripcion |
|--------|---------------|-------------|
| `tool_profile` | `ToolProfile` | Filtrado de herramientas basado en glob con flag de verificado y limite maximo |
| `progress_tracker` | `ProgressTracker`, `StepIterationConfig`, `StepDecision`, `LimitAction` | Seguimiento de iteraciones por paso con deteccion de atascos por Levenshtein |
| `pre_hydrate` | `PreHydrationEngine`, `PreHydrationConfig`, `HydratedContext` | Extraccion de referencias, resolucion en paralelo, poda por presupuesto de tokens |
| `knowledge_bridge` | (extendido) | `retrieve_scoped_conventions()`, herramienta `recall_knowledge` extendida |

---

## Proximos Pasos

- **[Guia del Bucle de Razonamiento](reasoning-loop.md)** — Documentacion del ciclo ORGA principal
- **[Arquitectura del Runtime](runtime-architecture.md)** — Vision general completa de la arquitectura del sistema
- **[Referencia de API](api-reference.md)** — Documentacion completa de la API

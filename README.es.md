<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | **Español** | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**Runtime de agentes gobernado por políticas para producción.**

Symbiont es un runtime nativo de Rust para ejecutar agentes de IA, herramientas y flujos de trabajo bajo controles explícitos de políticas, identidad y auditoría.

La mayoría de los frameworks de agentes se centran en la orquestación. Symbiont se centra en lo que sucede cuando los agentes necesitan ejecutarse en entornos reales con riesgo real: herramientas no confiables, datos sensibles, límites de aprobación, requisitos de auditoría y aplicación repetible de reglas.

---

## Por qué Symbiont

Los agentes de IA son fáciles de demostrar y difíciles de confiar.

Una vez que un agente puede llamar herramientas, acceder a archivos, enviar mensajes o invocar servicios externos, necesitas más que prompts y código improvisado. Necesitas:

* **Aplicación de políticas** para lo que un agente puede hacer — DSL integrado y autorización [Cedar](https://www.cedarpolicy.com/)
* **Verificación de herramientas** para que la ejecución no sea confianza ciega — verificación criptográfica de herramientas MCP con [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin)
* **Identidad de agentes** para saber quién está actuando — identidad ES256 anclada al dominio con [AgentPin](https://github.com/ThirdKeyAI/AgentPin)
* **Sandboxing** para cargas de trabajo riesgosas — aislamiento Docker con límites de recursos
* **Rastros de auditoría** de lo que sucedió y por qué — logs criptográficamente resistentes a manipulación
* **Flujos de revisión** para acciones que requieren aprobación — puertas de supervisión humana en el ciclo de razonamiento

Symbiont está construido para esa capa.

---

## Inicio rápido

### Prerrequisitos

* Docker (recomendado) o Rust 1.82+
* No se requiere base de datos vectorial externa (LanceDB integrado; Qdrant opcional para despliegues a escala)

### Ejecutar con Docker

```bash
# Iniciar el runtime (API en :8080, entrada HTTP en :8081)
docker run --rm -p 8080:8080 -p 8081:8081 ghcr.io/thirdkeyai/symbi:latest up

# Ejecutar solo el servidor MCP
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Parsear un archivo DSL de agente
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl
```

### Construir desde código fuente

```bash
cargo build --release
./target/release/symbi --help

# Ejecutar el runtime
cargo run -- up

# REPL interactivo
cargo run -- repl
```

> Para despliegues en producción, revisa `SECURITY.md` y la [guía de despliegue](https://docs.symbiont.dev/getting-started) antes de habilitar la ejecución de herramientas no confiables.

---

## Cómo funciona

Symbiont separa la intención del agente de la autoridad de ejecución:

1. **Los agentes proponen** acciones a través del ciclo de razonamiento ORGA (Observe-Reason-Gate-Act)
2. **El runtime evalúa** cada acción contra controles de políticas, identidad y confianza
3. **La política decide** — las acciones permitidas se ejecutan; las denegadas se bloquean o se envían para aprobación
4. **Todo queda registrado** — rastro de auditoría resistente a manipulación para cada decisión

Esto significa que la salida del modelo nunca se trata como autoridad de ejecución. El runtime controla lo que realmente sucede.

### Ejemplo: herramienta no confiable bloqueada por política

Un agente intenta llamar a una herramienta MCP no verificada. El runtime:

1. Verifica el estado de verificación SchemaPin — la firma de la herramienta falta o es inválida
2. Evalúa la política Cedar — `forbid(action == Action::"tool_call") when { !resource.verified }`
3. Bloquea la ejecución y registra la denegación con contexto completo
4. Opcionalmente envía a un operador para aprobación manual

No se requiere cambio de código. La política gobierna la ejecución.

---

## Ejemplo de DSL

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]

    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }

    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

---

## Capacidades principales

| Capacidad | Qué hace |
|-----------|----------|
| **Cedar policy engine** | Autorización granular para acciones de agentes, llamadas a herramientas y acceso a recursos |
| **Verificación SchemaPin** | Verificación criptográfica de esquemas de herramientas MCP antes de la ejecución |
| **Identidad AgentPin** | Identidad ES256 anclada al dominio para agentes y tareas programadas |
| **Ciclo de razonamiento ORGA** | Ciclo Observe-Reason-Gate-Act con estado tipificado, puertas de política y circuit breakers |
| **Sandboxing** | Aislamiento basado en Docker con límites de recursos para cargas de trabajo no confiables |
| **Registro de auditoría** | Logs resistentes a manipulación con registros estructurados para cada decisión de política |
| **Escaneo ClawHavoc** | 40 reglas en 10 categorías de ataque para análisis de contenido de skills/herramientas |
| **Gestión de secretos** | Integración Vault/OpenBao, almacenamiento cifrado AES-256-GCM, con alcance por agente |
| **Programación cron** | Programador respaldado por SQLite con jitter, guardas de concurrencia y colas de mensajes fallidos |
| **Memoria persistente** | Memoria de agente basada en Markdown con extracción de hechos, procedimientos y compactación |
| **Motor RAG** | Búsqueda híbrida semántica + palabra clave vía LanceDB (integrado) o Qdrant (escalado) |
| **Integración MCP** | Soporte nativo del Model Context Protocol con acceso gobernado a herramientas |
| **Verificación de webhooks** | Verificación HMAC-SHA256 y JWT con presets para GitHub, Stripe y Slack |
| **Enrutamiento de entregas** | Envía la salida del agente a webhooks, Slack, email o canales personalizados |
| **Métricas y telemetría** | Exportación OTLP con spans de trazado OpenTelemetry para el ciclo de razonamiento |
| **Seguridad HTTP** | Enlace solo a loopback, listas de permitidos CORS, validación JWT EdDSA, claves API por agente |
| **Plugins para asistentes IA** | Plugins de gobernanza para [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) y [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli) |

Rendimiento: evaluación de políticas <1ms, verificación ECDSA P-256 <5ms, programación de 10k agentes con <2% de uso de CPU. Ver [benchmarks](crates/runtime/benches/performance_claims.rs) y [pruebas de umbral](crates/runtime/tests/performance_claims.rs).

---

## Modelo de seguridad

Symbiont está diseñado en torno a un principio simple: **la salida del modelo nunca debe tratarse como autoridad de ejecución.**

Las acciones fluyen a través de controles del runtime:

* **Confianza cero** — todas las entradas de agentes son no confiables por defecto
* **Verificación de políticas** — autorización Cedar antes de cada llamada a herramienta y acceso a recursos
* **Verificación de herramientas** — verificación criptográfica SchemaPin de esquemas de herramientas
* **Límites de sandbox** — aislamiento Docker para ejecución no confiable
* **Aprobación del operador** — puertas de supervisión humana para acciones sensibles
* **Control de secretos** — backends Vault/OpenBao, almacenamiento local cifrado, namespaces de agentes
* **Registro de auditoría** — registros criptográficamente resistentes a manipulación de cada decisión

Si estás ejecutando código no confiable o herramientas riesgosas, no dependas de un modelo de ejecución local débil como tu única barrera. Consulta [`SECURITY.md`](SECURITY.md) y la [documentación del modelo de seguridad](https://docs.symbiont.dev/security-model).

---

## Workspace

| Crate | Descripción |
|-------|-------------|
| `symbi` | Binario CLI unificado |
| `symbi-runtime` | Runtime principal de agentes y motor de ejecución |
| `symbi-dsl` | Parser y evaluador del DSL |
| `symbi-channel-adapter` | Adaptadores para Slack/Teams/Mattermost |
| `repl-core` / `repl-proto` / `repl-cli` | REPL interactivo y servidor JSON-RPC |
| `repl-lsp` | Soporte del Language Server Protocol |
| `symbi-a2ui` | Panel de administración (Lit/TypeScript, alpha) |

Plugins de gobernanza: [`symbi-claude-code`](https://github.com/thirdkeyai/symbi-claude-code) | [`symbi-gemini-cli`](https://github.com/thirdkeyai/symbi-gemini-cli)

---

## Documentación

* [Primeros pasos](https://docs.symbiont.dev/getting-started)
* [Modelo de seguridad](https://docs.symbiont.dev/security-model)
* [Arquitectura del runtime](https://docs.symbiont.dev/runtime-architecture)
* [Guía del ciclo de razonamiento](https://docs.symbiont.dev/reasoning-loop)
* [Guía del DSL](https://docs.symbiont.dev/dsl-guide)
* [Referencia de la API](https://docs.symbiont.dev/api-reference)
* [Primitivas avanzadas de razonamiento](https://docs.symbiont.dev/orga-adaptive)

Si estás evaluando Symbiont para producción, comienza con la documentación del modelo de seguridad y primeros pasos.

---

## Licencia

* **Community Edition** (Apache 2.0): Runtime principal, DSL, ciclo de razonamiento ORGA, Cedar policy engine, verificación SchemaPin/AgentPin, sandboxing Docker, memoria persistente, programación cron, integración MCP, RAG (LanceDB), registro de auditoría, verificación de webhooks, escaneo ClawHavoc de skills, y todas las herramientas CLI/REPL.
* **Enterprise Edition** (licencia comercial): Sandboxing multi-nivel (gVisor, Firecracker, E2B), rastros de auditoría criptográficos con exportaciones de cumplimiento (HIPAA, SOX, PCI-DSS), revisión de herramientas y detección de amenazas con IA, colaboración multi-agente cifrada, paneles de monitoreo en tiempo real y soporte dedicado.
Contacta a [ThirdKey](https://thirdkey.ai) para licenciamiento empresarial.

---

*El mismo agente. Runtime seguro.*

<div align="right">
  <img src="symbi-trans.png" alt="Logo de Symbi" width="120">
</div>

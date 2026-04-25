<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | **Español** | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**Runtime de agentes gobernado por políticas para producción.**
*El mismo agente. Runtime seguro.*

Symbiont es un runtime nativo de Rust para ejecutar agentes de IA y herramientas bajo controles explícitos de políticas, identidad y auditoría.

La mayoría de los frameworks de agentes se centran en la orquestación. Symbiont se centra en lo que sucede cuando los agentes necesitan ejecutarse en entornos reales con riesgo real: herramientas no confiables, datos sensibles, límites de aprobación, requisitos de auditoría y aplicación repetible de reglas.

---

## Por qué Symbiont

Los agentes de IA son fáciles de demostrar y difíciles de confiar.

Una vez que un agente puede llamar herramientas, acceder a archivos, enviar mensajes o invocar servicios externos, necesitas más que prompts y código improvisado. Necesitas:

* **Aplicación de políticas** para lo que un agente puede hacer — DSL integrado y autorización [Cedar](https://www.cedarpolicy.com/)
* **Verificación de herramientas** para que la ejecución no sea confianza ciega — verificación criptográfica de herramientas MCP con [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin)
* **Contratos de herramientas** para regular cómo se ejecutan — [ToolClad](https://github.com/ThirdKeyAI/ToolClad) con validación declarativa de argumentos, aplicación de scope y prevención de inyección
* **Identidad de agentes** para saber quién está actuando — identidad ES256 anclada al dominio con [AgentPin](https://github.com/ThirdKeyAI/AgentPin)
* **Sandboxing** para cargas de trabajo riesgosas — aislamiento Docker con límites de recursos
* **Rastros de auditoría** de lo que sucedió y por qué — logs criptográficamente resistentes a manipulación
* **Puertas de aprobación** para acciones sensibles — revisión humana antes de la ejecución cuando la política lo requiere

Symbiont está construido para esa capa.

---

## Inicio rápido

### Prerrequisitos

* Docker (recomendado) o Rust 1.82+

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

1. **Los agentes proponen** acciones a través del ciclo de razonamiento (Observe-Reason-Gate-Act)
2. **El runtime evalúa** cada acción contra controles de políticas, identidad y confianza
3. **La política decide** — las acciones permitidas se ejecutan; las denegadas se bloquean o se envían para aprobación
4. **Todo queda registrado** — rastro de auditoría resistente a manipulación para cada decisión

La salida del modelo nunca se trata como autoridad de ejecución. El runtime controla lo que realmente sucede.

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
agent secure_analyst(input: DataSet) -> Result {
    policy access_control {
        allow: read(input) if input.verified == true
        deny: send_email without approval
        audit: all_operations
    }

    with memory = "persistent", requires = "approval" {
        result = analyze(input);
        return result;
    }
}
```

Consulta la [guía del DSL](https://docs.symbiont.dev/dsl-guide) para la gramática completa incluyendo bloques `metadata`, `schedule`, `webhook` y `channel`.

---

## Capacidades principales

| Capacidad | Qué hace |
|-----------|----------|
| **Motor de políticas** | Autorización granular [Cedar](https://www.cedarpolicy.com/) para acciones de agentes, llamadas a herramientas y acceso a recursos |
| **Verificación de herramientas** | Verificación criptográfica [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) de esquemas de herramientas MCP antes de la ejecución |
| **Contratos de herramientas** | Contratos declarativos [ToolClad](https://github.com/ThirdKeyAI/ToolClad) con validación de argumentos, aplicación de scope y generación de políticas Cedar |
| **Identidad de agentes** | Identidad ES256 anclada al dominio [AgentPin](https://github.com/ThirdKeyAI/AgentPin) para agentes y tareas programadas |
| **Ciclo de razonamiento** | Ciclo Observe-Reason-Gate-Act con estado tipificado, puertas de política y circuit breakers |
| **Sandboxing** | Aislamiento basado en Docker con límites de recursos para cargas de trabajo no confiables |
| **Registro de auditoría** | Logs resistentes a manipulación con registros estructurados para cada decisión de política |
| **Gestión de secretos** | Integración Vault/OpenBao, almacenamiento cifrado AES-256-GCM, con alcance por agente |
| **Integración MCP** | Soporte nativo del Model Context Protocol con acceso gobernado a herramientas |

Capacidades adicionales: escaneo de amenazas para contenido de herramientas/skills (40 reglas, 10 categorías de ataque), programación cron, memoria persistente de agentes, búsqueda RAG híbrida (LanceDB/Qdrant), verificación de webhooks, enrutamiento de entregas, telemetría OTLP, hardening de seguridad HTTP, y plugins de gobernanza para [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) y [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli). Consulta la [documentación completa](https://docs.symbiont.dev) para más detalles.

Los benchmarks representativos están disponibles en el [harness de benchmarks](crates/runtime/benches/performance_claims.rs) y las [pruebas de umbral](crates/runtime/tests/performance_claims.rs).

---

## Modelo de seguridad

Symbiont está diseñado en torno a un principio simple: **la salida del modelo nunca debe tratarse como autoridad de ejecución.**

Las acciones fluyen a través de controles del runtime:

* **Confianza cero** — todas las entradas de agentes son no confiables por defecto
* **Verificación de políticas** — autorización Cedar antes de cada llamada a herramienta y acceso a recursos
* **Verificación de herramientas** — verificación criptográfica SchemaPin de esquemas de herramientas
* **Límites de sandbox** — aislamiento Docker para ejecución no confiable
* **Aprobación del operador** — puertas de revisión humana para acciones sensibles
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
| `symbi-shell` | TUI interactiva para autoría, orquestación y attach remoto (beta) |
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

Si estás evaluando Symbiont para producción, comienza con la documentación del modelo de seguridad y primeros pasos.

---

## SDKs

SDKs oficiales para integrar el runtime de Symbiont desde tu aplicación:

| Lenguaje | Paquete | Repositorio |
|----------|---------|-------------|
| **JavaScript/TypeScript** | [symbiont-sdk-js](https://www.npmjs.com/package/symbiont-sdk-js) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-js) |
| **Python** | [symbiont-sdk](https://pypi.org/project/symbiont-sdk/) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-python) |

---

## Licencia

* **Community Edition** (Apache 2.0): Runtime principal, DSL, motor de políticas, verificación de herramientas, sandboxing, memoria de agentes, programación, integración MCP, RAG, registro de auditoría, y todas las herramientas CLI/REPL.
* **Enterprise Edition** (comercial): Backends de sandbox avanzados, exportaciones de auditoría de cumplimiento, revisión de herramientas con IA, colaboración multi-agente cifrada, paneles de monitoreo y soporte dedicado.

Contacta a [ThirdKey](https://thirdkey.ai) para licenciamiento empresarial.

---

<div align="right">
  <img src="symbi-trans.png" alt="Logo de Symbi" width="120">
</div>

---
layout: default
title: Primeros Pasos
description: "Guia de inicio rapido para Symbiont"
nav_exclude: true
---

# Primeros Pasos
{: .no_toc }

## 🌐 Otros idiomas
{: .no_toc}

[English](getting-started.md) | [中文简体](getting-started.zh-cn.md) | **Español** | [Português](getting-started.pt.md) | [日本語](getting-started.ja.md) | [Deutsch](getting-started.de.md)

---

Esta guia te guiara a traves de la configuracion de Symbi y la creacion de tu primer agente de IA.
{: .fs-6 .fw-300 }

## Tabla de contenidos
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Prerrequisitos

Antes de comenzar con Symbi, asegurate de tener lo siguiente instalado:

### Dependencias Requeridas

- **Docker** (para desarrollo containerizado)
- **Rust 1.88+** (si construyes localmente)
- **Git** (para clonar el repositorio)

### Dependencias Opcionales

- **SchemaPin Go CLI** (para verificacion de herramientas)

> **Nota:** La busqueda vectorial esta integrada. Symbi incluye [LanceDB](https://lancedb.com/) como base de datos vectorial embebida -- no se necesita ningun servicio externo.

---

## Instalacion

### Opcion 1: Docker (Recomendado)

La forma mas rapida de empezar es usando Docker:

```bash
# Clone the repository
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Build the unified symbi container
docker build -t symbi:latest .

# Or use pre-built container
docker pull ghcr.io/thirdkeyai/symbi:latest

# Run the development environment
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Opcion 2: Instalacion Local

Para desarrollo local:

```bash
# Clone the repository
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Install Rust dependencies and build
cargo build --release

# Run tests to verify installation
cargo test
```

### Verificar Instalacion

Probar que todo funciona correctamente:

```bash
# Test the DSL parser
cd crates/dsl && cargo run && cargo test

# Test the runtime system
cd ../runtime && cargo test

# Run example agents
cargo run --example basic_agent
cargo run --example full_system

# Test the unified symbi CLI
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Test with Docker container
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## Tu Primer Agente

Vamos a crear un agente simple de analisis de datos para entender los conceptos basicos de Symbi.

### 1. Crear Definicion de Agente

Crear un nuevo archivo `my_agent.dsl`:

```rust
metadata {
    version = "1.0.0"
    author = "your-name"
    description = "My first Symbi agent"
}

agent greet_user(name: String) -> String {
    capabilities = ["greeting", "text_processing"]

    policy safe_greeting {
        allow: read(name) if name.length <= 100
        deny: store(name) if name.contains_sensitive_data
        audit: all_operations with signature
    }

    with memory = "ephemeral", privacy = "low" {
        if (validate_name(name)) {
            greeting = format_greeting(name);
            audit_log("greeting_generated", greeting.metadata);
            return greeting;
        } else {
            return "Hello, anonymous user!";
        }
    }
}
```

### 2. Ejecutar el Agente

```bash
# Parse and validate the agent definition
cargo run -- dsl parse my_agent.dsl

# Run the agent in the runtime
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.dsl
```

---

## Entendiendo el DSL

El DSL de Symbi tiene varios componentes clave:

### Bloque de Metadatos

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

Proporciona informacion esencial sobre tu agente para documentacion y gestion del runtime.

### Definicion de Agente

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // agent implementation
}
```

Define la interfaz, capacidades y comportamiento del agente.

### Definiciones de Politicas

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Politicas de seguridad declarativas que se aplican en tiempo de ejecucion.

### Contexto de Ejecucion

```rust
with memory = "persistent", privacy = "high" {
    // agent implementation
}
```

Especifica la configuracion de runtime para gestion de memoria y requisitos de privacidad.

---

## Siguientes Pasos

### Explorar Ejemplos

El repositorio incluye varios agentes de ejemplo:

```bash
# Basic agent example
cd crates/runtime && cargo run --example basic_agent

# Full system demonstration
cd crates/runtime && cargo run --example full_system

# Context and memory example
cd crates/runtime && cargo run --example context_example

# RAG-powered agent
cd crates/runtime && cargo run --example rag_example
```

### Habilitar Funciones Avanzadas

#### API HTTP (Opcional)

```bash
# Enable the HTTP API feature
cd crates/runtime && cargo build --features http-api

# Run with API endpoints
cd crates/runtime && cargo run --features http-api --example full_system
```

**Endpoints de API Principales:**
- `GET /api/v1/health` - Verificacion de salud y estado del sistema
- `GET /api/v1/agents` - Listar todos los agentes activos con estado de ejecucion en tiempo real
- `GET /api/v1/agents/{id}/status` - Obtener metricas detalladas de ejecucion del agente
- `POST /api/v1/workflows/execute` - Ejecutar flujos de trabajo

**Nuevas Funciones de Gestion de Agentes:**
- Monitoreo de procesos en tiempo real y verificaciones de salud
- Capacidades de apagado gracioso para agentes en ejecucion
- Metricas de ejecucion completas y seguimiento de uso de recursos
- Soporte para diferentes modos de ejecucion (efimero, persistente, programado, basado en eventos)

#### Inferencia LLM en la Nube

Conecta a proveedores de LLM en la nube via OpenRouter:

```bash
# Enable cloud inference
cargo build --features cloud-llm

# Set API key and model
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # optional
```

#### Modo Agente Autonomo

Una sola linea para agentes nativos de la nube con inferencia LLM y acceso a herramientas Composio:

```bash
cargo build --features standalone-agent
# Enables: cloud-llm + composio
```

#### Primitivas de Razonamiento Avanzado

Habilita curacion de herramientas, deteccion de bucles atascados, pre-carga de contexto y convenciones con alcance:

```bash
cargo build --features symbi-dev
```

Consulta la [guia de symbi-dev](/symbi-dev) para la documentacion completa.

#### Motor de Politicas Cedar

Autorizacion formal con el lenguaje de politicas Cedar:

```bash
cargo build --features cedar
```

#### Base de Datos Vectorial (Integrada)

Symbi incluye LanceDB como base de datos vectorial embebida sin configuracion. La busqueda semantica y RAG funcionan de inmediato -- no se necesita iniciar ningun servicio separado:

```bash
# Run agent with RAG capabilities (vector search just works)
cd crates/runtime && cargo run --example rag_example

# Test context management with advanced search
cd crates/runtime && cargo run --example context_example
```

> **Opcion enterprise:** Para equipos que necesiten una base de datos vectorial dedicada, Qdrant esta disponible como backend opcional con feature gate. Configura `SYMBIONT_VECTOR_BACKEND=qdrant` y `QDRANT_URL` para usarlo.

**Funciones de Gestion de Contexto:**
- **Busqueda Multi-Modal**: Modos de busqueda por palabra clave, temporal, similitud e hibrido
- **Calculo de Importancia**: Algoritmo de puntuacion sofisticado considerando patrones de acceso, recencia y retroalimentacion del usuario
- **Control de Acceso**: Integracion del motor de politicas con controles de acceso por agente
- **Archivado Automatico**: Politicas de retencion con almacenamiento comprimido y limpieza
- **Compartir Conocimiento**: Comparticion segura de conocimiento entre agentes con puntuaciones de confianza

#### Referencia de Feature Flags

| Feature | Descripcion | Por defecto |
|---------|-------------|-------------|
| `keychain` | Integracion de llavero del SO para secretos | Si |
| `vector-lancedb` | Backend vectorial embebido LanceDB | Si |
| `vector-qdrant` | Backend vectorial distribuido Qdrant | No |
| `embedding-models` | Modelos de embedding locales via Candle | No |
| `http-api` | API REST con Swagger UI | No |
| `http-input` | Servidor webhook con autenticacion JWT | No |
| `cloud-llm` | Inferencia LLM en la nube (OpenRouter) | No |
| `composio` | Integracion de herramientas Composio MCP | No |
| `standalone-agent` | Combo Cloud LLM + Composio | No |
| `cedar` | Motor de politicas Cedar | No |
| `symbi-dev` | Primitivas de razonamiento avanzado | No |
| `cron` | Programacion cron persistente | No |
| `native-sandbox` | Sandboxing nativo de procesos | No |
| `metrics` | Metricas/trazado OpenTelemetry | No |
| `full` | Todas las funciones excepto enterprise | No |

```bash
# Build with specific features
cargo build --features "cloud-llm,symbi-dev,cedar"

# Build with everything
cargo build --features full
```

---

## Configuracion

### Variables de Entorno

Configura tu entorno para rendimiento optimo:

```bash
# Basic configuration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vector search works out of the box with the built-in LanceDB backend.
# To use Qdrant instead (optional, enterprise):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP integration (optional)
export MCP_SERVER_URLS="http://localhost:8080"
```

### Configuracion de Runtime

Crear un archivo de configuracion `symbi.toml`:

```toml
[runtime]
max_agents = 1000
memory_limit_mb = 512
execution_timeout_seconds = 300

[security]
default_sandbox_tier = "docker"
audit_enabled = true
policy_enforcement = "strict"

[vector_db]
enabled = true
backend = "lancedb"              # default; also supports "qdrant"
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # only needed when backend = "qdrant"
```

---

## Problemas Comunes

### Problemas con Docker

**Problema**: La construccion de Docker falla con errores de permisos
```bash
# Solution: Ensure Docker daemon is running and user has permissions
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problema**: El contenedor sale inmediatamente
```bash
# Solution: Check Docker logs
docker logs <container_id>
```

### Problemas de Construccion con Rust

**Problema**: La construccion de Cargo falla con errores de dependencias
```bash
# Solution: Update Rust and clean build cache
rustup update
cargo clean
cargo build
```

**Problema**: Faltan dependencias del sistema
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### Problemas de Runtime

**Problema**: El agente falla al iniciar
```bash
# Check agent definition syntax
cargo run -- dsl parse your_agent.dsl

# Enable debug logging
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Obtener Ayuda

### Documentacion

- **[Guia DSL](/dsl-guide)** - Referencia completa del DSL
- **[Arquitectura del Runtime](/runtime-architecture)** - Detalles de arquitectura del sistema
- **[Modelo de Seguridad](/security-model)** - Documentacion de seguridad y politicas

### Soporte de la Comunidad

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discusiones**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Documentacion**: [Referencia Completa de API](https://docs.symbiont.dev/api-reference)

### Modo de Depuracion

Para solucion de problemas, habilitar logging detallado:

```bash
# Enable debug logging
export RUST_LOG=symbi=debug

# Run with detailed output
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## ¿Que Sigue?

Ahora que tienes Symbi ejecutandose, explora estos temas avanzados:

1. **[Guia DSL](/dsl-guide)** - Aprende funciones avanzadas del DSL
2. **[Guia del Bucle de Razonamiento](/reasoning-loop)** - Entiende el ciclo ORGA
3. **[Razonamiento Avanzado (symbi-dev)](/symbi-dev)** - Curacion de herramientas, deteccion de bucles atascados, pre-hidratacion
4. **[Arquitectura del Runtime](/runtime-architecture)** - Entiende los internos del sistema
5. **[Modelo de Seguridad](/security-model)** - Implementa politicas de seguridad
6. **[Contribuir](/contributing)** - Contribuye al proyecto

¿Listo para construir algo increible? Comienza con nuestros [proyectos de ejemplo](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) o sumergete en la [especificacion completa](/specification).

---
layout: default
title: Primeros Pasos
description: "Guía de inicio rápido para Symbiont"
---

# Primeros Pasos
{: .no_toc }

## 🌐 Otros idiomas
{: .no_toc}

[English](getting-started.md) | [中文简体](getting-started.zh-cn.md) | **Español** | [Português](getting-started.pt.md) | [日本語](getting-started.ja.md) | [Deutsch](getting-started.de.md)

---

Esta guía te guiará a través de la configuración de Symbi y la creación de tu primer agente de IA.
{: .fs-6 .fw-300 }

## Tabla de contenidos
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Prerrequisitos

Antes de comenzar con Symbi, asegúrate de tener lo siguiente instalado:

### Dependencias Requeridas

- **Docker** (para desarrollo containerizado)
- **Rust 1.88+** (si construyes localmente)
- **Git** (para clonar el repositorio)

### Dependencias Opcionales

- **Qdrant** base de datos vectorial (para capacidades de búsqueda semántica)
- **SchemaPin Go CLI** (para verificación de herramientas)

---

## Instalación

### Opción 1: Docker (Recomendado)

La forma más rápida de empezar es usando Docker:

```bash
# Clonar el repositorio
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Construir el contenedor unificado symbi
docker build -t symbi:latest .

# O usar contenedor pre-construido
docker pull ghcr.io/thirdkeyai/symbi:latest

# Ejecutar el entorno de desarrollo
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Opción 2: Instalación Local

Para desarrollo local:

```bash
# Clonar el repositorio
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Instalar dependencias de Rust y construir
cargo build --release

# Ejecutar pruebas para verificar la instalación
cargo test
```

### Verificar Instalación

Probar que todo funciona correctamente:

```bash
# Probar el analizador DSL
cd crates/dsl && cargo run && cargo test

# Probar el sistema de runtime
cd ../runtime && cargo test

# Ejecutar agentes de ejemplo
cargo run --example basic_agent
cargo run --example full_system

# Probar el CLI unificado symbi
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Probar con contenedor Docker
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## Tu Primer Agente

Vamos a crear un agente simple de análisis de datos para entender los conceptos básicos de Symbi.

### 1. Crear Definición de Agente

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
# Analizar y validar la definición del agente
cargo run -- dsl parse my_agent.dsl

# Ejecutar el agente en el runtime
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

Proporciona información esencial sobre tu agente para documentación y gestión del runtime.

### Definición de Agente

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // implementación del agente
}
```

Define la interfaz, capacidades y comportamiento del agente.

### Definiciones de Políticas

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Políticas de seguridad declarativas que se aplican en tiempo de ejecución.

### Contexto de Ejecución

```rust
with memory = "persistent", privacy = "high" {
    // implementación del agente
}
```

Especifica la configuración de runtime para gestión de memoria y requisitos de privacidad.

---

## Siguientes Pasos

### Explorar Ejemplos

El repositorio incluye varios agentes de ejemplo:

```bash
# Ejemplo de agente básico
cd crates/runtime && cargo run --example basic_agent

# Demostración completa del sistema
cd crates/runtime && cargo run --example full_system

# Ejemplo de contexto y memoria
cd crates/runtime && cargo run --example context_example

# Agente potenciado por RAG
cd crates/runtime && cargo run --example rag_example
```

### Habilitar Funciones Avanzadas

#### API HTTP (Opcional)

```bash
# Habilitar la función de API HTTP
cd crates/runtime && cargo build --features http-api

# Ejecutar con endpoints de API
cd crates/runtime && cargo run --features http-api --example full_system
```

**Endpoints de API Principales:**
- `GET /api/v1/health` - Verificación de salud y estado del sistema
- `GET /api/v1/agents` - Listar todos los agentes activos
- `POST /api/v1/workflows/execute` - Ejecutar flujos de trabajo

#### Integración de Base de Datos Vectorial

Para capacidades de búsqueda semántica:

```bash
# Iniciar base de datos vectorial Qdrant
docker run -p 6333:6333 qdrant/qdrant

# Ejecutar agente con capacidades RAG
cd crates/runtime && cargo run --example rag_example
```

---

## Configuración

### Variables de Entorno

Configurar tu entorno para rendimiento óptimo:

```bash
# Configuración básica
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Base de datos vectorial (opcional)
export QDRANT_URL=http://localhost:6333

# Integración MCP (opcional)
export MCP_SERVER_URLS="http://localhost:8080"
```

### Configuración de Runtime

Crear un archivo de configuración `symbi.toml`:

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
url = "http://localhost:6333"
collection_name = "symbi_knowledge"
```

---

## Problemas Comunes

### Problemas con Docker

**Problema**: La construcción de Docker falla con errores de permisos
```bash
# Solución: Asegurar que el daemon de Docker esté ejecutándose y el usuario tenga permisos
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problema**: El contenedor sale inmediatamente
```bash
# Solución: Revisar los logs de Docker
docker logs <container_id>
```

### Problemas de Construcción con Rust

**Problema**: La construcción de Cargo falla con errores de dependencias
```bash
# Solución: Actualizar Rust y limpiar caché de construcción
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
# Revisar sintaxis de definición del agente
cargo run -- dsl parse your_agent.dsl

# Habilitar logging de depuración
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Obtener Ayuda

### Documentación

- **[Guía DSL](/dsl-guide)** - Referencia completa del DSL
- **[Arquitectura de Runtime](/runtime-architecture)** - Detalles de arquitectura del sistema
- **[Modelo de Seguridad](/security-model)** - Documentación de seguridad y políticas

### Soporte de la Comunidad

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discusiones**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Documentación**: [Referencia Completa de API](https://docs.symbiont.platform)

### Modo de Depuración

Para solución de problemas, habilitar logging detallado:

```bash
# Habilitar logging de depuración
export RUST_LOG=symbi=debug

# Ejecutar con salida detallada
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## ¿Qué Sigue?

Ahora que tienes Symbi ejecutándose, explora estos temas avanzados:

1. **[Guía DSL](/dsl-guide)** - Aprende funciones avanzadas del DSL
2. **[Arquitectura de Runtime](/runtime-architecture)** - Entiende los internos del sistema
3. **[Modelo de Seguridad](/security-model)** - Implementa políticas de seguridad
4. **[Contribuir](/contributing)** - Contribuye al proyecto

¿Listo para construir algo increíble? Comienza con nuestros [proyectos de ejemplo](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) o sumérgete en la [especificación completa](/specification).
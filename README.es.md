<img src="logo-hz.png" alt="Symbi">

**Symbi** es un framework de agentes nativo de IA para construir agentes autónomos y conscientes de políticas que pueden colaborar de forma segura con humanos, otros agentes y modelos de lenguaje grandes. La edición Community proporciona funcionalidad central con características Enterprise opcionales para seguridad avanzada, monitoreo y colaboración.

## 🚀 Inicio Rápido

### Prerrequisitos
- Docker (recomendado) o Rust 1.88+
- Base de datos vectorial Qdrant (para búsqueda semántica)

### Ejecutar con Contenedores Pre-construidos

**Usando GitHub Container Registry (Recomendado):**

```bash
# Ejecutar CLI unificado de symbi
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# Ejecutar MCP Server
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Desarrollo interactivo
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### Construir desde el Código Fuente

```bash
# Construir entorno de desarrollo
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# Construir el binario unificado de symbi
cargo build --release

# Probar los componentes
cargo test

# Ejecutar agentes de ejemplo (desde crates/runtime)
cd crates/runtime && cargo run --example basic_agent
cd crates/runtime && cargo run --example full_system
cd crates/runtime && cargo run --example rag_example

# Usar el CLI unificado de symbi
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080

# Habilitar HTTP API (opcional)
cd crates/runtime && cargo run --features http-api --example full_system
```

### API HTTP Opcional

Habilitar API HTTP RESTful para integración externa:

```bash
# Construir con característica HTTP API
cargo build --features http-api

# O agregar a Cargo.toml
[dependencies]
symbi-runtime = { version = "0.1.2", features = ["http-api"] }
```

**Endpoints Principales:**
- `GET /api/v1/health` - Verificación de salud y estado del sistema
- `GET /api/v1/agents` - Listar todos los agentes activos
- `POST /api/v1/workflows/execute` - Ejecutar flujos de trabajo
- `GET /api/v1/metrics` - Métricas del sistema

## 📁 Estructura del Proyecto

```
symbi/
├── src/                   # Binario CLI unificado de symbi
├── crates/                # Crates del workspace
│   ├── dsl/              # Implementación del DSL de Symbi
│   │   ├── src/          # Código del analizador y biblioteca
│   │   ├── tests/        # Suite de pruebas del DSL
│   │   └── tree-sitter-symbiont/ # Definición de gramática
│   └── runtime/          # Sistema de Runtime de Agentes (Community)
│       ├── src/          # Componentes centrales del runtime
│       ├── examples/     # Ejemplos de uso
│       └── tests/        # Pruebas de integración
├── docs/                 # Documentación
└── Cargo.toml           # Configuración del workspace
```

## 🔧 Características

### ✅ Características Community (OSS)
- **Gramática DSL**: Gramática Tree-sitter completa para definiciones de agentes
- **Runtime de Agentes**: Programación de tareas, gestión de recursos, control del ciclo de vida
- **Aislamiento Tier 1**: Aislamiento containerizado con Docker para operaciones de agentes
- **Integración MCP**: Cliente del Protocolo de Contexto de Modelo para herramientas externas
- **Seguridad SchemaPin**: Verificación criptográfica básica de herramientas
- **Motor RAG**: Generación aumentada por recuperación con búsqueda vectorial
- **Gestión de Contexto**: Memoria persistente de agentes y almacenamiento de conocimiento
- **Base de Datos Vectorial**: Integración con Qdrant para búsqueda semántica
- **Gestión Integral de Secretos**: Integración con HashiCorp Vault con múltiples métodos de autenticación
- **Backend de Archivos Encriptados**: Encriptación AES-256-GCM con integración de llavero del OS
- **Herramientas CLI de Secretos**: Operaciones completas de encriptar/desencriptar/editar con pistas de auditoría
- **API HTTP**: Interfaz RESTful opcional (controlada por características)

### 🏢 Características Enterprise (Licencia Requerida)
- **Aislamiento Avanzado**: Aislamiento gVisor y Firecracker **(Enterprise)**
- **Revisión de Herramientas IA**: Flujo de trabajo de análisis de seguridad automatizado **(Enterprise)**
- **Auditoría Criptográfica**: Pistas de auditoría completas con firmas Ed25519 **(Enterprise)**
- **Comunicación Multi-Agente**: Mensajería encriptada entre agentes **(Enterprise)**
- **Monitoreo en Tiempo Real**: Métricas SLA y dashboards de rendimiento **(Enterprise)**
- **Servicios Profesionales y Soporte**: Desarrollo personalizado y soporte **(Enterprise)**

## 📐 DSL Symbiont

Define agentes inteligentes con políticas y capacidades integradas:

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

## 🔐 Gestión de Secretos

Symbi proporciona gestión de secretos de nivel empresarial con múltiples opciones de backend:

### Opciones de Backend
- **HashiCorp Vault**: Gestión de secretos lista para producción con múltiples métodos de autenticación
  - Autenticación basada en tokens
  - Autenticación de cuenta de servicio de Kubernetes
- **Archivos Encriptados**: Almacenamiento local encriptado AES-256-GCM con integración de llavero del OS
- **Espacios de Nombres de Agentes**: Acceso a secretos con alcance por agente para aislamiento

### Operaciones CLI
```bash
# Encriptar archivo de secretos
symbi secrets encrypt config.json --output config.enc

# Desencriptar archivo de secretos
symbi secrets decrypt config.enc --output config.json

# Editar secretos encriptados directamente
symbi secrets edit config.enc

# Configurar backend de Vault
symbi secrets configure vault --endpoint https://vault.company.com
```

### Auditoría y Cumplimiento
- Pistas de auditoría completas para todas las operaciones de secretos
- Verificación de integridad criptográfica
- Controles de acceso con alcance por agente
- Registro a prueba de manipulación

## 🔒 Modelo de Seguridad

### Seguridad Básica (Community)
- **Aislamiento Tier 1**: Ejecución de agentes containerizada con Docker
- **Verificación de Esquemas**: Validación criptográfica de herramientas con SchemaPin
- **Motor de Políticas**: Control básico de acceso a recursos
- **Gestión de Secretos**: Integración con Vault y almacenamiento de archivos encriptados
- **Registro de Auditoría**: Seguimiento de operaciones y cumplimiento

### Seguridad Avanzada (Enterprise)
- **Aislamiento Mejorado**: Aislamiento gVisor (Tier2) y Firecracker (Tier3) **(Enterprise)**
- **Revisión de Seguridad IA**: Análisis automatizado de herramientas y aprobación **(Enterprise)**
- **Comunicación Encriptada**: Mensajería segura entre agentes **(Enterprise)**
- **Auditorías Integrales**: Garantías de integridad criptográfica **(Enterprise)**

## 🧪 Pruebas

```bash
# Ejecutar todas las pruebas
cargo test

# Ejecutar componentes específicos
cd crates/dsl && cargo test          # Analizador DSL
cd crates/runtime && cargo test     # Sistema de runtime

# Pruebas de integración
cd crates/runtime && cargo test --test integration_tests
cd crates/runtime && cargo test --test rag_integration_tests
cd crates/runtime && cargo test --test mcp_client_tests
```

## 📚 Documentación

- **[Primeros Pasos](https://docs.symbiont.dev/getting-started)** - Instalación y primeros pasos
- **[Guía del DSL](https://docs.symbiont.dev/dsl-guide)** - Referencia completa del lenguaje
- **[Arquitectura del Runtime](https://docs.symbiont.dev/runtime-architecture)** - Diseño del sistema
- **[Modelo de Seguridad](https://docs.symbiont.dev/security-model)** - Implementación de seguridad
- **[Referencia de la API](https://docs.symbiont.dev/api-reference)** - Documentación completa de la API
- **[Contribuir](https://docs.symbiont.dev/contributing)** - Guías de desarrollo

### Referencias Técnicas
- [`crates/runtime/README.md`](crates/runtime/README.md) - Documentación específica del runtime
- [`crates/runtime/API_REFERENCE.md`](crates/runtime/API_REFERENCE.md) - Referencia completa de la API
- [`crates/dsl/README.md`](crates/dsl/README.md) - Detalles de implementación del DSL

## 🤝 Contribuir

¡Las contribuciones son bienvenidas! Por favor consulta [`docs/contributing.md`](docs/contributing.md) para las guías.

**Principios de Desarrollo:**
- Seguridad primero - todas las características deben pasar revisión de seguridad
- Confianza cero - asumir que todas las entradas son potencialmente maliciosas
- Pruebas integrales - mantener alta cobertura de pruebas
- Documentación clara - documentar todas las características y APIs

## 🎯 Casos de Uso

### Desarrollo y Automatización
- Generación segura de código y refactorización
- Pruebas automatizadas con cumplimiento de políticas
- Despliegue de agentes IA con verificación de herramientas
- Gestión de conocimiento con búsqueda semántica

### Empresas e Industrias Reguladas
- Procesamiento de datos de salud con cumplimiento HIPAA **(Enterprise)**
- Servicios financieros con requisitos de auditoría **(Enterprise)**
- Sistemas gubernamentales con autorizaciones de seguridad **(Enterprise)**
- Análisis de documentos legales con confidencialidad **(Enterprise)**

## 📄 Licencia

**Edición Community**: Licencia MIT  
**Edición Enterprise**: Licencia comercial requerida

Contacta a [ThirdKey](https://thirdkey.ai) para licenciamiento Enterprise.

## 🔗 Enlaces

- [Sitio Web de ThirdKey](https://thirdkey.ai)
- [Referencia de la API del Runtime](crates/runtime/API_REFERENCE.md)

---

*Symbi permite la colaboración segura entre agentes IA y humanos a través de la aplicación inteligente de políticas, verificación criptográfica y pistas de auditoría integrales.*

<div align="right">
  <img src="symbi-trans.png" alt="Logo Transparente de Symbi" width="120">
</div>
---
layout: default
title: Guia del REPL
nav_exclude: true
---

# Guia del REPL de Symbiont

## Otros idiomas
{: .no_toc}

[English](repl-guide.md) | [中文简体](repl-guide.zh-cn.md) | **Español** | [Português](repl-guide.pt.md) | [日本語](repl-guide.ja.md) | [Deutsch](repl-guide.de.md)

---

El REPL (Read-Eval-Print Loop) de Symbiont proporciona un entorno interactivo para desarrollar, probar y depurar agentes de Symbiont y codigo DSL.

## Caracteristicas

- **Evaluacion Interactiva de DSL**: Ejecutar codigo DSL de Symbiont en tiempo real
- **Gestion del Ciclo de Vida de Agentes**: Crear, iniciar, detener, pausar, reanudar y destruir agentes
- **Monitoreo de Ejecucion**: Monitoreo en tiempo real de la ejecucion de agentes con estadisticas y trazas
- **Aplicacion de Politicas**: Verificacion de politicas y control de capacidades integrados
- **Gestion de Sesiones**: Crear instantaneas y restaurar sesiones del REPL
- **Protocolo JSON-RPC**: Acceso programatico via JSON-RPC sobre stdio
- **Soporte LSP**: Language Server Protocol para integracion con IDEs

## Primeros Pasos

### Iniciar el REPL

```bash
# Interactive REPL mode
symbi repl

# JSON-RPC server mode (for IDE integration)
symbi repl --json-rpc

# With custom configuration
symbi repl --config custom-config.toml
```

### Uso Basico

```rust
# Define an agent
agent GreetingAgent {
  name: "Greeting Agent"
  version: "1.0.0"
  description: "A simple greeting agent"
}

# Define a behavior
behavior Greet {
  input { name: string }
  output { greeting: string }
  steps {
    let greeting = format("Hello, {}!", name)
    return greeting
  }
}

# Execute expressions
let message = "Welcome to Symbiont"
print(message)
```

## Comandos del REPL

### Gestion de Agentes

| Comando | Descripcion |
|---------|-------------|
| `:agents` | Listar todos los agentes |
| `:agent list` | Listar todos los agentes |
| `:agent start <id>` | Iniciar un agente |
| `:agent stop <id>` | Detener un agente |
| `:agent pause <id>` | Pausar un agente |
| `:agent resume <id>` | Reanudar un agente pausado |
| `:agent destroy <id>` | Destruir un agente |
| `:agent execute <id> <behavior> [args]` | Ejecutar comportamiento de un agente |
| `:agent debug <id>` | Mostrar informacion de depuracion de un agente |

### Comandos de Monitoreo

| Comando | Descripcion |
|---------|-------------|
| `:monitor stats` | Mostrar estadisticas de ejecucion |
| `:monitor traces [limit]` | Mostrar trazas de ejecucion |
| `:monitor report` | Mostrar informe detallado de ejecucion |
| `:monitor clear` | Limpiar datos de monitoreo |

### Comandos de Sesion

| Comando | Descripcion |
|---------|-------------|
| `:snapshot` | Crear una instantanea de la sesion |
| `:clear` | Limpiar la sesion |
| `:help` o `:h` | Mostrar mensaje de ayuda |
| `:version` | Mostrar informacion de version |

## Caracteristicas del DSL

### Definiciones de Agentes

```rust
agent DataAnalyzer {
  name: "Data Analysis Agent"
  version: "2.1.0"
  description: "Analyzes datasets with privacy protection"

  security {
    capabilities: ["data_read", "analysis"]
    sandbox: true
  }

  resources {
    memory: 512MB
    cpu: 2
    storage: 1GB
  }
}
```

### Definiciones de Comportamiento

```rust
behavior AnalyzeData {
  input {
    data: DataSet
    options: AnalysisOptions
  }
  output {
    results: AnalysisResults
  }

  steps {
    # Check data privacy requirements
    require capability("data_read")

    if (data.contains_pii) {
      return error("Cannot process data with PII")
    }

    # Perform analysis
    let results = analyze(data, options)
    emit analysis_completed { results: results }

    return results
  }
}
```

### Funciones Integradas

| Funcion | Descripcion | Ejemplo |
|---------|-------------|---------|
| `print(...)` | Imprimir valores en la salida | `print("Hello", name)` |
| `len(value)` | Obtener la longitud de una cadena, lista o mapa | `len("hello")` -> `5` |
| `upper(string)` | Convertir cadena a mayusculas | `upper("hello")` -> `"HELLO"` |
| `lower(string)` | Convertir cadena a minusculas | `lower("HELLO")` -> `"hello"` |
| `format(template, ...)` | Formatear cadena con argumentos | `format("Hello, {}!", name)` |

### Tipos de Datos

```rust
# Basic types
let name = "Alice"          # String
let age = 30               # Integer
let height = 5.8           # Number
let active = true          # Boolean
let empty = null           # Null

# Collections
let items = [1, 2, 3]      # List
let config = {             # Map
  "host": "localhost",
  "port": 8080
}

# Time and size units
let timeout = 30s          # Duration
let max_size = 100MB       # Size
```

## Arquitectura

### Componentes

```
symbi repl
├── repl-cli/          # CLI interface and JSON-RPC server
├── repl-core/         # Core REPL engine and evaluator
├── repl-proto/        # JSON-RPC protocol definitions
└── repl-lsp/          # Language Server Protocol implementation
```

### Componentes Principales

- **DslEvaluator**: Ejecuta programas DSL con integracion al runtime
- **ReplEngine**: Coordina la evaluacion y el manejo de comandos
- **ExecutionMonitor**: Rastrea estadisticas y trazas de ejecucion
- **RuntimeBridge**: Se integra con el runtime de Symbiont para la aplicacion de politicas
- **SessionManager**: Gestiona instantaneas y estado de sesion

### Protocolo JSON-RPC

El REPL soporta JSON-RPC 2.0 para acceso programatico:

```json
// Evaluate DSL code
{
  "jsonrpc": "2.0",
  "method": "evaluate",
  "params": {"input": "let x = 42"},
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {"value": "42", "type": "integer"},
  "id": 1
}
```

## Seguridad y Aplicacion de Politicas

### Verificacion de Capacidades

El REPL aplica los requisitos de capacidades definidos en los bloques de seguridad de los agentes:

```rust
agent SecureAgent {
  name: "Secure Agent"
  security {
    capabilities: ["filesystem", "network"]
    sandbox: true
  }
}

behavior ReadFile {
  input { path: string }
  output { content: string }
  steps {
    # This will check if agent has "filesystem" capability
    require capability("filesystem")
    let content = read_file(path)
    return content
  }
}
```

### Integracion de Politicas

El REPL se integra con el motor de politicas de Symbiont para aplicar controles de acceso y requisitos de auditoria.

## Depuracion y Monitoreo

### Trazas de Ejecucion

```
:monitor traces 10

Recent Execution Traces:
  14:32:15.123 - AgentCreated [Agent: abc-123] (2ms)
  14:32:15.125 - AgentStarted [Agent: abc-123] (1ms)
  14:32:15.130 - BehaviorExecuted [Agent: abc-123] (5ms)
  14:32:15.135 - AgentPaused [Agent: abc-123]
```

### Estadisticas

```
:monitor stats

Execution Monitor Statistics:
  Total Executions: 42
  Successful: 38
  Failed: 4
  Success Rate: 90.5%
  Average Duration: 12.3ms
  Total Duration: 516ms
  Active Executions: 2
```

### Depuracion de Agentes

```
:agent debug abc-123

Agent Debug Information:
  ID: abc-123-def-456
  Name: Data Analyzer
  Version: 2.1.0
  State: Running
  Created: 2024-01-15 14:30:00 UTC
  Description: Analyzes datasets with privacy protection
  Author: data-team@company.com
  Available Functions/Behaviors: 5
  Required Capabilities: 2
    - data_read
    - analysis
  Resource Configuration:
    Memory: 512MB
    CPU: 2
    Storage: 1GB
```

## Integracion con IDEs

### Language Server Protocol

El REPL proporciona soporte LSP para integracion con IDEs:

```bash
# Start LSP server
symbi repl --lsp --port 9257
```

### Caracteristicas Soportadas

- Resaltado de sintaxis
- Autocompletado de codigo
- Diagnosticos de errores
- Informacion al pasar el cursor
- Ir a la definicion
- Busqueda de simbolos

## Mejores Practicas

### Flujo de Trabajo de Desarrollo

1. **Comenzar con Expresiones Simples**: Probar construcciones basicas del DSL
2. **Definir Agentes Incrementalmente**: Comenzar con definiciones minimas de agentes
3. **Probar Comportamientos por Separado**: Definir y probar comportamientos antes de la integracion
4. **Usar Monitoreo**: Aprovechar el monitoreo de ejecucion para depuracion
5. **Crear Instantaneas**: Guardar estados de sesion importantes

### Consejos de Rendimiento

- Usar `:monitor clear` periodicamente para reiniciar datos de monitoreo
- Limitar el historial de trazas con `:monitor traces <limit>`
- Destruir agentes no utilizados para liberar recursos
- Usar instantaneas para estados de sesion complejos

### Consideraciones de Seguridad

- Siempre definir capacidades apropiadas para los agentes
- Probar la aplicacion de politicas en desarrollo
- Usar modo sandbox para codigo no confiable
- Monitorear trazas de ejecucion para eventos de seguridad

## Solucion de Problemas

### Problemas Comunes

**Fallo en la Creacion del Agente**
```
Error: Missing capability: filesystem
```
*Solucion*: Agregar las capacidades requeridas al bloque de seguridad del agente

**Tiempo de Ejecucion Agotado**
```
Error: Maximum execution depth exceeded
```
*Solucion*: Verificar si hay recursion infinita en la logica de comportamiento

**Violacion de Politica**
```
Error: Policy violation: data access denied
```
*Solucion*: Verificar que el agente tiene los permisos apropiados

### Comandos de Depuracion

```rust
# Check agent state
:agent debug <agent-id>

# View execution traces
:monitor traces 50

# Check system statistics
:monitor stats

# Create debug snapshot
:snapshot
```

## Ejemplos

### Agente Simple

```rust
agent Calculator {
  name: "Basic Calculator"
  version: "1.0.0"
}

behavior Add {
  input { a: number, b: number }
  output { result: number }
  steps {
    return a + b
  }
}

# Test the behavior
let result = Add(5, 3)
print("5 + 3 =", result)
```

### Agente de Procesamiento de Datos

```rust
agent DataProcessor {
  name: "Data Processing Agent"
  version: "1.0.0"

  security {
    capabilities: ["data_read", "data_write"]
    sandbox: true
  }

  resources {
    memory: 256MB
    cpu: 1
  }
}

behavior ProcessCsv {
  input { file_path: string }
  output { summary: ProcessingSummary }

  steps {
    require capability("data_read")

    let data = read_csv(file_path)
    let processed = transform_data(data)

    require capability("data_write")
    write_results(processed)

    return {
      "rows_processed": len(data),
      "status": "completed"
    }
  }
}
```

## Vease Tambien

- [Guia DSL](dsl-guide.md) - Referencia completa del lenguaje DSL
- [Arquitectura del Runtime](runtime-architecture.md) - Vision general de la arquitectura del sistema
- [Modelo de Seguridad](security-model.md) - Detalles de la implementacion de seguridad
- [Referencia de API](api-reference.md) - Documentacion completa de la API

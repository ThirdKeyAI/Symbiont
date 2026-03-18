---
layout: default
title: Guia DSL
description: "Guia completa del Lenguaje Especifico de Dominio de Symbiont"
nav_exclude: true
---

# Guia DSL
{: .no_toc }

## Otros idiomas
{: .no_toc}

[English](dsl-guide.md) | [中文简体](dsl-guide.zh-cn.md) | **Español** | [Português](dsl-guide.pt.md) | [日本語](dsl-guide.ja.md) | [Deutsch](dsl-guide.de.md)

---

Domina el DSL de Symbi para construir agentes de IA seguros y conscientes de politicas.
{: .fs-6 .fw-300 }

## Tabla de contenidos
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Descripcion general

El DSL de Symbi es un lenguaje especifico de dominio disenado para crear agentes autonomos y conscientes de politicas. Combina construcciones de programacion tradicionales con caracteristicas de seguridad avanzadas, operaciones criptograficas y definiciones de politicas declarativas.

### Caracteristicas principales

- **Diseno con seguridad primero**: Capacidades integradas de aplicacion de politicas y auditoria
- **Politicas declarativas**: Expresar requisitos de seguridad como codigo
- **Operaciones criptograficas**: Soporte nativo para cifrado, firma y pruebas
- **Comunicacion entre agentes**: Patrones integrados de mensajeria y colaboracion
- **Seguridad de tipos**: Tipado fuerte con anotaciones de tipo conscientes de la seguridad

---

## Sintaxis del lenguaje

### Estructura basica

Todo programa Symbi consiste en metadatos opcionales y definiciones de agentes:

```rust
metadata {
    version: "1.0.0"
    author: "developer"
    description: "Example agent"
}

agent process_data(input: DataSet) -> Result {
    // Agent implementation
}
```

> **Caracteristica planificada** — La sintaxis de importacion esta planificada para una futura version.
>
> ```rust
> import data_processing as dp;
> import security_utils;
> ```

### Comentarios

```rust
// Single-line comment
# Hash-style comment (tambien soportado)

/*
 * Multi-line comment
 * Supports markdown formatting
 */
```

---

## Bloques de metadatos

Los metadatos proporcionan informacion esencial sobre tu agente:

```rust
metadata {
    version: "1.2.0"
    author: "ThirdKey Security Team"
    description: "Healthcare data analysis agent with HIPAA compliance"
    license: "Proprietary"
    tags: ["healthcare", "hipaa", "analysis"]
    min_runtime_version: "1.0.0"
    dependencies: ["medical_nlp", "privacy_tools"]
}
```

### Campos de metadatos

| Campo | Tipo | Requerido | Descripcion |
|-------|------|----------|-------------|
| `version` | String | Si | Version semantica del agente |
| `author` | String | Si | Autor o organizacion del agente |
| `description` | String | Si | Breve descripcion de la funcionalidad del agente |
| `license` | String | No | Identificador de licencia |
| `tags` | Array[String] | No | Etiquetas de clasificacion |
| `min_runtime_version` | String | No | Version minima requerida del runtime |
| `dependencies` | Array[String] | No | Dependencias externas |

---

## Definiciones de agentes

### Estructura basica de agente

```rust
agent agent_name(param1: Type1, param2: Type2) -> ReturnType {
    capabilities: [capability1, capability2]

    policy policy_name {
        // Policy rules
    }

    with configuration_options {
        // Agent implementation
    }
}
```

### Parametros de agente

Soporte para varios tipos de parametros:

```rust
agent complex_agent(
    // Basic types
    name: String,
    age: int,
    active: bool,

    // Optional parameters
    email: Optional<String>,

    // Complex types
    data: Array<Record>,
    config: Map<String, Value>,

    // Security-aware types
    sensitive_data: EncryptedData<PersonalInfo>,
    credentials: SecureString
) -> ProcessingResult {
    // Implementation
}
```

### Declaracion de capacidades

Declara lo que tu agente puede hacer:

```rust
agent data_processor(input: DataSet) -> Analysis {
    capabilities: [
        data_analysis,          // Core data processing
        statistical_modeling,   // Advanced analytics
        report_generation,      // Output formatting
        audit_logging           // Compliance tracking
    ]

    // Implementation
}
```

---

## Definiciones de politicas

Las politicas definen reglas de seguridad y cumplimiento que se aplican en tiempo de ejecucion.

### Estructura de politica

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    require: requirement_list
    audit: audit_specification
}
```

### Politicas de control de acceso

```rust
policy medical_data_access {
    allow: ["read", "analyze"] if user.role == "doctor"
    allow: ["read"] if user.role == "nurse"
    deny: ["export", "print"] if data.contains_pii == true
    require: [
        user.clearance >= "medical_professional",
        session.mfa_verified == true,
        audit_trail = true
    ]
}
```

### Politicas de clasificacion de datos

```rust
policy data_classification {
    allow: process(data) if data.anonymized == true
    deny: store(data) if data.classification == "restricted"
    audit: all_operations with digital_signature
}
```

### Logica de politica compleja

```rust
policy dynamic_access_control {
    allow: read(resource) if (
        user.department == resource.owner_department ||
        user.role == "administrator" ||
        (user.role == "auditor" && current_time.business_hours)
    )

    deny: write(resource) if (
        resource.locked == true ||
        user.last_training < 30d ||
        system.maintenance_mode == true
    )

    require: approval("supervisor") for operations on sensitive_data
}
```

---

## Sistema de tipos

### Tipos primitivos

```rust
// Basic types
let name: String = "Alice";
let count: int = 42;
let rate: float = 3.14;
let active: bool = true;
```

### Tipos de coleccion

```rust
// Arrays
let numbers: Array<int> = [1, 2, 3, 4, 5];
let names: Array<String> = ["Alice", "Bob", "Charlie"];

// Maps
let config: Map<String, String> = {
    "host": "localhost",
    "port": "8080",
    "ssl": "true"
};

// Sets
let unique_ids: Set<String> = {"id1", "id2", "id3"};
```

### Tipos conscientes de la seguridad

```rust
// Encrypted types
let secret: EncryptedString = encrypt("sensitive_data", key);
let secure_number: Encrypted<int> = encrypt(42, key);

// Private data with differential privacy
let private_data: PrivateData<float> = PrivateData::new(value, epsilon=1.0);

// Verifiable results with zero-knowledge proofs
let verified_result: VerifiableResult<Analysis> = VerifiableResult {
    value: analysis,
    proof: generate_proof(analysis),
    signature: sign(analysis)
};
```

### Tipos personalizados

```rust
// Type aliases
type UserId = String;
type EncryptedPersonalInfo = EncryptedData<PersonalInfo>;
```

> **Caracteristica planificada** — Las definiciones de `struct` y `enum` estan planificadas para una futura version. Actualmente, solo se soportan los alias de `type`.
>
> ```rust
> // Struct definitions (planned)
> struct PersonalInfo {
>     name: String,
>     email: EncryptedString,
>     phone: Optional<String>,
>     birth_date: Date
> }
>
> // Enum definitions (planned)
> enum SecurityLevel {
>     Public,
>     Internal,
>     Confidential,
>     Restricted
> }
> ```

---

## Contexto de ejecucion

Configura como se ejecuta tu agente con la clausula `with`:

### Gestion de memoria

```rust
agent persistent_agent(data: DataSet) -> Result {
    with memory = "persistent", storage = "encrypted" {
        // Agent state persists across sessions
        store_knowledge(data);
        return process_with_history(data);
    }
}

agent ephemeral_agent(query: String) -> Answer {
    with memory = "ephemeral", cleanup = "immediate" {
        // Agent state is discarded after execution
        return quick_answer(query);
    }
}
```

### Configuracion de privacidad

```rust
agent privacy_preserving_agent(sensitive_data: PersonalInfo) -> Statistics {
    with privacy = "differential", epsilon = 1.0 {
        // Add differential privacy noise
        let noisy_stats = compute_statistics(sensitive_data);
        return add_privacy_noise(noisy_stats, epsilon);
    }
}
```

### Configuracion de seguridad

```rust
agent high_security_agent(classified_data: ClassifiedInfo) -> Report {
    with
        security = "maximum",
        sandbox = "firecracker",
        encryption = "homomorphic",
        requires = "top_secret_clearance"
    {
        // High-security processing
        return process_classified(classified_data);
    }
}
```

---

## Funciones integradas

### Procesamiento de datos

```rust
// Validation functions
if (validate_input(data)) {
    // Process valid data
}

// Data transformation
let cleaned_data = sanitize(raw_data);
let normalized = normalize(cleaned_data);
```

### Operaciones criptograficas

```rust
// Encryption/Decryption
let encrypted = encrypt(plaintext, public_key);
let decrypted = decrypt(ciphertext, private_key);

// Digital signatures
let signature = sign(message, private_key);
let valid = verify(message, signature, public_key);

// Zero-knowledge proofs
let proof = prove(statement);
let verified = verify_proof(proof, public_statement);
```

### Auditoria y registro

```rust
// Audit logging
audit_log("operation_started", {
    "operation": "data_processing",
    "user": user.id,
    "timestamp": now()
});

// Security events
security_event("policy_violation", {
    "policy": "data_access",
    "user": user.id,
    "resource": resource.id
});
```

---

## Comunicacion entre agentes

### Mensajeria directa

```rust
agent coordinator(task: Task) -> Result {
    with communication = "secure" {
        // Send task to specialized agent
        let result = agent security_analyzer.analyze(task);

        if (result.safe) {
            let processed = agent data_processor.process(task);
            return processed;
        } else {
            return reject("Security check failed");
        }
    }
}
```

### Patron publicar-suscribir

```rust
agent event_publisher(event: Event) -> Confirmation {
    with communication = "broadcast" {
        // Broadcast event to all subscribers
        broadcast(EventNotification {
            type: event.type,
            data: event.data,
            timestamp: now()
        });

        return Confirmation { sent: true };
    }
}

agent event_subscriber() -> Void {
    with communication = "subscribe" {
        // Subscribe to specific events
        let events = subscribe(EventNotification);

        for event in events {
            process_event(event);
        }
    }
}
```

### Comunicacion segura

```rust
agent secure_collaborator(request: SecureRequest) -> SecureResponse {
    with
        communication = "encrypted",
        authentication = "mutual_tls"
    {
        // Establish secure channel
        let channel = establish_secure_channel(request.source);

        // Send encrypted response
        let response = process_request(request);
        return encrypt_response(response, channel.key);
    }
}
```

---

## Manejo de errores

### Bloques Try-Catch

```rust
agent robust_processor(data: DataSet) -> Result {
    try {
        let validated = validate_data(data);
        let processed = process_data(validated);
        return Ok(processed);
    } catch (ValidationError e) {
        audit_log("validation_failed", e.details);
        return Error("Invalid input data");
    } catch (ProcessingError e) {
        audit_log("processing_failed", e.details);
        return Error("Processing failed");
    }
}
```

### Recuperacion de errores

```rust
agent fault_tolerant_agent(input: Input) -> Result {
    let max_retries = 3;
    let retry_count = 0;

    while (retry_count < max_retries) {
        try {
            return process_with_fallback(input);
        } catch (TransientError e) {
            retry_count += 1;
            sleep(exponential_backoff(retry_count));
        } catch (PermanentError e) {
            return Error(e.message);
        }
    }

    return Error("Max retries exceeded");
}
```

---

## Caracteristicas avanzadas

### Compilacion condicional

> **Caracteristica planificada** — La compilacion condicional esta planificada para una futura version.

```rust
agent development_agent(data: DataSet) -> Result {
    capabilities: [development, testing]

    #if debug {
        debug_log("Processing data: " + data.summary);
    }

    #if feature.enhanced_security {
        policy strict_security {
            require: multi_factor_authentication
            audit: all_operations with timestamps
        }
    }

    // Implementation
}
```

### Macros y generacion de codigo

> **Caracteristica planificada** — Las definiciones de macros estan planificadas para una futura version.

```rust
// Define reusable policy template
macro secure_data_policy($classification: String) {
    policy secure_access {
        allow: read(data) if user.clearance >= $classification
        deny: export(data) if data.contains_pii
        audit: all_operations with signature
    }
}

agent classified_processor(data: ClassifiedData) -> Report {
    // Use the macro
    secure_data_policy!("secret");

    // Implementation
}
```

### Integracion con sistemas externos

```rust
agent api_integrator(request: APIRequest) -> APIResponse {
    capabilities: [api_access, data_transformation]

    policy api_access {
        allow: call(external_api) if api.rate_limit_ok
        require: valid_api_key
        audit: all_api_calls with response_codes
    }

    with
        timeout = 30.seconds,
        retry_policy = "exponential_backoff"
    {
        let response = call_external_api(request);
        return transform_response(response);
    }
}
```

---

## Mejores practicas

### Directrices de seguridad

1. **Siempre define politicas** para acceso a datos y operaciones
2. **Usa tipos cifrados** para datos sensibles
3. **Implementa registro de auditoria** para cumplimiento
4. **Valida todas las entradas** antes del procesamiento
5. **Usa el principio de menor privilegio** en definiciones de politicas

### Optimizacion de rendimiento

1. **Usa memoria efimera** para agentes de corta duracion
2. **Agrupa operaciones** cuando sea posible
3. **Implementa manejo adecuado de errores** con reintentos
4. **Monitorea el uso de recursos** en el contexto de ejecucion
5. **Usa tipos de datos apropiados** para tu caso de uso

### Organizacion del codigo

1. **Agrupa politicas relacionadas** en el mismo bloque
2. **Usa nombres descriptivos de capacidades**
3. **Documenta logica de politicas complejas** con comentarios
4. **Separa responsabilidades** en diferentes agentes
5. **Reutiliza patrones comunes** con definiciones de politicas compartidas

---

## Ejemplos

### Procesador de datos de salud

```rust
metadata {
    version: "2.1.0"
    author: "Medical AI Team"
    description: "HIPAA-compliant patient data analyzer"
    tags: ["healthcare", "hipaa", "privacy"]
}

agent medical_analyzer(patient_data: EncryptedPatientRecord) -> MedicalInsights {
    capabilities: [
        medical_analysis,
        privacy_preservation,
        audit_logging,
        report_generation
    ]

    policy hipaa_compliance {
        allow: analyze(data) if user.medical_license.valid
        deny: export(data) if data.contains_identifiers
        require: [
            user.hipaa_training.completed,
            session.secure_connection,
            audit_trail = true
        ]
    }

    with
        memory = "encrypted",
        privacy = "differential",
        security = "high",
        requires = "medical_clearance"
    {
        try {
            let decrypted = decrypt(patient_data, medical_key);
            let anonymized = anonymize_data(decrypted);
            let insights = analyze_medical_data(anonymized);

            audit_log("analysis_completed", {
                "patient_id_hash": hash(decrypted.id),
                "insights_generated": insights.count,
                "timestamp": now()
            });

            return insights;
        } catch (DecryptionError e) {
            security_event("decryption_failed", e.details);
            return Error("Unable to process patient data");
        }
    }
}
```

### Monitor de transacciones financieras

```rust
agent fraud_detector(transaction: Transaction) -> FraudAssessment {
    capabilities: [fraud_detection, risk_analysis, real_time_processing]

    policy financial_compliance {
        allow: analyze(transaction) if user.role == "fraud_analyst"
        deny: store(transaction.details) if transaction.amount > 10000
        require: [
            user.financial_license.valid,
            system.compliance_mode.active,
            real_time_monitoring = true
        ]
        audit: all_decisions with reasoning
    }

    with
        memory = "ephemeral",
        timeout = 500.milliseconds,
        priority = "high"
    {
        let risk_score = calculate_risk(transaction);
        let historical_pattern = analyze_pattern(transaction.account_id);

        if (risk_score > 0.8 || historical_pattern.suspicious) {
            alert_fraud_team(transaction, risk_score);
            return FraudAssessment {
                risk_level: "high",
                recommended_action: "block_transaction",
                confidence: risk_score
            };
        }

        return FraudAssessment {
            risk_level: "low",
            recommended_action: "approve",
            confidence: 1.0 - risk_score
        };
    }
}
```

---

## Proximos pasos

- **[Especificacion del DSL](dsl-specification.md)** - Referencia completa de la especificacion del lenguaje
- **[Arquitectura del Runtime](/runtime-architecture)** - Comprende como se ejecutan los agentes
- **[Modelo de Seguridad](/security-model)** - Aprende sobre la implementacion de seguridad
- **[Referencia de API](/api-reference)** - Referencia completa de funciones y tipos
- **[Ejemplos](https://github.com/thirdkeyai/symbiont/tree/main/examples)** - Mas ejemplos completos

Listo para construir tu primer agente? Consulta nuestra [guia de inicio](/getting-started) o explora los [ejemplos del runtime](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples).

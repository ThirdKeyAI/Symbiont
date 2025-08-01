---
layout: default
title: Guía DSL
description: "Guía completa del Lenguaje Específico de Dominio de Symbiont"
nav_exclude: true
---

# Guía DSL
{: .no_toc }

## 🌐 Otros idiomas
{: .no_toc}

[English](dsl-guide.md) | [中文简体](dsl-guide.zh-cn.md) | **Español** | [Português](dsl-guide.pt.md) | [日本語](dsl-guide.ja.md) | [Deutsch](dsl-guide.de.md)

---

Domina el DSL de Symbi para construir agentes de IA seguros y conscientes de políticas.
{: .fs-6 .fw-300 }

## Tabla de contenidos
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Descripción general

El DSL de Symbi es un lenguaje específico de dominio diseñado para crear agentes autónomos y conscientes de políticas. Combina construcciones de programación tradicionales con características de seguridad avanzadas, operaciones criptográficas y definiciones de políticas declarativas.

### Características principales

- **Diseño con seguridad primero**: Capacidades integradas de aplicación de políticas y auditoría
- **Políticas declarativas**: Expresar requisitos de seguridad como código
- **Operaciones criptográficas**: Soporte nativo para cifrado, firma y pruebas
- **Comunicación entre agentes**: Patrones integrados de mensajería y colaboración
- **Seguridad de tipos**: Tipado fuerte con anotaciones de tipo conscientes de la seguridad

---

## Sintaxis del lenguaje

### Estructura básica

Todo programa Symbi consiste en metadatos opcionales, importaciones y definiciones de agentes:

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Example agent"
}

import data_processing as dp;
import security_utils;

agent process_data(input: DataSet) -> Result {
    // Agent implementation
}
```

### Comentarios

```rust
// Single-line comment

/*
 * Multi-line comment
 * Supports markdown formatting
 */
```

---

## Bloques de metadatos

Los metadatos proporcionan información esencial sobre tu agente:

```rust
metadata {
    version = "1.2.0"
    author = "ThirdKey Security Team"
    description = "Healthcare data analysis agent with HIPAA compliance"
    license = "Proprietary"
    tags = ["healthcare", "hipaa", "analysis"]
    min_runtime_version = "0.5.0"
    dependencies = ["medical_nlp", "privacy_tools"]
}
```

### Campos de metadatos

| Campo | Tipo | Requerido | Descripción |
|-------|------|----------|-------------|
| `version` | String | Sí | Versión semántica del agente |
| `author` | String | Sí | Autor o organización del agente |
| `description` | String | Sí | Breve descripción de la funcionalidad del agente |
| `license` | String | No | Identificador de licencia |
| `tags` | Array[String] | No | Etiquetas de clasificación |
| `min_runtime_version` | String | No | Versión mínima requerida del runtime |
| `dependencies` | Array[String] | No | Dependencias externas |

---

## Definiciones de agentes

### Estructura básica de agente

```rust
agent agent_name(param1: Type1, param2: Type2) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    
    policy policy_name {
        // Policy rules
    }
    
    with configuration_options {
        // Agent implementation
    }
}
```

### Parámetros de agente

Soporte para varios tipos de parámetros:

```rust
agent complex_agent(
    // Basic types
    name: String,
    age: Integer,
    active: Boolean,
    
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

### Declaración de capacidades

Declara lo que tu agente puede hacer:

```rust
agent data_processor(input: DataSet) -> Analysis {
    capabilities = [
        "data_analysis",        // Core data processing
        "statistical_modeling", // Advanced analytics
        "report_generation",    // Output formatting
        "audit_logging"         // Compliance tracking
    ]
    
    // Implementation
}
```

---

## Definiciones de políticas

Las políticas definen reglas de seguridad y cumplimiento que se aplican en tiempo de ejecución.

### Estructura de política

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    require: requirement_list
    audit: audit_specification
    conditions: {
        field: value,
        another_field: condition
    }
}
```

### Políticas de control de acceso

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

### Políticas de clasificación de datos

```rust
policy data_classification {
    conditions: {
        classification: "confidential",
        retention_period: 7.years,
        geographic_restriction: "EU",
        encryption_required: true
    }
    
    allow: process(data) if data.anonymized == true
    deny: store(data) if data.classification == "restricted"
    audit: all_operations with digital_signature
}
```

### Lógica de política compleja

```rust
policy dynamic_access_control {
    allow: read(resource) if (
        user.department == resource.owner_department ||
        user.role == "administrator" ||
        (user.role == "auditor" && current_time.business_hours)
    )
    
    deny: write(resource) if (
        resource.locked == true ||
        user.last_training < 30.days_ago ||
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
let count: Integer = 42;
let rate: Float = 3.14;
let active: Boolean = true;
let data: Bytes = b"binary_data";
```

### Tipos de colección

```rust
// Arrays
let numbers: Array<Integer> = [1, 2, 3, 4, 5];
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
let secure_number: EncryptedInteger = encrypt(42, key);

// Private data with differential privacy
let private_data: PrivateData<Float> = PrivateData::new(value, epsilon=1.0);

// Verifiable results with zero-knowledge proofs
let verified_result: VerifiableResult<Analysis> = VerifiableResult {
    value: analysis,
    proof: generate_proof(analysis),
    signature: sign(analysis)
};
```

### Tipos personalizados

```rust
// Struct definitions
struct PersonalInfo {
    name: String,
    email: EncryptedString,
    phone: Optional<String>,
    birth_date: Date
}

// Enum definitions
enum SecurityLevel {
    Public,
    Internal,
    Confidential,
    Restricted
}

// Type aliases
type UserId = String;
type EncryptedPersonalInfo = EncryptedData<PersonalInfo>;
```

---

## Contexto de ejecución

Configura cómo se ejecuta tu agente con la cláusula `with`:

### Gestión de memoria

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

### Configuración de privacidad

```rust
agent privacy_preserving_agent(sensitive_data: PersonalInfo) -> Statistics {
    with privacy = "differential", epsilon = 1.0 {
        // Add differential privacy noise
        let noisy_stats = compute_statistics(sensitive_data);
        return add_privacy_noise(noisy_stats, epsilon);
    }
}
```

### Configuración de seguridad

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

### Operaciones criptográficas

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

### Auditoría y registro

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

## Comunicación entre agentes

### Mensajería directa

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

### Patrón publicar-suscribir

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

### Comunicación segura

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

### Recuperación de errores

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

## Características avanzadas

### Compilación condicional

```rust
agent development_agent(data: DataSet) -> Result {
    capabilities = ["development", "testing"]
    
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

### Macros y generación de código

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

### Integración con sistemas externos

```rust
agent api_integrator(request: APIRequest) -> APIResponse {
    capabilities = ["api_access", "data_transformation"]
    
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

## Mejores prácticas

### Directrices de seguridad

1. **Siempre define políticas** para acceso a datos y operaciones
2. **Usa tipos cifrados** para datos sensibles
3. **Implementa registro de auditoría** para cumplimiento
4. **Valida todas las entradas** antes del procesamiento
5. **Usa el principio de menor privilegio** en definiciones de políticas

### Optimización de rendimiento

1. **Usa memoria efímera** para agentes de corta duración
2. **Agrupa operaciones** cuando sea posible
3. **Implementa manejo adecuado de errores** con reintentos
4. **Monitorea el uso de recursos** en el contexto de ejecución
5. **Usa tipos de datos apropiados** para tu caso de uso

### Organización del código

1. **Agrupa políticas relacionadas** en el mismo bloque
2. **Usa nombres descriptivos de capacidades**
3. **Documenta lógica de políticas complejas** con comentarios
4. **Separa responsabilidades** en diferentes agentes
5. **Reutiliza patrones comunes** con macros

---

## Ejemplos

### Procesador de datos de salud

```rust
metadata {
    version = "2.1.0"
    author = "Medical AI Team"
    description = "HIPAA-compliant patient data analyzer"
    tags = ["healthcare", "hipaa", "privacy"]
}

agent medical_analyzer(patient_data: EncryptedPatientRecord) -> MedicalInsights {
    capabilities = [
        "medical_analysis",
        "privacy_preservation", 
        "audit_logging",
        "report_generation"
    ]
    
    policy hipaa_compliance {
        allow: analyze(data) if user.medical_license.valid
        deny: export(data) if data.contains_identifiers
        require: [
            user.hipaa_training.completed,
            session.secure_connection,
            audit_trail = true
        ]
        conditions: {
            data_classification: "medical",
            retention_period: 7.years,
            access_logging: "detailed"
        }
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
    capabilities = ["fraud_detection", "risk_analysis", "real_time_processing"]
    
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

## Próximos pasos

- **[Arquitectura del runtime](/runtime-architecture.es)** - Comprende cómo se ejecutan los agentes
- **[Modelo de seguridad](/security-model.es)** - Aprende sobre la implementación de seguridad
- **[Referencia de API](/api-reference.es)** - Referencia completa de funciones y tipos
- **[Ejemplos](https://github.com/thirdkeyai/symbiont/tree/main/examples)** - Más ejemplos completos

¿Listo para construir tu primer agente? Consulta nuestra [guía de inicio](/getting-started.es) o explora los [ejemplos del runtime](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples).
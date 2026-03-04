---
layout: default
title: Guia DSL
description: "Guia completo da Linguagem Específica de Domínio do Symbiont"
nav_exclude: true
---

# Guia DSL
{: .no_toc }

## Outros idiomas
{: .no_toc}

[English](dsl-guide.md) | [中文简体](dsl-guide.zh-cn.md) | [Español](dsl-guide.es.md) | **Português** | [日本語](dsl-guide.ja.md) | [Deutsch](dsl-guide.de.md)

---

Domine a DSL do Symbi para construir agentes de IA seguros e conscientes de políticas.
{: .fs-6 .fw-300 }

## Índice
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Visão Geral

A DSL do Symbi é uma linguagem específica de domínio projetada para criar agentes autônomos e conscientes de políticas. Ela combina construções de programação tradicionais com recursos de segurança avançados, operações criptográficas e definições de políticas declarativas.

### Principais Características

- **Design com segurança em primeiro lugar**: Capacidades integradas de aplicação de políticas e auditoria
- **Políticas declarativas**: Expressar requisitos de segurança como código
- **Operações criptográficas**: Suporte nativo para criptografia, assinatura e provas
- **Comunicação entre agentes**: Padrões integrados de mensagens e colaboração
- **Segurança de tipos**: Tipagem forte com anotações de tipo conscientes de segurança

---

## Sintaxe da Linguagem

### Estrutura Básica

Todo programa Symbi consiste em metadados opcionais e definições de agentes:

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

> **Recurso planejado** — A sintaxe de importação está planejada para uma versão futura.
>
> ```rust
> import data_processing as dp;
> import security_utils;
> ```

### Comentários

```rust
// Single-line comment

/*
 * Multi-line comment
 * Supports markdown formatting
 */
```

---

## Blocos de Metadados

Os metadados fornecem informações essenciais sobre seu agente:

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

### Campos de Metadados

| Campo | Tipo | Obrigatório | Descrição |
|-------|------|-------------|-----------|
| `version` | String | Sim | Versão semântica do agente |
| `author` | String | Sim | Autor ou organização do agente |
| `description` | String | Sim | Breve descrição da funcionalidade do agente |
| `license` | String | Não | Identificador da licença |
| `tags` | Array[String] | Não | Tags de classificação |
| `min_runtime_version` | String | Não | Versão mínima necessária do runtime |
| `dependencies` | Array[String] | Não | Dependências externas |

---

## Definições de Agentes

### Estrutura Básica de Agente

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

### Parâmetros de Agente

Suporte para vários tipos de parâmetros:

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

### Declaração de Capacidades

Declare o que seu agente pode fazer:

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

## Definições de Políticas

As políticas definem regras de segurança e conformidade que são aplicadas em tempo de execução.

### Estrutura de Política

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    require: requirement_list
    audit: audit_specification
}
```

### Políticas de Controle de Acesso

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

### Políticas de Classificação de Dados

```rust
policy data_classification {
    allow: process(data) if data.anonymized == true
    deny: store(data) if data.classification == "restricted"
    audit: all_operations with digital_signature
}
```

### Lógica de Política Complexa

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

## Sistema de Tipos

### Tipos Primitivos

```rust
// Basic types
let name: String = "Alice";
let count: int = 42;
let rate: float = 3.14;
let active: bool = true;
```

### Tipos de Coleção

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

### Tipos Conscientes de Segurança

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

### Tipos Personalizados

```rust
// Type aliases
type UserId = String;
type EncryptedPersonalInfo = EncryptedData<PersonalInfo>;
```

> **Recurso planejado** — Definições de `struct` e `enum` estão planejadas para uma versão futura. Atualmente, apenas aliases de `type` são suportados.
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

## Contexto de Execução

Configure como seu agente executa com a cláusula `with`:

### Gerenciamento de Memória

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

### Configurações de Privacidade

```rust
agent privacy_preserving_agent(sensitive_data: PersonalInfo) -> Statistics {
    with privacy = "differential", epsilon = 1.0 {
        // Add differential privacy noise
        let noisy_stats = compute_statistics(sensitive_data);
        return add_privacy_noise(noisy_stats, epsilon);
    }
}
```

### Configuração de Segurança

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

## Funções Integradas

### Processamento de Dados

```rust
// Validation functions
if (validate_input(data)) {
    // Process valid data
}

// Data transformation
let cleaned_data = sanitize(raw_data);
let normalized = normalize(cleaned_data);
```

### Operações Criptográficas

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

### Auditoria e Registro

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

## Comunicação entre Agentes

### Mensagens Diretas

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

### Padrão Publicar-Subscrever

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

### Comunicação Segura

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

## Tratamento de Erros

### Blocos Try-Catch

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

### Recuperação de Erros

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

## Recursos Avançados

### Compilação Condicional

> **Recurso planejado** — Compilação condicional está planejada para uma versão futura.

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

### Macros e Geração de Código

> **Recurso planejado** — Definições de macros estão planejadas para uma versão futura.

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

### Integração com Sistemas Externos

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

## Melhores Práticas

### Diretrizes de Segurança

1. **Sempre defina políticas** para acesso a dados e operações
2. **Use tipos criptografados** para dados sensíveis
3. **Implemente registro de auditoria** para conformidade
4. **Valide todas as entradas** antes do processamento
5. **Use o princípio do menor privilégio** nas definições de políticas

### Otimização de Performance

1. **Use memória efêmera** para agentes de curta duração
2. **Agrupe operações** quando possível
3. **Implemente tratamento adequado de erros** com tentativas de repetição
4. **Monitore o uso de recursos** no contexto de execução
5. **Use tipos de dados apropriados** para seu caso de uso

### Organização do Código

1. **Agrupe políticas relacionadas** no mesmo bloco
2. **Use nomes descritivos de capacidades**
3. **Documente lógica de políticas complexas** com comentários
4. **Separe responsabilidades** em diferentes agentes
5. **Reutilize padrões comuns** com definições de políticas compartilhadas

---

## Exemplos

### Processador de Dados de Saúde

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

### Monitor de Transações Financeiras

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

## Próximos Passos

- **[Especificação do DSL](dsl-specification.md)** - Referência completa da especificação da linguagem
- **[Arquitetura do Runtime](/runtime-architecture)** - Entenda como os agentes executam
- **[Modelo de Segurança](/security-model)** - Aprenda sobre implementação de segurança
- **[Referência da API](/api-reference)** - Referência completa de funções e tipos
- **[Exemplos](https://github.com/thirdkeyai/symbiont/tree/main/examples)** - Mais exemplos completos

Pronto para construir seu primeiro agente? Confira nosso [guia de início](/getting-started) ou explore os [exemplos do runtime](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples).

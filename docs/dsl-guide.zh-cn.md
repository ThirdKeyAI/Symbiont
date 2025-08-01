---
layout: default
title: DSL 指南
nav_order: 3
description: "Symbiont 领域特定语言完整指南"
---

# DSL 指南
{: .no_toc }

## 🌐 其他语言

[English](dsl-guide.md) | **中文简体** | [Español](dsl-guide.es.md) | [Português](dsl-guide.pt.md) | [日本語](dsl-guide.ja.md) | [Deutsch](dsl-guide.de.md)

---

掌握 Symbi DSL，构建策略感知的安全 AI 代理。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 概述

Symbi DSL 是一种专为创建自主、策略感知代理而设计的领域特定语言。它将传统编程结构与高级安全功能、加密操作和声明式策略定义相结合。

### 主要特性

- **安全优先设计**：内置策略执行和审计功能
- **声明式策略**：以代码形式表达安全要求
- **加密操作**：原生支持加密、签名和证明
- **代理间通信**：内置消息传递和协作模式
- **类型安全**：具有安全感知类型注释的强类型系统

---

## 语言语法

### 基本结构

每个 Symbi 程序都由可选的元数据、导入和代理定义组成：

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

### 注释

```rust
// Single-line comment

/*
 * Multi-line comment
 * Supports markdown formatting
 */
```

---

## 元数据块

元数据提供关于您的代理的基本信息：

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

### 元数据字段

| 字段 | 类型 | 必需 | 描述 |
|-------|------|----------|-------------|
| `version` | String | 是 | 代理的语义版本 |
| `author` | String | 是 | 代理作者或组织 |
| `description` | String | 是 | 代理功能的简要描述 |
| `license` | String | 否 | 许可证标识符 |
| `tags` | Array[String] | 否 | 分类标签 |
| `min_runtime_version` | String | 否 | 所需的最低运行时版本 |
| `dependencies` | Array[String] | 否 | 外部依赖项 |

---

## 代理定义

### 基本代理结构

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

### 代理参数

支持各种参数类型：

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

### 能力声明

声明您的代理能够做什么：

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

## 策略定义

策略定义在运行时强制执行的安全和合规规则。

### 策略结构

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

### 访问控制策略

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

### 数据分类策略

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

### 复杂策略逻辑

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

## 类型系统

### 基本类型

```rust
// Basic types
let name: String = "Alice";
let count: Integer = 42;
let rate: Float = 3.14;
let active: Boolean = true;
let data: Bytes = b"binary_data";
```

### 集合类型

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

### 安全感知类型

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

### 自定义类型

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

## 执行上下文

使用 `with` 子句配置代理的执行方式：

### 内存管理

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

### 隐私设置

```rust
agent privacy_preserving_agent(sensitive_data: PersonalInfo) -> Statistics {
    with privacy = "differential", epsilon = 1.0 {
        // Add differential privacy noise
        let noisy_stats = compute_statistics(sensitive_data);
        return add_privacy_noise(noisy_stats, epsilon);
    }
}
```

### 安全配置

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

## 内置函数

### 数据处理

```rust
// Validation functions
if (validate_input(data)) {
    // Process valid data
}

// Data transformation
let cleaned_data = sanitize(raw_data);
let normalized = normalize(cleaned_data);
```

### 加密操作

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

### 审计和日志记录

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

## 代理间通信

### 直接消息传递

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

### 发布-订阅模式

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

### 安全通信

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

## 错误处理

### Try-Catch 块

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

### 错误恢复

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

## 高级功能

### 条件编译

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

### 宏和代码生成

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

### 外部系统集成

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

## 最佳实践

### 安全指南

1. **始终为数据访问和操作定义策略**
2. **对敏感数据使用加密类型**
3. **为合规性实施审计日志记录**
4. **在处理之前验证所有输入**
5. **在策略定义中使用最小权限原则**

### 性能优化

1. **对短期代理使用临时内存**
2. **尽可能批量操作**
3. **实施适当的错误处理和重试机制**
4. **在执行上下文中监控资源使用情况**
5. **为您的用例使用适当的数据类型**

### 代码组织

1. **将相关策略分组在同一块中**
2. **使用描述性的能力名称**
3. **用注释记录复杂的策略逻辑**
4. **将关注点分离到不同的代理中**
5. **使用宏重用常见模式**

---

## 示例

### 医疗数据处理器

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

### 金融交易监控器

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

## 下一步

- **[运行时架构](/runtime-architecture.zh-cn)** - 了解代理如何执行
- **[安全模型](/security-model.zh-cn)** - 学习安全实现
- **[API 参考](/api-reference.zh-cn)** - 完整的函数和类型参考
- **[示例](https://github.com/thirdkeyai/symbiont/tree/main/examples)** - 更多完整示例

准备构建您的第一个代理？查看我们的[入门指南](/getting-started.zh-cn)或探索[运行时示例](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)。
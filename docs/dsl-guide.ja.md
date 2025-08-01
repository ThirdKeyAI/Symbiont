---
layout: default
title: DSLガイド
description: "Symbiont ドメイン固有言語の完全ガイド"
---

# DSLガイド
{: .no_toc }

## 🌐 他の言語
{: .no_toc}

[English](dsl-guide.md) | [中文简体](dsl-guide.zh-cn.md) | [Español](dsl-guide.es.md) | [Português](dsl-guide.pt.md) | **日本語** | [Deutsch](dsl-guide.de.md)

---

ポリシー対応のセキュアなAIエージェントを構築するためのSymbi DSLをマスターしましょう。
{: .fs-6 .fw-300 }

## 目次
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 概要

Symbi DSLは、自律的でポリシー対応のエージェントを作成するために設計されたドメイン固有言語です。従来のプログラミング構造と高度なセキュリティ機能、暗号化操作、宣言的ポリシー定義を組み合わせています。

### 主な機能

- **セキュリティファーストデザイン**: 組み込みのポリシー実行と監査機能
- **宣言的ポリシー**: セキュリティ要件をコードとして表現
- **暗号化操作**: 暗号化、署名、証明のネイティブサポート
- **エージェント間通信**: 組み込みのメッセージングと協働パターン
- **型安全性**: セキュリティ対応型注釈を持つ強い型付け

---

## 言語構文

### 基本構造

すべてのSymbiプログラムは、オプションのメタデータ、インポート、エージェント定義で構成されます：

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

### コメント

```rust
// Single-line comment

/*
 * Multi-line comment
 * Supports markdown formatting
 */
```

---

## メタデータブロック

メタデータは、あなたのエージェントに関する重要な情報を提供します：

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

### メタデータフィールド

| フィールド | 型 | 必須 | 説明 |
|-------|------|----------|-------------|
| `version` | String | はい | エージェントのセマンティックバージョン |
| `author` | String | はい | エージェントの作者または組織 |
| `description` | String | はい | エージェント機能の簡潔な説明 |
| `license` | String | いいえ | ライセンス識別子 |
| `tags` | Array[String] | いいえ | 分類タグ |
| `min_runtime_version` | String | いいえ | 必要な最小ランタイムバージョン |
| `dependencies` | Array[String] | いいえ | 外部依存関係 |

---

## エージェント定義

### 基本エージェント構造

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

### エージェントパラメータ

さまざまなパラメータ型をサポート：

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

### 機能宣言

あなたのエージェントができることを宣言します：

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

## ポリシー定義

ポリシーは、実行時に強制されるセキュリティとコンプライアンスルールを定義します。

### ポリシー構造

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

### アクセス制御ポリシー

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

### データ分類ポリシー

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

### 複雑なポリシーロジック

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

## 型システム

### プリミティブ型

```rust
// Basic types
let name: String = "Alice";
let count: Integer = 42;
let rate: Float = 3.14;
let active: Boolean = true;
let data: Bytes = b"binary_data";
```

### コレクション型

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

### セキュリティ対応型

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

### カスタム型

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

## 実行コンテキスト

`with`句でエージェントの実行方法を設定します：

### メモリ管理

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

### プライバシー設定

```rust
agent privacy_preserving_agent(sensitive_data: PersonalInfo) -> Statistics {
    with privacy = "differential", epsilon = 1.0 {
        // Add differential privacy noise
        let noisy_stats = compute_statistics(sensitive_data);
        return add_privacy_noise(noisy_stats, epsilon);
    }
}
```

### セキュリティ設定

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

## 組み込み関数

### データ処理

```rust
// Validation functions
if (validate_input(data)) {
    // Process valid data
}

// Data transformation
let cleaned_data = sanitize(raw_data);
let normalized = normalize(cleaned_data);
```

### 暗号化操作

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

### 監査とログ記録

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

## エージェント間通信

### 直接メッセージング

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

### パブリッシュ・サブスクライブパターン

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

### セキュア通信

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

## エラーハンドリング

### Try-Catchブロック

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

### エラー回復

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

## 高度な機能

### 条件付きコンパイル

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

### マクロとコード生成

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

### 外部システム統合

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

## ベストプラクティス

### セキュリティガイドライン

1. **データアクセスと操作に対して常にポリシーを定義する**
2. **機密データには暗号化型を使用する**
3. **コンプライアンスのために監査ログを実装する**
4. **処理前にすべての入力を検証する**
5. **ポリシー定義で最小権限の原則を使用する**

### パフォーマンス最適化

1. **短期間のエージェントには一時的メモリを使用する**
2. **可能な限り操作をバッチ処理する**
3. **リトライを含む適切なエラーハンドリングを実装する**
4. **実行コンテキストでリソース使用量を監視する**
5. **使用ケースに適したデータ型を使用する**

### コード組織

1. **関連するポリシーを同じブロックにグループ化する**
2. **説明的な機能名を使用する**
3. **複雑なポリシーロジックをコメントで文書化する**
4. **関心事を異なるエージェントに分離する**
5. **共通パターンをマクロで再利用する**

---

## 例

### 医療データプロセッサ

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

### 金融取引監視

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

## 次のステップ

- **[ランタイムアーキテクチャ](/runtime-architecture.ja)** - エージェントの実行方法を理解する
- **[セキュリティモデル](/security-model.ja)** - セキュリティ実装について学ぶ
- **[APIリファレンス](/api-reference.ja)** - 完全な関数と型のリファレンス
- **[例](https://github.com/thirdkeyai/symbiont/tree/main/examples)** - より多くの完全な例

最初のエージェントを構築する準備はできましたか？[スタートガイド](/getting-started.ja)をチェックするか、[ランタイムの例](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)を探索してください。
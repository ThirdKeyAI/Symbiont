# HTTP 入力モジュール

HTTP 入力モジュールは、外部システムが HTTP リクエストを通じて Symbiont エージェントを呼び出すことを可能にする webhook サーバーを提供します。このモジュールは、HTTP エンドポイントを通じてエージェントを公開することで、外部サービス、webhook、API との統合を可能にします。

## 概要

HTTP 入力モジュールは以下で構成されています：

- **HTTP サーバー**: 受信 HTTP リクエストをリッスンする Axum ベースの Web サーバー
- **認証**: Bearer トークンと JWT ベースの認証をサポート
- **リクエストルーティング**: 特定のエージェントにリクエストを向ける柔軟なルーティングルール
- **レスポンス制御**: 設定可能なレスポンスフォーマットとステータスコード
- **セキュリティ機能**: CORS サポート、リクエストサイズ制限、監査ログ
- **並行性管理**: 組み込みリクエストレート制限と並行性制御

このモジュールは `http-input` 機能フラグで条件付きコンパイルされ、Symbiont エージェントランタイムとシームレスに統合されます。

## 設定

HTTP 入力モジュールは [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs) 構造体を使用して設定されます：

### 基本設定

```rust
use symbiont_runtime::http_input::HttpInputConfig;
use symbiont_runtime::types::AgentId;

let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    // ... other fields
    ..Default::default()
};
```

### 設定フィールド

| フィールド | 型 | デフォルト | 説明 |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"127.0.0.1"` | HTTP サーバーをバインドする IP アドレス |
| `port` | `u16` | `8081` | リッスンするポート番号 |
| `path` | `String` | `"/webhook"` | HTTP パスエンドポイント |
| `agent` | `AgentId` | 新規 ID | リクエストに対して呼び出すデフォルトエージェント |
| `auth_header` | `Option<String>` | `None` | 認証用の Bearer トークン |
| `jwt_public_key_path` | `Option<String>` | `None` | JWT 公開鍵ファイルのパス |
| `max_body_bytes` | `usize` | `65536` | 最大リクエストボディサイズ（64 KB） |
| `concurrency` | `usize` | `10` | 最大同時リクエスト数 |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | リクエストルーティングルール |
| `response_control` | `Option<ResponseControlConfig>` | `None` | レスポンスフォーマット設定 |
| `forward_headers` | `Vec<String>` | `[]` | エージェントに転送するヘッダー |
| `cors_origins` | `Vec<String>` | `[]` | 許可された CORS オリジン（空 = CORS 無効） |
| `audit_enabled` | `bool` | `true` | リクエスト監査ログを有効にする |

### エージェントルーティングルール

リクエストの特性に基づいて異なるエージェントにリクエストをルーティング：

```rust
use symbiont_runtime::http_input::{AgentRoutingRule, RouteMatch};

let routing_rules = vec![
    AgentRoutingRule {
        condition: RouteMatch::PathPrefix("/api/github".to_string()),
        agent: AgentId::from_str("github_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::HeaderEquals("X-Source".to_string(), "slack".to_string()),
        agent: AgentId::from_str("slack_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::JsonFieldEquals("source".to_string(), "twilio".to_string()),
        agent: AgentId::from_str("sms_handler")?,
    },
];
```

### レスポンス制御

[`ResponseControlConfig`](../crates/runtime/src/http_input/config.rs) を使用して HTTP レスポンスをカスタマイズ：

```rust
use symbiont_runtime::http_input::ResponseControlConfig;

let response_control = ResponseControlConfig {
    default_status: 200,
    agent_output_to_json: true,
    error_status: 500,
    echo_input_on_error: false,
};
```

## セキュリティ機能

### 認証

HTTP 入力モジュールは複数の認証方法をサポートします：

#### Bearer トークン認証

静的 Bearer トークンを設定：

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### シークレットストア統合

セキュリティ強化のためのシークレット参照を使用：

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### JWT 認証（EdDSA）

Ed25519公開鍵によるJWTベース認証を設定：

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/ed25519-public.pem".to_string()),
    ..Default::default()
};
```

JWTベリファイアは指定されたPEMファイルからEd25519公開鍵をロードし、受信する `Authorization: Bearer <jwt>` トークンを検証します。**EdDSA** アルゴリズムのみが受け入れられ、HS256、RS256、およびその他のアルゴリズムは拒否されます。

#### ヘルスエンドポイント

HTTP 入力モジュールは独自の `/health` エンドポイントを公開しません。ヘルスチェックは、完全なランタイム（APIサーバーを含む）を起動する `symbi up` を実行する際に、メインHTTP APIの `/api/v1/health` を通じて利用可能です：

```bash
# メインAPIサーバー経由のヘルスチェック（デフォルトポート8080）
curl http://127.0.0.1:8080/api/v1/health
# => {"status": "ok"}
```

HTTP 入力サーバー専用のヘルスプローブが必要な場合は、代わりにロードバランサーをメインAPIヘルスエンドポイントにルーティングしてください。

### セキュリティ制御

- **ループバックのみがデフォルト**: `bind_address` はデフォルトで `127.0.0.1` — 明示的に設定しない限り、サーバーはローカル接続のみを受け入れます
- **デフォルトでCORS無効**: `cors_origins` はデフォルトで空リスト。つまりCORSは無効です。クロスオリジンアクセスを有効にするには特定のオリジンを追加してください
- **リクエストサイズ制限**: 設定可能な最大ボディサイズでリソース枯渇を防止
- **並行性制限**: 組み込みセマフォが同時リクエスト処理を制御
- **監査ログ**: 有効時にすべての受信リクエストの構造化ログ
- **シークレット解決**: Vault とファイルベースシークレットストアとの統合

## 使用例

### HTTP 入力サーバーの開始

```rust
use symbiont_runtime::http_input::{HttpInputConfig, start_http_input};
use symbiont_runtime::secrets::SecretsConfig;
use std::sync::Arc;

// HTTP 入力サーバーを設定
let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    auth_header: Some("Bearer secret-token".to_string()),
    audit_enabled: true,
    cors_origins: vec!["https://example.com".to_string()],
    ..Default::default()
};

// オプション: シークレットを設定
let secrets_config = SecretsConfig::default();

// サーバーを開始
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### エージェント定義例

[`webhook_handler.dsl`](../agents/webhook_handler.dsl) で webhook ハンドラーエージェントを作成：

```dsl
agent webhook_handler(body: JSON) -> Maybe<Alert> {
    capabilities = ["http_input", "event_processing", "alerting"]
    memory = "ephemeral"
    privacy = "strict"

    policy webhook_guard {
        allow: use("llm") if body.source == "slack" || body.user.ends_with("@company.com")
        allow: publish("topic://alerts") if body.type == "security_alert"
        audit: all_operations
    }

    with context = {} {
        if body.type == "security_alert" {
            alert = {
                "summary": body.message,
                "source": body.source,
                "level": body.severity,
                "user": body.user
            }
            publish("topic://alerts", alert)
            return alert
        }

        return None
    }
}
```

### HTTP リクエスト例

エージェントをトリガーするために webhook リクエストを送信：

```bash
curl -X POST http://localhost:8081/webhook \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer secret-token" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

### 予期されるレスポンス

サーバーはエージェントの出力を含む JSON レスポンスを返します：

```json
{
  "status": "execution_started",
  "agent_id": "webhook_handler",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## 統合パターン

### Webhook エンドポイント

異なる webhook ソースに対して異なるエージェントを設定：

```rust
let routing_rules = vec![
    AgentRoutingRule {
        condition: RouteMatch::HeaderEquals("X-GitHub-Event".to_string(), "push".to_string()),
        agent: AgentId::from_str("github_push_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::JsonFieldEquals("source".to_string(), "stripe".to_string()),
        agent: AgentId::from_str("payment_processor")?,
    },
];
```

### API ゲートウェイ統合

API ゲートウェイの背後でバックエンドサービスとして使用：

```rust
let config = HttpInputConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8081,
    path: "/api/webhook".to_string(),
    cors_origins: vec!["https://example.com".to_string()],
    forward_headers: vec![
        "X-Forwarded-For".to_string(),
        "X-Request-ID".to_string(),
    ],
    ..Default::default()
};
```

### ヘルスチェック統合

HTTP 入力モジュールは専用のヘルスエンドポイントを含みません。ロードバランサーと監視の統合にはメインAPIヘルスエンドポイント（`/api/v1/health`）を使用してください。詳細については上記の[ヘルスエンドポイント](#ヘルスエンドポイント)セクションを参照してください。

## エラーハンドリング

HTTP 入力モジュールは包括的なエラーハンドリングを提供します：

- **認証エラー**: 無効なトークンに対して `401 Unauthorized` を返す
- **レート制限**: 並行性制限を超えた場合に `429 Too Many Requests` を返す
- **ペイロードエラー**: 不正な JSON に対して `400 Bad Request` を返す
- **エージェントエラー**: エラー詳細と共に設定可能なエラーステータスを返す
- **サーバーエラー**: ランタイム障害に対して `500 Internal Server Error` を返す

## 監視と可観測性

### 監査ログ

`audit_enabled` が true の場合、モジュールはすべてのリクエストに関する構造化情報をログに記録します：

```log
INFO HTTP Input: Received request with 5 headers
INFO Would invoke agent webhook_handler with input data
```

### メトリクス統合

このモジュールは Symbiont ランタイムのメトリクスシステムと統合して以下を提供します：

- リクエスト数とレート
- レスポンス時間分布
- タイプ別エラー率
- アクティブ接続数
- 並行性使用率

## ベストプラクティス

1. **セキュリティ**: 本番環境では常に認証を使用する
2. **レート制限**: インフラストラクチャに基づいて適切な並行性制限を設定する
3. **監視**: 監査ログを有効にし、監視スタックと統合する
4. **エラーハンドリング**: ユースケースに適したエラーレスポンスを設定する
5. **エージェント設計**: webhook 固有の入力フォーマットを処理するようにエージェントを設計する
6. **リソース制限**: リソース枯渇を防ぐために合理的なボディサイズ制限を設定する

## 関連項目

- [はじめてのガイド](getting-started.md)
- [DSL ガイド](dsl-guide.md)
- [API リファレンス](api-reference.md)
- [エージェントランタイムドキュメント](../crates/runtime/README.md)

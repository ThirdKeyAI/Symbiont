# HTTP 入力モジュール

HTTP 入力モジュールは、外部システムが HTTP リクエストを通じて Symbiont エージェントを呼び出すことを可能にする webhook サーバーを提供します。このモジュールは、HTTP エンドポイントを通じてエージェントを公開することで、外部サービス、webhook、API との統合を可能にします。

## 概要

本ブランチでは、登録済みのエージェントがすでに稼働中であっても、各 HTTP 推論リクエストは独立して実行されます。登録されたソースとセキュリティティアが、推論の前に固定されたツールエグゼキューターを選択します。CPU、メモリ、実行時間の上限がその呼び出しを制約します。ガバナンス下のワーカーは、同じ非公開の状態ディレクトリを使用するスケジューラーや CLI の起動と、スーパーバイザーに設定された CPU、メモリ、ワーカープールを共有します。[共有予算](/shared-budgets)を参照してください。成功レスポンスには、`run_id`、`path`、`public_key` を含む `audit` が付きます。必須ストレージの失敗は以降の作用を停止させ、破棄されたリクエストもクリーンアップの所有権を保持します。[実行監査](/run-audit)および[ブランチガイド](/containment-branch-guide)を参照してください。

HTTP 入力モジュールは以下で構成されています：

- **HTTP サーバー**: 受信 HTTP リクエストをリッスンする Axum ベースの Web サーバー
- **認証**: Bearer トークンと JWT ベースの認証をサポート
- **リクエストルーティング**: 特定のエージェントにリクエストを向ける柔軟なルーティングルール
- **レスポンス制御**: 設定可能なレスポンスフォーマットとステータスコード
- **セキュリティ機能**: CORS サポート、リクエストサイズ制限、監査ログ
- **並行性管理**: 組み込みリクエストレート制限と並行性制御
- **ToolClad による LLM 呼び出し**: 各リクエストは、別の呼び出しが実行中である場合も含め、設定済みの LLM プロバイダーとガバナンス下の ORGA ツール呼び出しループを通じて、登録済みエージェントを独立して呼び出します

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

鍵ローダーは、EdDSA 検証のために Ed25519 の PEM または生の公開鍵バイト列を受け付けます。JWT には有効な `exp` と、空でない（最大 512 バイトの）`sub` が必要です。有効期限の検証では 5 秒のクロックスキューが許容されます。`iss` を指定する場合は、空でない最大 2,048 バイトの値である必要があります。署名された subject、issuer、設定された鍵が同じであれば、トークンを更新しても呼び出し元の識別情報は維持されます。

この HTTP 入力の検証器は、audience や issuer の許可リストを**強制しません**。設定された鍵がその信頼の根拠となるため、その権限専用の鍵を使用してください。署名された issuer は再試行時の識別に寄与しますが、issuer の許可リストを構成するものではありません。webhook 署名の検証を設定している場合でも Bearer 認証は必須であり、webhook 署名は追加の検査という位置づけです。

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
- **デフォルトでCORS無効**: `cors_origins` はデフォルトで空リスト。つまりCORSは無効です。クロスオリジンアクセスを有効にするには特定のオリジンを追加してください。`cors_origins` 内のリテラル `"*"` は**起動時に拒否されます** — HTTP 入力サーバーはワイルドカードオリジンでは起動を拒否します。（v1.13.0 監査で追加。`SECURITY_AUDIT.md` M1 を参照。）
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

[`webhook_handler.symbi`](../agents/webhook_handler.symbi) で webhook ハンドラーエージェントを作成：

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
  -H "Idempotency-Key: 72d6a833-b825-4b22-b50c-206337d77f7c" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

処理したいタスクごとに新しい UUID を選び、保持してください。HTTP での各送信には、`Idempotency-Key` ヘッダーをちょうど 1 つ付ける必要があります。再試行では同じ ID、URI、JSON ペイロードを使用します。この例の ID を別の処理に再利用すると拒否されます。webhook の送信側は、配信ごとに安定した UUID を保持するか、自身の配信識別子を安定した UUID に対応付けるアダプターを経由して送信する必要があります。サーバーは、モデルの出力から識別情報を推測したり、ヘッダーが欠落している場合に代替値を生成したりはしません。

### 再試行の状態

ID は、正規のプロジェクトごとに 1 つの HTTP ドメインを占有します。永続的なクレームは、検証済みの呼び出し元、リクエスト URI、JSON 入力、信頼された対象のソースと設定を束縛します。登録済みエージェントの ID は再起動で変わることがありますが、それによって別のリクエストになることはありません。スタンドアロンの SDK サーバーは、設定された `AgentId` を再起動後も保持する必要があります。既存の ID のもとでソース、対象、呼び出し元、ペイロードを変更した場合は処理を拒否します。キャッシュの内容や監査参照が、別の呼び出し元に返されることはありません。

| HTTP ステータス | ボディの `status` | 意味 |
|---|---|---|
| 既定では 200 | `completed` | 元の結果が永続化されています。`replayed` は保存済みレスポンスであることを示します。 |
| 422 | `failed` | 追跡された完全な証拠を伴う終端的な失敗が永続化されており、再試行でも同じ結果が返ります。 |
| 409 | `in_progress` | 別の所有者がその ID を保持しており、このリクエストは処理を開始しません。 |
| 409 | `unresolved` | 元の実行に突き合わせが必要です。可能な場合は監査参照が含まれます。 |
| 409 | `reconciled` | 個別に署名された運用者の評価を返します。元の ID で再度実行することはできません。 |
| 409 | `conflict` | その ID は別の呼び出し元またはリクエストに束縛されています。 |
| 400 | `invalid_invocation_id` | UUID ヘッダーが欠落、重複、または不正です。 |
| 503 | `unavailable` | 必須の呼び出しストレージが実行を認可できませんでした。 |

呼び出し状態のレスポンスには、`Idempotency-Key`、`Idempotency-Replayed`、`Cache-Control: no-store` の各ヘッダーが含まれます。完了した結果には設定済みの成功フォーマットが引き続き適用されますが、未解決や競合の結果を成功レスポンスに変えることはできません。設定された CORS オリジンは、これらの呼び出しヘッダーを許可し、公開します。

保持された所有者は、セットアップ、実行、クリーンアップ、結果の保存を通じてクレームを保持します。クライアントの切断はその所有者の処理をキャンセルしますが、ID が再び実行可能になるわけではありません。結果が永続化される前にプロセスが失われた場合、クレームは未解決のまま残ります。保存済みの結果は、返却前に元の署名付き監査と照合して検証されます。取得時にプロバイダーやエグゼキューターが再実行されることはありません。

静的な共有資格情報は 1 つの呼び出し元を表します。JWT の呼び出し元は、設定された鍵、署名された issuer と subject に束縛され、有効期限や更新に関するフィールドによって変わることはありません。資格情報や鍵素材をローテーションすると、既存の ID は黙って 2 つ目のタスクを作るのではなく競合になります。クレームはプロジェクト全体で HTTP リスナーをまたいで有効なため、新しいランダムな UUID を使用し、ストアを監査証拠とともに保全してください。ストレージの制限については[永続的な呼び出し識別子](/invocation-idempotency)を参照してください。

### 予期されるレスポンス

各推論リクエストは、同じエージェントの別の呼び出しが実行中である場合も含め、それぞれ独自の結果を返します。従来の `execution_started` / `message_id` による引き渡しレスポンスは、この経路では使用されなくなりました。成功レスポンスには、実行の公開監査参照、`invocation_id`、`replayed`、`total_usage`、および共有の `budget` スナップショットが含まれます。値の例を示します：

```json
{
  "status": "completed",
  "agent_id": "11111111-1111-4111-8111-111111111111",
  "response": "Task complete.",
  "tool_runs": [],
  "termination_reason": "Completed",
  "iterations": 1,
  "audit": {
    "run_id": "22222222-2222-4222-8222-222222222222",
    "path": "/srv/control/.symbiont/governed/11111111-1111-4111-8111-111111111111.22222222-2222-4222-8222-222222222222.jsonl",
    "public_key": "<hex-encoded-public-key>"
  },
  "model": "<configured-model>",
  "provider": "<configured-provider>",
  "latency_ms": 4821,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

`tool_runs` は、拒否や検証失敗を含め、対応づけられたツールの観測結果をまとめたものです。その存在は作用が実行されたことを証明するものではなく、`status: completed` がポリシーによる拒否のレスポンスに伴うこともあります。検証には、保護されたジャーナルにある正規化済みの正確な引数、判断、作用の記録を使用してください。必須の監査やクリーンアップが失敗した場合は、すでに作用が発生していたとしてもエラーを返します。`audit.path` はランタイムホスト上のパスであり、ダウンロード URL ではありません。

## ToolClad ツールによる LLM 呼び出し

各 HTTP 推論リクエストは、登録済みのエージェントがすでに別の呼び出しを実行している場合も含め、独立したガバナンス下の呼び出しを開始します。

### 仕組み

1. ランタイムが接続されている場合、信頼されたレジストリからエージェントを解決します。選択されたソース、サンドボックス、リソース設定を固定し、エージェントの欠落、曖昧な選択、セキュリティティアの不一致は推論の前に拒否します。スタンドアロンの SDK サーバーは、明示的に設定された汎用のエージェント／エグゼキューターを使用します。
2. システムプロンプトは、選択されたエージェントのソースのみから構築します。呼び出し元が任意で指定する `system_prompt` は引き続き長さ上限がありログにも記録されますが、ポリシー、プリンシパル、サンドボックスの権限を与えるものではありません。ユーザーメッセージはリクエストペイロードから構築します。
3. 固定されたプロジェクト内で ToolClad ツールを検出し、必須となる非公開の署名付きジャーナルを開きます。ORGA ループは最大 15 回の反復を許容します。登録時およびエージェントで選択された期限は、ループとツールの上限をより厳しくします。ツールあたりの既定の上限は 120 秒です。
4. 提案された呼び出しを Cedar の前に準備し正規化します。必須となる厳密な承認、必須の監査、単回限りの認可が作用に先行します。重複または空の呼び出し ID は拒否され、結果は実際に準備された呼び出しと対応づけられている必要があります。
5. ワーカーのクリーンアップと終端のジャーナル記録を待機します。成功レスポンスには、最終応答、ツールの結果、プロバイダー／モデルのメタデータ、`audit` 参照が含まれます。キャンセル時もクリーンアップの所有権は保持され、必須ストレージやクリーンアップのエラーが黙って成功結果になることはありません。

[準備済み呼び出し](/prepared-calls)および[実行監査](/run-audit)を参照してください。呼び出し単位のリソース上限は、リクエスト全体の受付制御を保証するものではありません。

### プロバイダーの自動検出

LLM クライアントはサーバー起動時に環境変数から初期化されます。API キーが設定されている最初のプロバイダーが次の順序で採用されます：

| 環境変数 | プロバイダー | モデルのオーバーライド | ベース URL のオーバーライド |
|---------|----------|----------------|-------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL`（デフォルト: `anthropic/claude-sonnet-4`） | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL`（デフォルト: `gpt-4o`） | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL`（デフォルト: `claude-sonnet-4-20250514`） | `ANTHROPIC_BASE_URL` |

推論プロバイダーが設定されていない場合、推論リクエストはエラーを返します。運用者が設定したローカルのエンドポイントは引き続きサポートされます。

### 入力フィールド

LLM パスが採用されるとき、webhook の JSON ボディは以下のように解釈されます：

- `prompt` または `message` — ユーザーメッセージとして使用されます。どちらも存在しない場合、ペイロード全体が pretty-print されてタスク記述として渡されます。
- `system_prompt` — DSL から派生したシステムプロンプトに追加される、呼び出し元が任意で指定するシステムプロンプト。4096 バイトに上限が設定され、ログに記録されます。プロンプトインジェクションの面として扱うこと：信頼できない呼び出し元にこのエンドポイントを公開する場合は必ず認証を強制してください。

### 正規化されたツール呼び出し形式

LLM クライアントは OpenAI/OpenRouter の関数呼び出しを、Anthropic Messages API が使用するのと同じコンテンツブロック形式に正規化します。プロバイダーに関係なく、各レスポンスのコンテンツブロックは `{"type": "text", "text": "..."}` または `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}` のいずれかであり、`stop_reason` は `"end_turn"` または `"tool_use"` です。

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
- **呼び出しの結果**: 前述の明示的な再試行状態を返します。未解決の処理が完了として報告されることはありません。
- **サーバーエラー**: 分類されないランタイム障害は、設定可能なステータスと汎用的な公開メッセージを返します。

## 監視と可観測性

### 監査ログ

`audit_enabled` が true の場合、モジュールはすべてのリクエストに関する構造化情報をログに記録します：

```log
INFO HTTP Input: Received request with 5 headers
INFO Agent webhook_handler is running, dispatching via communication bus
INFO Runtime execution dispatched for agent webhook_handler: message_id=… latency=3ms
```

LLM 呼び出しパスが使用されるとき、ORGA ループをトレースする追加の行が出力されます：

```log
INFO Agent webhook_handler is not running, using LLM invocation path
INFO Invoking LLM for agent webhook_handler: provider=Anthropic model=… tools=4 …
INFO ORGA ACT: executing tool 'nmap_scan' (id=…) for agent webhook_handler
INFO Tool 'nmap_scan' executed successfully
INFO ORGA loop iteration 1 for agent webhook_handler: executed 1 tool(s), continuing
INFO LLM invocation completed for agent webhook_handler: latency=4821ms tool_runs=1 response_len=…
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
- [推論ループ (ORGA)](reasoning-loop.md)
- [ToolClad ツールコントラクト](toolclad.md)
- [エージェントランタイムドキュメント](../crates/runtime/README.md)

運用者による解決が記録された呼び出しは、HTTP 409 と `status: "reconciled"`、元の監査参照、および個別に署名された `resolution` のレシートを返します。成功結果を捏造して返すことも、再度実行することもありません。[運用者による突き合わせ](/invocation-reconciliation)を参照してください。

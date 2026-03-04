---
layout: default
title: スケジューリングガイド
description: "Symbiont AIエージェント向けの本番レベルのcronベースタスクスケジューリング"
nav_exclude: true
---

# スケジューリングガイド

## 🌐 他の言語
{: .no_toc}

[English](scheduling.md) | [中文简体](scheduling.zh-cn.md) | [Español](scheduling.es.md) | [Português](scheduling.pt.md) | **日本語** | [Deutsch](scheduling.de.md)

---

## 概要

Symbiontのスケジューリングシステムは、AIエージェント向けの本番レベルのcronベースタスク実行機能を提供します。以下の機能をサポートしています：

- **cronスケジュール**: 定期タスク用の標準的なcron構文
- **ワンショットジョブ**: 指定時刻に一度だけ実行
- **ハートビートパターン**: 監視エージェント向けの継続的な評価-アクション-スリープサイクル
- **セッション分離**: エフェメラル、共有、または完全分離のエージェントコンテキスト
- **配信ルーティング**: 複数の出力チャネル（Stdout、LogFile、Webhook、Slack、Email、Custom）
- **ポリシー適用**: 実行前のセキュリティおよびコンプライアンスチェック
- **本番環境の堅牢化**: ジッター、同時実行制限、デッドレターキュー、AgentPin検証

## アーキテクチャ

スケジューリングシステムは3つのコアコンポーネントで構成されています：

```
┌─────────────────────┐
│   CronScheduler     │  バックグラウンドティックループ（1秒間隔）
│   (Tick Loop)       │  ジョブ選択と実行オーケストレーション
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  永続的なジョブストレージ
│   (Job Storage)     │  トランザクションサポート、状態管理
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  エージェント実行ランタイム
│ (Execution Engine)  │  AgentContextライフサイクル管理
└─────────────────────┘
```

### CronScheduler

`CronScheduler`は主要なエントリポイントです。以下を管理します：

- 1秒間隔で動作するバックグラウンドティックループ
- 次回実行時刻に基づくジョブ選択
- 同時実行制御とジッター挿入
- メトリクス収集とヘルスモニタリング
- 実行中ジョブの追跡を伴うグレースフルシャットダウン

### SqliteJobStore

`SqliteJobStore`は以下の機能を備えた永続的なジョブストレージを提供します：

- ジョブ状態更新のためのACIDトランザクション
- ジョブライフサイクル追跡（Active、Paused、Completed、Failed、DeadLetter）
- 監査証跡付きの実行履歴
- ステータス、エージェントIDなどによるフィルタリングクエリ機能

### DefaultAgentScheduler

`DefaultAgentScheduler`はスケジュールされたエージェントを実行します：

- 分離または共有の`AgentContext`インスタンスを作成
- セッションライフサイクル（作成、実行、破棄）を管理
- 設定されたチャネルへの配信をルーティング
- 実行前にポリシーゲートを適用

## DSL構文

### スケジュールブロックの構造

スケジュールブロックはSymbiont DSLファイルで定義されます：

```symbiont
schedule {
  name: "daily-report"
  agent: "reporter-agent"
  cron: "0 9 * * *"

  session_mode: "ephemeral_with_summary"
  delivery: ["stdout", "log_file"]

  policy {
    require_approval: false
    max_runtime: "5m"
  }
}
```

### Cron構文

6つのフィールドを持つ拡張cron構文（秒が先頭、オプションの7番目のフィールドは年）：

```
┌─────────────── 秒 (0-59)
│ ┌───────────── 分 (0-59)
│ │ ┌─────────── 時 (0-23)
│ │ │ ┌───────── 日 (1-31)
│ │ │ │ ┌─────── 月 (1-12)
│ │ │ │ │ ┌───── 曜日 (0-6, 日曜日 = 0)
│ │ │ │ │ │
* * * * * *
```

**例：**

```symbiont
# 毎日午前9時
cron: "0 0 9 * * *"

# 毎週月曜日の午後6時
cron: "0 0 18 * * 1"

# 15分ごと
cron: "0 */15 * * * *"

# 毎月1日の深夜0時
cron: "0 0 0 1 * *"
```

### ワンショットジョブ（At構文）

指定時刻に一度だけ実行するジョブの場合：

```symbiont
schedule {
  name: "deployment-check"
  agent: "health-checker"
  at: "2026-02-15T14:30:00Z"  # ISO 8601タイムスタンプ

  delivery: ["webhook"]
  webhook_url: "https://ops.example.com/hooks/deployment"
}
```

### ハートビートパターン

評価 → アクション → スリープの継続的な監視エージェント向け：

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "*/5 * * * *"  # 5分ごとに起動

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # 安全制限
  }
}
```

ハートビートエージェントは以下のサイクルに従います：

1. **評価**: システム状態を評価（例：メトリクス、ログの確認）
2. **アクション**: 必要に応じて是正措置を実行（例：サービスの再起動、運用チームへのアラート）
3. **スリープ**: 次のスケジュールされたティックまで待機

## CLIコマンド

`symbi cron`コマンドにより、完全なライフサイクル管理が可能です：

### ジョブ一覧

```bash
# すべてのジョブを一覧表示
symbi cron list

# ステータスでフィルタリング
symbi cron list --status active
symbi cron list --status paused

# エージェントでフィルタリング
symbi cron list --agent "reporter-agent"

# JSON出力
symbi cron list --format json
```

### ジョブ追加

```bash
# DSLファイルから追加
symbi cron add --file agent.symbi --schedule "daily-report"

# インライン定義（JSON）
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 * * * *"
}'
```

### ジョブ削除

```bash
# ジョブIDで削除
symbi cron remove <job-id>

# 名前で削除
symbi cron remove --name "daily-report"

# 強制削除（確認をスキップ）
symbi cron remove <job-id> --force
```

### 一時停止/再開

```bash
# ジョブを一時停止（スケジューリングを停止、状態は保持）
symbi cron pause <job-id>

# 一時停止中のジョブを再開
symbi cron resume <job-id>
```

### ステータス

```bash
# 次回実行時刻を含むジョブ詳細
symbi cron status <job-id>

# 直近10件の実行記録を含む
symbi cron status <job-id> --history 10

# ウォッチモード（5秒ごとに自動更新）
symbi cron status <job-id> --watch
```

### 即時実行

```bash
# 即時実行をトリガー（スケジュールをバイパス）
symbi cron run <job-id>

# カスタム入力付きで実行
symbi cron run <job-id> --input "Check production database"
```

### 履歴

```bash
# ジョブの実行履歴を表示
symbi cron history <job-id>

# 直近20件の実行
symbi cron history <job-id> --limit 20

# ステータスでフィルタリング
symbi cron history <job-id> --status failed

# CSVにエクスポート
symbi cron history <job-id> --format csv > runs.csv
```

## ハートビートパターン

### HeartbeatContextMode

ハートビートのイテレーション間でコンテキストがどのように保持されるかを制御します：

```rust
pub enum HeartbeatContextMode {
    /// イテレーションごとに新しいコンテキスト、実行履歴にサマリーを追加
    EphemeralWithSummary,

    /// すべてのイテレーションで共有コンテキスト（メモリが蓄積）
    SharedPersistent,

    /// イテレーションごとに新しいコンテキスト、サマリーなし（ステートレス）
    FullyEphemeral,
}
```

**EphemeralWithSummary（デフォルト）**：
- イテレーションごとに新しい`AgentContext`を作成
- 前回のイテレーションのサマリーをコンテキストに追加
- 無制限のメモリ増加を防止
- 関連するアクション間の継続性を維持

**SharedPersistent**：
- すべてのイテレーションで単一の`AgentContext`を再利用
- 完全な会話履歴を保持
- メモリ使用量が高い
- 深いコンテキストを必要とするエージェントに最適（例：デバッグセッション）

**FullyEphemeral**：
- イテレーションごとに新しい`AgentContext`、引き継ぎなし
- 最小のメモリフットプリント
- 独立したチェックに最適（例：APIヘルスプローブ）

### ハートビートエージェントの例

```symbiont
agent heartbeat_monitor {
  model: "claude-sonnet-4.5"
  system_prompt: """
  あなたはシステム監視エージェントです。各ハートビートで：
  1. システムメトリクス（CPU、メモリ、ディスク）を確認
  2. 最近のエラーログをレビュー
  3. 問題が検出された場合、アクションを実行：
     - 安全であればサービスを再起動
     - Slack経由で運用チームにアラート
     - インシデントの詳細をログに記録
  4. 結果をサマリー
  5. 完了したら'sleep'を返す
  """
}

schedule {
  name: "heartbeat-monitor"
  agent: "heartbeat_monitor"
  cron: "*/10 * * * *"  # 10分ごと

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 50
  }

  delivery: ["log_file", "slack"]
  slack_channel: "#ops-alerts"
}
```

## セッション分離

### セッションモード

```rust
pub enum SessionIsolationMode {
    /// サマリー引き継ぎ付きエフェメラルコンテキスト（デフォルト）
    EphemeralWithSummary,

    /// すべての実行で共有される永続コンテキスト
    SharedPersistent,

    /// 完全エフェメラル、状態の引き継ぎなし
    FullyEphemeral,
}
```

**設定：**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 2 * * *"

  # 実行ごとに新しいコンテキスト、前回実行のサマリーを含む
  session_mode: "ephemeral_with_summary"
}
```

### セッションライフサイクル

スケジュールされた各実行について：

1. **実行前**: 同時実行制限の確認、ジッターの適用
2. **セッション作成**: `session_mode`に基づいて`AgentContext`を作成
3. **ポリシーゲート**: ポリシー条件を評価
4. **実行**: 入力とコンテキストでエージェントを実行
5. **配信**: 設定されたチャネルに出力をルーティング
6. **セッションクリーンアップ**: モードに基づいてコンテキストを破棄または保持
7. **実行後**: 実行記録の更新、メトリクスの収集

## 配信ルーティング

### サポートされるチャネル

```rust
pub enum DeliveryChannel {
    Stdout,           // コンソールに出力
    LogFile,          // ジョブ固有のログファイルに追記
    Webhook,          // URLへのHTTP POST
    Slack,            // SlackウェブフックまたはAPI
    Email,            // SMTPメール
    Custom(String),   // ユーザー定義チャネル
}
```

### 設定例

**単一チャネル：**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 3 * * *"
  delivery: ["log_file"]
}
```

**複数チャネル：**

```symbiont
schedule {
  name: "security-scan"
  agent: "scanner"
  cron: "0 1 * * *"

  delivery: ["log_file", "slack", "email"]

  slack_channel: "#security"
  email_recipients: ["ops@example.com", "security@example.com"]
}
```

**Webhook配信：**

```symbiont
schedule {
  name: "metrics-report"
  agent: "metrics-agent"
  cron: "*/30 * * * *"

  delivery: ["webhook"]
  webhook_url: "https://metrics.example.com/ingest"
  webhook_headers: {
    "Authorization": "Bearer ${METRICS_API_KEY}"
    "Content-Type": "application/json"
  }
}
```

### DeliveryRouterトレイト

カスタム配信チャネルは以下を実装します：

```rust
#[async_trait]
pub trait DeliveryRouter: Send + Sync {
    async fn route(
        &self,
        channel: &DeliveryChannel,
        job: &CronJobDefinition,
        run: &JobRunRecord,
        output: &str,
    ) -> Result<(), SchedulerError>;
}
```

## ポリシー適用

### PolicyGate

`PolicyGate`は実行前にスケジュール固有のポリシーを評価します：

```rust
pub struct PolicyGate {
    policy_engine: Arc<RealPolicyParser>,
}

impl PolicyGate {
    pub async fn evaluate(
        &self,
        job: &CronJobDefinition,
        context: &AgentContext,
    ) -> Result<SchedulePolicyDecision, SchedulerError>;
}
```

### ポリシー条件

```symbiont
schedule {
  name: "production-deploy"
  agent: "deploy-agent"
  cron: "0 0 * * 0"  # 日曜深夜

  policy {
    # 実行前に人間の承認を要求
    require_approval: true

    # 強制終了までの最大実行時間
    max_runtime: "30m"

    # 特定のケイパビリティを要求
    require_capabilities: ["deployment", "production_write"]

    # 時間枠の適用（UTC）
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # 環境制限
    allowed_environments: ["staging", "production"]

    # AgentPin検証を要求
    require_agent_pin: true
  }
}
```

### SchedulePolicyDecision

```rust
pub enum SchedulePolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { approvers: Vec<String> },
}
```

## 本番環境の堅牢化

### ジッター

複数のジョブが同じスケジュールを共有する場合のサンダリングハード問題を防止します：

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // 0〜N秒のランダム遅延
    // ...
}
```

**例：**

```toml
[scheduler]
max_jitter_seconds = 30  # 30秒のウィンドウにジョブ開始を分散
```

### ジョブごとの同時実行制限

リソース枯渇を防ぐためにジョブごとの同時実行数を制限します：

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "*/5 * * * *"

  max_concurrent: 2  # 最大2つの同時実行を許可
}
```

ジョブが最大同時実行数で既に実行中の場合、スケジューラーはそのティックをスキップします。

### デッドレターキュー

`max_retries`を超えたジョブは手動レビューのために`DeadLetter`ステータスに移行します：

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 * * * *"

  max_retries: 3  # 3回失敗後、デッドレターに移動
}
```

**復旧：**

```bash
# デッドレター化されたジョブを一覧表示
symbi cron list --status dead_letter

# 失敗理由を確認
symbi cron history <job-id> --status failed

# 修正後にジョブをアクティブにリセット
symbi cron reset <job-id>
```

### AgentPin検証

実行前にエージェントのIDを暗号的に検証します：

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # agentpin-cliからのES256 JWT

  policy {
    require_agent_pin: true
  }
}
```

スケジューラーは以下を検証します：
1. ES256（ECDSA P-256）を使用したJWT署名
2. エージェントIDが`iss`クレームと一致
3. ドメインアンカーが期待されるオリジンと一致
4. 有効期限（`exp`）が有効

検証失敗時は`SecurityEventType::AgentPinVerificationFailed`監査イベントが発行されます。

## HTTP APIエンドポイント

### スケジュール管理

**POST /api/v1/schedule**
新しいスケジュールジョブを作成します。

```bash
curl -X POST http://localhost:8080/api/v1/schedule \
  -H "Content-Type: application/json" \
  -d '{
    "name": "hourly-report",
    "agent_id": "reporter",
    "cron_expr": "0 * * * *",
    "session_mode": "ephemeral_with_summary",
    "delivery": ["stdout"]
  }'
```

**GET /api/v1/schedule**
すべてのジョブを一覧表示（ステータス、エージェントIDでフィルタリング可能）。

```bash
curl "http://localhost:8080/api/v1/schedule?status=active&agent_id=reporter"
```

**GET /api/v1/schedule/{job_id}**
ジョブの詳細を取得します。

```bash
curl http://localhost:8080/api/v1/schedule/job-123
```

**PUT /api/v1/schedule/{job_id}**
ジョブを更新（cron式、配信先など）。

```bash
curl -X PUT http://localhost:8080/api/v1/schedule/job-123 \
  -H "Content-Type: application/json" \
  -d '{"cron_expr": "0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
ジョブを削除します。

```bash
curl -X DELETE http://localhost:8080/api/v1/schedule/job-123
```

**POST /api/v1/schedule/{job_id}/pause**
ジョブを一時停止します。

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/pause
```

**POST /api/v1/schedule/{job_id}/resume**
一時停止中のジョブを再開します。

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/resume
```

**POST /api/v1/schedule/{job_id}/run**
即時実行をトリガーします。

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
実行履歴を取得します。

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
次回スケジュール実行時刻を取得します。

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### ヘルスモニタリング

**GET /api/v1/health/scheduler**
スケジューラーのヘルスとメトリクス。

```bash
curl http://localhost:8080/api/v1/health/scheduler
```

**レスポンス：**

```json
{
  "status": "healthy",
  "active_jobs": 15,
  "paused_jobs": 3,
  "in_flight_jobs": 2,
  "metrics": {
    "runs_total": 1234,
    "runs_succeeded": 1180,
    "runs_failed": 54,
    "avg_execution_time_ms": 850
  }
}
```

## SDKの例

### JavaScript SDK

```javascript
import { SymbiontClient } from '@symbiont/sdk-js';

const client = new SymbiontClient({
  baseUrl: 'http://localhost:8080',
  apiKey: process.env.SYMBI_API_KEY
});

// スケジュールジョブを作成
const job = await client.schedule.create({
  name: 'daily-backup',
  agentId: 'backup-agent',
  cronExpr: '0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Created job: ${job.id}`);

// アクティブなジョブを一覧表示
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Active jobs: ${activeJobs.length}`);

// ジョブのステータスを取得
const status = await client.schedule.getStatus(job.id);
console.log(`Next run: ${status.next_run}`);

// 即時実行をトリガー
await client.schedule.runNow(job.id, { input: 'Backup database' });

// ジョブを一時停止
await client.schedule.pause(job.id);

// 履歴を表示
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Run ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// ジョブを再開
await client.schedule.resume(job.id);

// ジョブを削除
await client.schedule.delete(job.id);
```

### Python SDK

```python
from symbiont import SymbiontClient

client = SymbiontClient(
    base_url='http://localhost:8080',
    api_key=os.environ['SYMBI_API_KEY']
)

# スケジュールジョブを作成
job = client.schedule.create(
    name='hourly-metrics',
    agent_id='metrics-agent',
    cron_expr='0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Created job: {job.id}")

# 特定のエージェントのジョブを一覧表示
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"Found {len(jobs)} jobs for metrics-agent")

# ジョブの詳細を取得
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Next run: {details.next_run}")

# cron式を更新
client.schedule.update(job.id, cron_expr='*/30 * * * *')

# 即時実行をトリガー
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Run ID: {run.id}")

# メンテナンス中に一時停止
client.schedule.pause(job.id)
print("Job paused for maintenance")

# 最近の失敗を表示
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Failed run {run.id}: {run.error_message}")

# メンテナンス後に再開
client.schedule.resume(job.id)

# スケジューラーのヘルスを確認
health = client.schedule.health()
print(f"Scheduler status: {health.status}")
print(f"Active jobs: {health.active_jobs}")
print(f"In-flight jobs: {health.in_flight_jobs}")
```

## 設定

### CronSchedulerConfig

```rust
pub struct CronSchedulerConfig {
    /// ティック間隔（秒）（デフォルト：1）
    pub tick_interval_seconds: u64,

    /// サンダリングハード防止のための最大ジッター（デフォルト：0）
    pub max_jitter_seconds: u64,

    /// グローバル同時実行制限（デフォルト：10）
    pub max_concurrent_jobs: usize,

    /// メトリクス収集の有効化（デフォルト：true）
    pub enable_metrics: bool,

    /// デッドレターリトライ閾値（デフォルト：3）
    pub default_max_retries: u32,

    /// グレースフルシャットダウンタイムアウト（デフォルト：30秒）
    pub shutdown_timeout_seconds: u64,
}
```

### TOML設定

```toml
[scheduler]
tick_interval_seconds = 1
max_jitter_seconds = 30
max_concurrent_jobs = 20
enable_metrics = true
default_max_retries = 3
shutdown_timeout_seconds = 60

[scheduler.delivery]
# Webhook設定
webhook_timeout_seconds = 30
webhook_retry_attempts = 3

# Slack設定
slack_api_token = "${SLACK_API_TOKEN}"
slack_default_channel = "#ops"

# メール設定
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_username = "${SMTP_USER}"
smtp_password = "${SMTP_PASS}"
email_from = "symbiont@example.com"
```

### 環境変数

```bash
# スケジューラー設定
SYMBI_SCHEDULER_MAX_JITTER=30
SYMBI_SCHEDULER_MAX_CONCURRENT=20

# 配信設定
SYMBI_SLACK_TOKEN=xoxb-...
SYMBI_WEBHOOK_AUTH_HEADER="Bearer secret-token"

# AgentPin検証
SYMBI_AGENTPIN_REQUIRED=true
SYMBI_AGENTPIN_DOMAIN=agent.example.com
```

## 可観測性

### メトリクス（Prometheus互換）

```
# 合計実行数
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# 失敗した実行
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# 実行時間ヒストグラム
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# 実行中ジョブゲージ
symbiont_cron_in_flight_jobs 3

# デッドレター化されたジョブ
symbiont_cron_dead_letter_total{job_name="flaky-job"} 2
```

### 監査イベント

すべてのスケジューラーアクションはセキュリティイベントを発行します：

```rust
pub enum SecurityEventType {
    CronJobCreated,
    CronJobUpdated,
    CronJobDeleted,
    CronJobPaused,
    CronJobResumed,
    CronJobExecuted,
    CronJobFailed,
    CronJobDeadLettered,
    AgentPinVerificationFailed,
}
```

監査ログのクエリ：

```bash
symbi audit query --type CronJobFailed --since "2026-02-01" --limit 50
```

## ベストプラクティス

1. **共有スケジュールにはジッターを使用**: 複数のジョブが同時に開始するのを防止
2. **同時実行制限を設定**: リソース枯渇から保護
3. **デッドレターキューを監視**: 失敗しているジョブを定期的にレビューして修正
4. **EphemeralWithSummaryを使用**: 長時間実行されるハートビートでの無制限のメモリ増加を防止
5. **AgentPin検証を有効化**: エージェントのIDを暗号的に検証
6. **配信ルーティングを設定**: ジョブタイプに応じた適切なチャネルを使用
7. **ポリシーゲートを設定**: 時間枠、承認、ケイパビリティチェックを適用
8. **監視にはハートビートパターンを使用**: 継続的な評価-アクション-スリープサイクル
9. **ステージングでスケジュールをテスト**: 本番環境前にcron式とジョブロジックを検証
10. **メトリクスをエクスポート**: 運用の可視化のためにPrometheus/Grafanaと統合

## トラブルシューティング

### ジョブが実行されない

1. ジョブのステータスを確認: `symbi cron status <job-id>`
2. cron式を検証: [crontab.guru](https://crontab.guru/)を使用
3. スケジューラーのヘルスを確認: `curl http://localhost:8080/api/v1/health/scheduler`
4. ログを確認: `symbi logs --filter scheduler --level debug`

### ジョブが繰り返し失敗する

1. 履歴を表示: `symbi cron history <job-id> --status failed`
2. 実行記録のエラーメッセージを確認
3. エージェントの設定とケイパビリティを検証
4. スケジューラー外でエージェントをテスト: `symbi run <agent-id> --input "test"`
5. ポリシーゲートを確認: 時間枠とケイパビリティが一致しているか確認

### デッドレター化されたジョブ

1. デッドレタージョブを一覧表示: `symbi cron list --status dead_letter`
2. 失敗パターンを確認: `symbi cron history <job-id>`
3. 根本原因を修正（エージェントコード、権限、外部依存関係）
4. ジョブをリセット: `symbi cron reset <job-id>`

### 高メモリ使用量

1. セッションモードを確認: `ephemeral_with_summary`または`fully_ephemeral`に切り替え
2. ハートビートのイテレーションを削減: `max_iterations`を低く設定
3. コンテキストサイズを監視: エージェントの出力の冗長性を確認
4. コンテキストのアーカイブを有効化: 保持ポリシーを設定

## v0.9.0からの移行

v1.0.0リリースでは本番環境の堅牢化機能が追加されました。ジョブ定義を更新してください：

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 * * * *"
+
+  # 同時実行制限を追加
+  max_concurrent: 2
+
+  # ID検証のためのAgentPinを追加
+  agent_pin_jwt: "${AGENT_PIN_JWT}"
+
+  policy {
+    require_agent_pin: true
+  }
 }
```

設定を更新：

```diff
 [scheduler]
 tick_interval_seconds = 1
+ max_jitter_seconds = 30
+ default_max_retries = 3
+ shutdown_timeout_seconds = 60
```

破壊的なAPI変更はありません。すべてのv0.9.0ジョブは引き続き動作します。

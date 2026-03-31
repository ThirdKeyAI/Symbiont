# はじめに

このガイドでは、Symbiのセットアップと初めてのAIエージェントの作成について説明します。

## 目次


---

## 前提条件

Symbiを使い始める前に、以下がインストールされていることを確認してください：

### 必須の依存関係

- **Docker**（コンテナ化開発用）
- **Rust 1.82+**（ローカルビルドする場合）
- **Git**（リポジトリのクローン用）

### オプションの依存関係

- **SchemaPin Go CLI**（ツール検証用）

> **注意:** ベクトル検索は組み込みです。Symbiは[LanceDB](https://lancedb.com/)を組み込みベクトルデータベースとして同梱しており、外部サービスは不要です。

---

## インストール

### オプション1：Docker（推奨）

最も簡単に始める方法はDockerを使用することです：

```bash
# リポジトリをクローン
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# 統合symbiコンテナをビルド
docker build -t symbi:latest .

# または事前ビルドされたコンテナを使用
docker pull ghcr.io/thirdkeyai/symbi:latest

# 開発環境を実行
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### オプション2：ローカルインストール

ローカル開発の場合：

```bash
# リポジトリをクローン
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Rustの依存関係をインストールしてビルド
cargo build --release

# インストールを確認するためにテストを実行
cargo test
```

### インストールの確認

すべてが正常に動作することをテストします：

```bash
# DSLパーサーをテスト
cd crates/dsl && cargo run && cargo test

# ランタイムシステムをテスト
cd ../runtime && cargo test

# サンプルエージェントを実行
cargo run --example basic_agent
cargo run --example full_system

# 統合symbi CLIをテスト
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Dockerコンテナでテスト
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## プロジェクト初期化

新しいSymbiontプロジェクトを始める最も速い方法は `symbi init` です：

```bash
symbi init
```

これにより、以下の手順を案内するインタラクティブウィザードが起動します：
- **プロファイル選択**: `minimal`、`assistant`、`dev-agent`、または `multi-agent`
- **SchemaPinモード**: `tofu`（Trust-On-First-Use）、`strict`、または `disabled`
- **サンドボックスティア**: `tier0`（なし）、`tier1`（Docker）、または `tier2`（gVisor）

### 非インタラクティブモード

CI/CDやスクリプトセットアップの場合：

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

### プロファイル

| プロファイル | 作成されるもの |
|-------------|--------------|
| `minimal` | `symbiont.toml` + デフォルトCedarポリシー |
| `assistant` | + 単一のガバナンスアシスタントエージェント |
| `dev-agent` | + 安全ポリシー付きCliExecutorエージェント |
| `multi-agent` | + エージェント間ポリシー付きコーディネーター/ワーカーエージェント |

### カタログからのインポート

任意のプロファイルと共にビルド済みエージェントをインポート：

```bash
symbi init --profile minimal --no-interact
symbi init --catalog assistant,dev
```

利用可能なカタログエージェントを一覧：

```bash
symbi init --catalog list
```

初期化後、検証して起動：

```bash
symbi dsl -f agents/assistant.dsl   # エージェントを検証
symbi run assistant -i '{"query": "hello"}'  # 単一エージェントをテスト
symbi up                             # ランタイムを起動
```

### 単一エージェントの実行

完全なランタイムサーバーを起動せずに単一のエージェントを実行するには `symbi run` を使用します：

```bash
symbi run <agent-name-or-file> --input <json>
```

このコマンドはエージェント名を解決する際に、直接パス、次に `agents/` ディレクトリの順で検索します。環境変数（`OPENROUTER_API_KEY`、`OPENAI_API_KEY`、または `ANTHROPIC_API_KEY`）からクラウド推論をセットアップし、ORGA推論ループを実行して終了します。

```bash
symbi run assistant -i 'Summarize this document'
symbi run agents/recon.dsl -i '{"target": "10.0.1.5"}' --max-iterations 5
```

---

## 初めてのエージェント

Symbiの基本を理解するために、シンプルなデータ分析エージェントを作成してみましょう。

### 1. エージェント定義の作成

新しいファイル `my_agent.dsl` を作成します：

```rust
metadata {
    version = "1.0.0"
    author = "your-name"
    description = "My first Symbi agent"
}

agent greet_user(name: String) -> String {
    capabilities = ["greeting", "text_processing"]

    policy safe_greeting {
        allow: read(name) if name.length <= 100
        deny: store(name) if name.contains_sensitive_data
        audit: all_operations with signature
    }

    with memory = "ephemeral", privacy = "low" {
        if (validate_name(name)) {
            greeting = format_greeting(name);
            audit_log("greeting_generated", greeting.metadata);
            return greeting;
        } else {
            return "Hello, anonymous user!";
        }
    }
}
```

### 2. エージェントの実行

```bash
# エージェント定義を解析して検証
cargo run -- dsl parse my_agent.dsl

# ランタイムでエージェントを実行
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.dsl
```

---

## DSLの理解

Symbi DSLには以下のキーコンポーネントがあります：

### メタデータブロック

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

ドキュメントとランタイム管理のためのエージェントの基本情報を提供します。

### エージェント定義

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // エージェントの実装
}
```

エージェントのインターフェース、機能、動作を定義します。

### ポリシー定義

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

ランタイムで強制される宣言的セキュリティポリシーです。

### 実行コンテキスト

```rust
with memory = "persistent", privacy = "high" {
    // エージェントの実装
}
```

メモリ管理とプライバシー要件のランタイム設定を指定します。

---

## 次のステップ

### サンプルの探索

リポジトリには複数のサンプルエージェントが含まれています：

```bash
# 基本エージェントのサンプル
cd crates/runtime && cargo run --example basic_agent

# 完全なシステムデモ
cd crates/runtime && cargo run --example full_system

# コンテキストとメモリのサンプル
cd crates/runtime && cargo run --example context_example

# RAG強化エージェント
cd crates/runtime && cargo run --example rag_example
```

### 高度な機能の有効化

#### HTTP API（オプション）

```bash
# HTTP API機能を有効化
cd crates/runtime && cargo build --features http-api

# APIエンドポイントで実行
cd crates/runtime && cargo run --features http-api --example full_system
```

**主要APIエンドポイント：**
- `GET /api/v1/health` - ヘルスチェックとシステムステータス
- `GET /api/v1/agents` - リアルタイム実行ステータスを含むすべてのアクティブエージェント一覧
- `GET /api/v1/agents/{id}/status` - 詳細なエージェント実行メトリクスの取得
- `POST /api/v1/workflows/execute` - ワークフローを実行

**新しいエージェント管理機能：**
- リアルタイムプロセス監視とヘルスチェック
- 実行中のエージェントのグレースフルシャットダウン機能
- 包括的な実行メトリクスとリソース使用追跡
- 異なる実行モード（エフェメラル、永続、スケジュール、イベントドリブン）のサポート

#### クラウドLLM推論

OpenRouter経由でクラウドLLMプロバイダーに接続：

```bash
# クラウド推論を有効化
cargo build --features cloud-llm

# APIキーとモデルを設定
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # オプション
```

#### スタンドアロンエージェントモード

LLM推論とComposioツールアクセスを備えたクラウドネイティブエージェントのワンライナー：

```bash
cargo build --features standalone-agent
# 有効化: cloud-llm + composio
```

#### 高度な推論プリミティブ

ツールキュレーション、スタックループ検出、コンテキストプリフェッチ、スコープ付きコンベンションを有効化：

```bash
cargo build --features orga-adaptive
```

完全なドキュメントは[orga-adaptiveガイド](/orga-adaptive)を参照してください。

#### Cedarポリシーエンジン

Cedarポリシー言語による正式認可：

```bash
cargo build --features cedar
```

#### ベクトルデータベース（組み込み）

SymbiはLanceDBをゼロ設定の組み込みベクトルデータベースとして含んでいます。セマンティック検索とRAGは追加設定なしで動作します -- 別途サービスを起動する必要はありません：

```bash
# RAG機能を持つエージェントを実行（ベクトル検索はそのまま動作）
cd crates/runtime && cargo run --example rag_example

# 高度な検索を使用したコンテキスト管理のテスト
cd crates/runtime && cargo run --example context_example
```

> **エンタープライズオプション:** 専用のベクトルデータベースが必要なチームには、Qdrantがオプションのフィーチャーゲート付きバックエンドとして利用可能です。`SYMBIONT_VECTOR_BACKEND=qdrant` と `QDRANT_URL` を設定してください。

**コンテキスト管理機能：**
- **マルチモーダル検索**: キーワード、時間、類似性、ハイブリッド検索モード
- **重要度計算**: アクセスパターン、最新性、ユーザーフィードバックを考慮した高度なスコアリングアルゴリズム
- **アクセス制御**: エージェントスコープのアクセス制御を備えたポリシーエンジン統合
- **自動アーカイブ**: 圧縮ストレージとクリーンアップを備えた保持ポリシー
- **知識共有**: 信頼スコアを備えた安全なクロスエージェント知識共有

#### フィーチャーフラグリファレンス

| Feature | 説明 | デフォルト |
|---------|------|-----------|
| `keychain` | シークレット用OSキーチェーン統合 | はい |
| `vector-lancedb` | LanceDB組み込みベクトルバックエンド | はい |
| `vector-qdrant` | Qdrant分散ベクトルバックエンド | いいえ |
| `embedding-models` | Candle経由のローカルエンベディングモデル | いいえ |
| `http-api` | Swagger UI付きREST API | いいえ |
| `http-input` | JWT認証付きWebhookサーバー | いいえ |
| `cloud-llm` | クラウドLLM推論（OpenRouter） | いいえ |
| `composio` | Composio MCPツール統合 | いいえ |
| `standalone-agent` | クラウドLLM + Composioコンボ | いいえ |
| `cedar` | Cedarポリシーエンジン | いいえ |
| `orga-adaptive` | 高度な推論プリミティブ | いいえ |
| `cron` | 永続cronスケジューリング | いいえ |
| `native-sandbox` | ネイティブプロセスサンドボックス | いいえ |
| `metrics` | OpenTelemetryメトリクス/トレーシング | いいえ |
| `interactive` | `symbi init` のインタラクティブプロンプト（dialoguer） | デフォルト |
| `full` | エンタープライズ以外のすべての機能 | いいえ |

```bash
# 特定の機能でビルド
cargo build --features "cloud-llm,orga-adaptive,cedar"

# すべてでビルド
cargo build --features full
```

---

## 設定

### 環境変数

最適なパフォーマンスのために環境を設定します：

```bash
# 基本設定
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# ベクトル検索は組み込みのLanceDBバックエンドでそのまま動作します。
# 代わりにQdrantを使用する場合（オプション、エンタープライズ向け）：
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP統合（オプション）
export MCP_SERVER_URLS="http://localhost:8080"
```

### ランタイム設定

`symbi.toml` 設定ファイルを作成します：

```toml
[runtime]
max_agents = 1000
memory_limit_mb = 512
execution_timeout_seconds = 300

[security]
default_sandbox_tier = "docker"
audit_enabled = true
policy_enforcement = "strict"

[vector_db]
enabled = true
backend = "lancedb"              # デフォルト；"qdrant" もサポート
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # backend = "qdrant" の場合のみ必要
```

---

## よくある問題

### Dockerの問題

**問題**：Dockerビルドが権限エラーで失敗
```bash
# 解決策：Dockerデーモンが実行中で、ユーザーに権限があることを確認
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**問題**：コンテナがすぐに終了する
```bash
# 解決策：Dockerログを確認
docker logs <container_id>
```

### Rustビルドの問題

**問題**：Cargoビルドが依存関係エラーで失敗
```bash
# 解決策：Rustを更新してビルドキャッシュをクリア
rustup update
cargo clean
cargo build
```

**問題**：システム依存関係が不足
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### ランタイムの問題

**問題**：エージェントの開始に失敗
```bash
# エージェント定義の構文を確認
cargo run -- dsl parse your_agent.dsl

# デバッグログを有効化
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## ヘルプの取得

### ドキュメント

- **[DSLガイド](/dsl-guide)** - 完全なDSLリファレンス
- **[ランタイムアーキテクチャ](/runtime-architecture)** - システムアーキテクチャの詳細
- **[セキュリティモデル](/security-model)** - セキュリティとポリシーのドキュメント

### コミュニティサポート

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **ディスカッション**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **ドキュメント**: [完全なAPIリファレンス](https://docs.symbiont.dev/api-reference)

### デバッグモード

トラブルシューティングのため、詳細ログを有効化します：

```bash
# デバッグログを有効化
export RUST_LOG=symbi=debug

# 詳細出力で実行
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## 次は何ですか？

Symbiが動作するようになったので、これらの高度なトピックを探索してください：

1. **[DSLガイド](/dsl-guide)** - 高度なDSL機能を学ぶ
2. **[推論ループガイド](/reasoning-loop)** - ORGAサイクルを理解する
3. **[高度な推論 (orga-adaptive)](/orga-adaptive)** - ツールキュレーション、スタックループ検出、プリハイドレーション
4. **[ランタイムアーキテクチャ](/runtime-architecture)** - システム内部を理解する
5. **[セキュリティモデル](/security-model)** - セキュリティポリシーを実装する
6. **[コントリビューション](/contributing)** - プロジェクトに貢献する

素晴らしいものを構築する準備はできましたか？[サンプルプロジェクト](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)から始めるか、[完全な仕様](/specification)に深く入り込んでみてください。

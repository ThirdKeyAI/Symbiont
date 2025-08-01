---
layout: default
title: はじめに
nav_order: 2
description: "Symbiont クイックスタートガイド"
---

# はじめに
{: .no_toc }

## 🌐 他の言語

[English](getting-started.md) | [中文简体](getting-started.zh-cn.md) | [Español](getting-started.es.md) | [Português](getting-started.pt.md) | **日本語** | [Deutsch](getting-started.de.md)

---

このガイドでは、Symbi のセットアップと初めてのAIエージェントの作成について説明します。
{: .fs-6 .fw-300 }

## 目次
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 前提条件

Symbi を使い始める前に、以下がインストールされていることを確認してください：

### 必須の依存関係

- **Docker**（コンテナ化開発用）
- **Rust 1.88+**（ローカルビルドする場合）
- **Git**（リポジトリのクローン用）

### オプションの依存関係

- **Qdrant** ベクトルデータベース（セマンティック検索機能用）
- **SchemaPin Go CLI**（ツール検証用）

---

## インストール

### オプション1：Docker（推奨）

最も簡単に始める方法はDockerを使用することです：

```bash
# リポジトリをクローン
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# 統合 symbi コンテナをビルド
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

# Rust の依存関係をインストールしてビルド
cargo build --release

# インストールを確認するためにテストを実行
cargo test
```

### インストールの確認

すべてが正常に動作することをテストします：

```bash
# DSL パーサーをテスト
cd crates/dsl && cargo run && cargo test

# ランタイムシステムをテスト
cd ../runtime && cargo test

# サンプルエージェントを実行
cargo run --example basic_agent
cargo run --example full_system

# 統合 symbi CLI をテスト
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Docker コンテナでテスト
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## 初めてのエージェント

Symbi の基本を理解するために、シンプルなデータ分析エージェントを作成してみましょう。

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

## DSL の理解

Symbi DSL には以下のキーコンポーネントがあります：

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

# RAG 強化エージェント
cd crates/runtime && cargo run --example rag_example
```

### 高度な機能の有効化

#### HTTP API（オプション）

```bash
# HTTP API 機能を有効化
cd crates/runtime && cargo build --features http-api

# API エンドポイントで実行
cd crates/runtime && cargo run --features http-api --example full_system
```

**主要 API エンドポイント：**
- `GET /api/v1/health` - ヘルスチェックとシステムステータス
- `GET /api/v1/agents` - すべてのアクティブエージェントを一覧表示
- `POST /api/v1/workflows/execute` - ワークフローを実行

#### ベクトルデータベース統合

セマンティック検索機能用：

```bash
# Qdrant ベクトルデータベースを開始
docker run -p 6333:6333 qdrant/qdrant

# RAG 機能を持つエージェントを実行
cd crates/runtime && cargo run --example rag_example
```

---

## 設定

### 環境変数

最適なパフォーマンスのために環境を設定します：

```bash
# 基本設定
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# ベクトルデータベース（オプション）
export QDRANT_URL=http://localhost:6333

# MCP 統合（オプション）
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
url = "http://localhost:6333"
collection_name = "symbi_knowledge"
```

---

## よくある問題

### Docker の問題

**問題**：Docker ビルドが権限エラーで失敗
```bash
# 解決策：Docker デーモンが実行中で、ユーザーに権限があることを確認
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**問題**：コンテナがすぐに終了する
```bash
# 解決策：Docker ログを確認
docker logs <container_id>
```

### Rust ビルドの問題

**問題**：Cargo ビルドが依存関係エラーで失敗
```bash
# 解決策：Rust を更新してビルドキャッシュをクリア
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

- **[DSL ガイド](/dsl-guide)** - 完全な DSL リファレンス
- **[ランタイムアーキテクチャ](/runtime-architecture)** - システムアーキテクチャの詳細
- **[セキュリティモデル](/security-model)** - セキュリティとポリシーのドキュメント

### コミュニティサポート

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **ディスカッション**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **ドキュメント**: [完全な API リファレンス](https://docs.symbiont.platform)

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

Symbi が動作するようになったので、これらの高度なトピックを探索してください：

1. **[DSL ガイド](/dsl-guide)** - 高度な DSL 機能を学ぶ
2. **[ランタイムアーキテクチャ](/runtime-architecture)** - システム内部を理解する
3. **[セキュリティモデル](/security-model)** - セキュリティポリシーを実装する
4. **[コントリビューション](/contributing)** - プロジェクトに貢献する

素晴らしいものを構築する準備はできましたか？[サンプルプロジェクト](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)から始めるか、[完全な仕様](/specification)に深く入り込んでみてください。
---
layout: default
title: REPLガイド
description: "Symbiont REPLの使い方ガイド"
nav_exclude: true
---

# Symbiont REPLガイド

## 他の言語
{: .no_toc}

[English](repl-guide.md) | [中文简体](repl-guide.zh-cn.md) | [Español](repl-guide.es.md) | [Português](repl-guide.pt.md) | **日本語** | [Deutsch](repl-guide.de.md)

---

Symbiont REPL（Read-Eval-Print Loop）は、SymbiontエージェントとDSLコードの開発、テスト、デバッグのためのインタラクティブ環境を提供します。

## 機能

- **インタラクティブDSL評価**：Symbiont DSLコードをリアルタイムで実行
- **エージェントライフサイクル管理**：エージェントの作成、開始、停止、一時停止、再開、破棄
- **実行モニタリング**：統計とトレースによるエージェント実行のリアルタイム監視
- **ポリシー強制**：組み込みポリシーチェックとケイパビリティゲーティング
- **セッション管理**：REPLセッションのスナップショットとリストア
- **JSON-RPCプロトコル**：stdio経由のJSON-RPCによるプログラムアクセス
- **LSPサポート**：IDE統合のためのLanguage Server Protocol

## はじめに

### REPLの起動

```bash
# インタラクティブREPLモード
symbi repl

# JSON-RPCサーバーモード（stdio経由、IDE統合用）
symbi repl --stdio
```

> **注意：** `--config` フラグはまだサポートされていません。設定はデフォルトの `symbiont.toml` の場所から読み取られます。カスタム設定サポートは将来のリリースで予定されています。

### 基本的な使い方

```rust
# エージェントを定義
agent GreetingAgent {
  name: "Greeting Agent"
  version: "1.0.0"
  description: "A simple greeting agent"
}

# ビヘイビアを定義
behavior Greet {
  input { name: string }
  output { greeting: string }
  steps {
    let greeting = format("Hello, {}!", name)
    return greeting
  }
}

# 式を実行
let message = "Welcome to Symbiont"
print(message)
```

## REPLコマンド

### エージェント管理

| コマンド | 説明 |
|---------|------|
| `:agents` | すべてのエージェントを一覧表示 |
| `:agent list` | すべてのエージェントを一覧表示 |
| `:agent start <id>` | エージェントを開始 |
| `:agent stop <id>` | エージェントを停止 |
| `:agent pause <id>` | エージェントを一時停止 |
| `:agent resume <id>` | 一時停止したエージェントを再開 |
| `:agent destroy <id>` | エージェントを破棄 |
| `:agent execute <id> <behavior> [args]` | エージェントのビヘイビアを実行 |
| `:agent debug <id>` | エージェントのデバッグ情報を表示 |

### モニタリングコマンド

| コマンド | 説明 |
|---------|------|
| `:monitor stats` | 実行統計を表示 |
| `:monitor traces [limit]` | 実行トレースを表示 |
| `:monitor report` | 詳細な実行レポートを表示 |
| `:monitor clear` | モニタリングデータをクリア |

### メモリコマンド

| コマンド | 説明 |
|---------|------|
| `:memory inspect <agent-id>` | エージェントのメモリ状態を検査 |
| `:memory compact <agent-id>` | エージェントのメモリストレージを圧縮 |
| `:memory purge <agent-id>` | エージェントのすべてのメモリをパージ |

### Webhookコマンド

| コマンド | 説明 |
|---------|------|
| `:webhook list` | 設定されたwebhookを一覧表示 |
| `:webhook add` | 新しいwebhookを追加 |
| `:webhook remove` | webhookを削除 |
| `:webhook test` | webhookをテスト |
| `:webhook logs` | webhookログを表示 |

### レコーディングコマンド

| コマンド | 説明 |
|---------|------|
| `:record on <file>` | セッションのファイルへのレコーディングを開始 |
| `:record off` | セッションのレコーディングを停止 |

### セッションコマンド

| コマンド | 説明 |
|---------|------|
| `:snapshot` | セッションスナップショットを作成 |
| `:clear` | セッションをクリア |
| `:help` または `:h` | ヘルプメッセージを表示 |
| `:version` | バージョン情報を表示 |

## DSL機能

### エージェント定義

```rust
agent DataAnalyzer {
  name: "Data Analysis Agent"
  version: "2.1.0"
  description: "Analyzes datasets with privacy protection"

  security {
    capabilities: ["data_read", "analysis"]
    sandbox: true
  }

  resources {
    memory: 512MB
    cpu: 2
    storage: 1GB
  }
}
```

### ビヘイビア定義

```rust
behavior AnalyzeData {
  input {
    data: DataSet
    options: AnalysisOptions
  }
  output {
    results: AnalysisResults
  }

  steps {
    # データプライバシー要件を確認
    require capability("data_read")

    if (data.contains_pii) {
      return error("Cannot process data with PII")
    }

    # 分析を実行
    # 注意：analyze()は計画中の組み込み関数です（まだ実装されていません）。
    # この例は意図されたビヘイビア定義パターンを示しています。
    let results = analyze(data, options)
    emit analysis_completed { results: results }

    return results
  }
}
```

### 組み込み関数

| 関数 | 説明 | 例 |
|------|------|----|
| `print(...)` | 値を出力に表示 | `print("Hello", name)` |
| `len(value)` | 文字列、リスト、マップの長さを取得 | `len("hello")` -> `5` |
| `upper(string)` | 文字列を大文字に変換 | `upper("hello")` -> `"HELLO"` |
| `lower(string)` | 文字列を小文字に変換 | `lower("HELLO")` -> `"hello"` |
| `format(template, ...)` | 引数で文字列をフォーマット | `format("Hello, {}!", name)` |

> **計画中の組み込み関数：** `read_file()`、`read_csv()`、`write_results()`、`analyze()`、`transform_data()` などの高度なI/O関数はまだ実装されていません。これらは将来のリリースで予定されています。

### データ型

```rust
# 基本型
let name = "Alice"          # String
let age = 30               # Integer
let height = 5.8           # Number
let active = true          # Boolean
let empty = null           # Null

# コレクション
let items = [1, 2, 3]      # List
let config = {             # Map
  "host": "localhost",
  "port": 8080
}

# 時間とサイズの単位
let timeout = 30s          # Duration
let max_size = 100MB       # Size
```

## アーキテクチャ

### コンポーネント

```
symbi repl
├── repl-cli/          # CLIインターフェースとJSON-RPCサーバー
├── repl-core/         # コアREPLエンジンとエバリュエーター
├── repl-proto/        # JSON-RPCプロトコル定義
└── repl-lsp/          # Language Server Protocol実装
```

### コアコンポーネント

- **DslEvaluator**：ランタイム統合付きでDSLプログラムを実行
- **ReplEngine**：評価とコマンドハンドリングを調整
- **ExecutionMonitor**：実行統計とトレースを追跡
- **RuntimeBridge**：ポリシー強制のためにSymbiontランタイムと統合
- **SessionManager**：スナップショットとセッション状態を処理

### JSON-RPCプロトコル

REPLはプログラムアクセスのためにJSON-RPC 2.0をサポートしています：

```json
// DSLコードを評価
{
  "jsonrpc": "2.0",
  "method": "evaluate",
  "params": {"input": "let x = 42"},
  "id": 1
}

// レスポンス
{
  "jsonrpc": "2.0",
  "result": {"value": "42", "type": "integer"},
  "id": 1
}
```

## セキュリティとポリシー強制

### ケイパビリティチェック

REPLはエージェントセキュリティブロックで定義されたケイパビリティ要件を強制します：

```rust
agent SecureAgent {
  name: "Secure Agent"
  security {
    capabilities: ["filesystem", "network"]
    sandbox: true
  }
}

behavior ReadFile {
  input { path: string }
  output { content: string }
  steps {
    # エージェントが "filesystem" ケイパビリティを持っているかチェック
    require capability("filesystem")
    # 注意：read_file()は計画中の組み込み関数です（まだ実装されていません）。
    # この例はケイパビリティチェックの動作を示しています。
    let content = read_file(path)
    return content
  }
}
```

### ポリシー統合

REPLはSymbiontポリシーエンジンと統合してアクセス制御と監査要件を強制します。

## デバッグとモニタリング

### 実行トレース

```
:monitor traces 10

Recent Execution Traces:
  14:32:15.123 - AgentCreated [Agent: abc-123] (2ms)
  14:32:15.125 - AgentStarted [Agent: abc-123] (1ms)
  14:32:15.130 - BehaviorExecuted [Agent: abc-123] (5ms)
  14:32:15.135 - AgentPaused [Agent: abc-123]
```

### 統計

```
:monitor stats

Execution Monitor Statistics:
  Total Executions: 42
  Successful: 38
  Failed: 4
  Success Rate: 90.5%
  Average Duration: 12.3ms
  Total Duration: 516ms
  Active Executions: 2
```

### エージェントデバッグ

```
:agent debug abc-123

Agent Debug Information:
  ID: abc-123-def-456
  Name: Data Analyzer
  Version: 2.1.0
  State: Running
  Created: 2024-01-15 14:30:00 UTC
  Description: Analyzes datasets with privacy protection
  Author: data-team@company.com
  Available Functions/Behaviors: 5
  Required Capabilities: 2
    - data_read
    - analysis
  Resource Configuration:
    Memory: 512MB
    CPU: 2
    Storage: 1GB
```

## IDE統合

### Language Server Protocol

REPLは `repl-lsp` クレートを通じてIDE統合のためのLSPサポートを提供します。LSPサーバーはREPL自体とは別に起動されます：

```bash
# LSPサーバーはrepl-lspクレートによって提供され、
# エディタのLSPクライアント設定によって起動されます（symbi replフラグ経由ではありません）。
```

> **注意：** `--lsp` フラグは `symbi repl` ではサポートされていません。LSPは `repl-lsp` クレートで実装されており、エディタのLSP設定を通じて構成する必要があります。

### サポートされる機能

- シンタックスハイライト
- エラー診断
- テキスト同期

**計画中の機能**（まだ実装されていません）：
- コード補完
- ホバー情報
- 定義へ移動
- シンボル検索

## ベストプラクティス

### 開発ワークフロー

1. **シンプルな式から始める**：基本的なDSL構文をテスト
2. **エージェントを段階的に定義**：最小限のエージェント定義から開始
3. **ビヘイビアを個別にテスト**：統合前にビヘイビアを定義してテスト
4. **モニタリングを活用**：デバッグのために実行モニタリングを利用
5. **スナップショットを作成**：重要なセッション状態を保存

### パフォーマンスのヒント

- 定期的に `:monitor clear` でモニタリングデータをリセット
- `:monitor traces <limit>` でトレース履歴を制限
- 未使用のエージェントを破棄してリソースを解放
- 複雑なセッション状態にはスナップショットを使用

### セキュリティに関する考慮事項

- エージェントには常に適切なケイパビリティを定義
- 開発段階でポリシー強制をテスト
- 信頼できないコードにはサンドボックスモードを使用
- セキュリティイベントの実行トレースを監視

## トラブルシューティング

### よくある問題

**エージェント作成の失敗**
```
Error: Missing capability: filesystem
```
*解決策*：エージェントセキュリティブロックに必要なケイパビリティを追加

**実行タイムアウト**
```
Error: Maximum execution depth exceeded
```
*解決策*：ビヘイビアロジックの無限再帰を確認

**ポリシー違反**
```
Error: Policy violation: data access denied
```
*解決策*：エージェントが適切な権限を持っているか確認

### デバッグコマンド

```rust
# エージェント状態を確認
:agent debug <agent-id>

# 実行トレースを表示
:monitor traces 50

# システム統計を確認
:monitor stats

# デバッグスナップショットを作成
:snapshot
```

## 例

### シンプルなエージェント

```rust
agent Calculator {
  name: "Basic Calculator"
  version: "1.0.0"
}

behavior Add {
  input { a: number, b: number }
  output { result: number }
  steps {
    return a + b
  }
}

# ビヘイビアをテスト
let result = Add(5, 3)
print("5 + 3 =", result)
```

### データ処理エージェント

```rust
agent DataProcessor {
  name: "Data Processing Agent"
  version: "1.0.0"

  security {
    capabilities: ["data_read", "data_write"]
    sandbox: true
  }

  resources {
    memory: 256MB
    cpu: 1
  }
}

behavior ProcessCsv {
  input { file_path: string }
  output { summary: ProcessingSummary }

  steps {
    require capability("data_read")

    # 注意：read_csv()、transform_data()、write_results()は計画中の
    # 組み込み関数です（まだ実装されていません）。この例はデータ処理
    # ビヘイビアの意図されたパターンを示しています。
    let data = read_csv(file_path)
    let processed = transform_data(data)

    require capability("data_write")
    write_results(processed)

    return {
      "rows_processed": len(data),
      "status": "completed"
    }
  }
}
```

## 関連ドキュメント

- [DSLガイド](dsl-guide.md) - 完全なDSL言語リファレンス
- [ランタイムアーキテクチャ](runtime-architecture.md) - システムアーキテクチャ概要
- [セキュリティモデル](security-model.md) - セキュリティ実装の詳細
- [APIリファレンス](api-reference.md) - 完全なAPIドキュメント

---
layout: default
title: 高度な推論プリミティブ (symbi-dev)
description: "高度な推論ループプリミティブ：ツールキュレーション、スタックループ検出、コンテキストプリフェッチ、スコープ付きコンベンション"
nav_exclude: true
---

# 高度な推論プリミティブ
{: .no_toc }

## 他の言語
{: .no_toc}

[English](symbi-dev.md) | [中文简体](symbi-dev.zh-cn.md) | [Español](symbi-dev.es.md) | [Português](symbi-dev.pt.md) | **日本語** | [Deutsch](symbi-dev.de.md)

---

ツールキュレーション、スタックループ検出、決定的コンテキストプリフェッチ、ディレクトリスコープのコンベンション取得により推論ループを強化するフィーチャーゲートランタイムプリミティブ。
{: .fs-6 .fw-300 }

## 目次
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 概要

`symbi-dev` フィーチャーゲートは推論ループに4つの高度な機能を追加します：

| プリミティブ | 解決する問題 | モジュール |
|-------------|-------------|----------|
| **Tool Profile** | LLMがツールを多く見すぎ、無関係なツールにトークンを浪費 | `tool_profile.rs` |
| **Progress Tracker** | ループが同じ失敗ステップのリトライでスタック | `progress_tracker.rs` |
| **Pre-Hydration** | コールドスタートのコンテキストギャップ -- エージェントが自分で参照を発見する必要がある | `pre_hydrate.rs` |
| **Scoped Conventions** | コンベンション取得が言語全体であり、ディレクトリ固有でない | `knowledge_bridge.rs` |

### 有効化

```toml
# Cargo.toml に記述
[dependencies]
symbi-runtime = { version = "1.6", features = ["symbi-dev"] }
```

またはソースからビルド：

```bash
cargo build --features symbi-dev
cargo test --features symbi-dev
```

すべてのプリミティブは追加的で後方互換性があります -- 既存のコードはフィーチャーゲートなしでも同一にコンパイルおよび実行されます。

---

## ツールプロファイルフィルタリング

LLMがツール定義を見る前にフィルタリングします。トークンの浪費を削減し、モデルが無関係なツールを選択するのを防ぎます。

### 設定

```rust
use symbi_runtime::reasoning::ToolProfile;

// ファイル関連ツールのみ含める
let profile = ToolProfile::include_only(&["file_*", "code_*"]);

// デバッグツールを除外
let profile = ToolProfile::exclude_only(&["debug_*", "internal_*"]);

// 組み合わせ：webツールを含め、実験的なものを除外、最大10に制限
let profile = ToolProfile {
    include: vec!["web_*".into(), "search_*".into()],
    exclude: vec!["web_experimental_*".into()],
    max_tools: Some(10),
    require_verified: false,
};
```

### フィルタリングパイプライン

パイプラインは順番に適用されます：

1. **Include** -- 空でない場合、いずれかのincludeグロブにマッチするツールのみ通過
2. **Exclude** -- いずれかのexcludeグロブにマッチするツールは除去
3. **Verified** -- `require_verified` がtrueの場合、説明に `[verified]` があるツールのみ通過
4. **Max cap** -- 設定されている場合、`max_tools` に切り詰め

### グロブ構文

| パターン | マッチ |
|---------|--------|
| `web_*` | `web_search`、`web_fetch`、`web_scrape` |
| `tool_?` | `tool_a`、`tool_1`（単一文字） |
| `exact_name` | `exact_name` のみ |

### LoopConfigとの統合

```rust
let config = LoopConfig {
    tool_profile: Some(ToolProfile::include_only(&["search_*", "file_*"])),
    ..Default::default()
};
```

プロファイルは `ReasoningLoopRunner::run()` でエクゼキューターとナレッジブリッジからツール定義が投入された後に自動的に適用されます。

---

## プログレストラッカー

ステップごとのリトライ回数を追跡し、正規化されたレーベンシュタイン類似度を使用して連続するエラー出力を比較してスタックループを検出します。

### 設定

```rust
use symbi_runtime::reasoning::{ProgressTracker, StepIterationConfig, LimitAction};

let config = StepIterationConfig {
    max_reattempts_per_step: 2,    // 2回の失敗試行後に停止
    similarity_threshold: 0.85,    // 85%以上の類似度 = スタック
    on_limit_reached: LimitAction::SkipStep,
};

let mut tracker = ProgressTracker::new(config);
```

### 使い方（コーディネーターレベル）

プログレストラッカーは**推論ループに直接組み込まれていません** -- マルチステップタスクをオーケストレーションするコーディネーターの上位レベルの関心事です。

```rust
// ステップの追跡を開始
tracker.begin_step("extract_data");

// 各試行後にエラーを記録してチェック
let decision = tracker.record_and_check("extract_data", &error_output);

match decision {
    StepDecision::Continue => { /* リトライ */ }
    StepDecision::Stop { reason } => {
        // LoopEvent::StepLimitReached を発行して次へ
        match tracker.limit_action() {
            LimitAction::SkipStep => { /* 次のステップへスキップ */ }
            LimitAction::AbortTask => { /* タスク全体を中止 */ }
            LimitAction::Escalate => { /* 人間に引き渡し */ }
        }
    }
}
```

### スタック検出

トラッカーは連続するエラー出力間の正規化レーベンシュタイン距離を計算します。類似度が閾値（デフォルト85%）を超えると、最大リトライ回数に達していなくてもステップはスタックと見なされます。

これは、エージェントがわずかに異なる表現で同じエラーに繰り返しヒットするシナリオをキャッチします。

---

## プリハイドレーションエンジン

タスク入力から参照（URL、ファイルパス、GitHubイシュー/PR）を抽出し、推論ループ開始前に並行して解決します。これにより、エージェントがこれらの参照を自分で発見してフェッチする必要があるコールドスタートレイテンシを排除します。

### 設定

```rust
use symbi_runtime::reasoning::PreHydrationConfig;
use std::time::Duration;

let config = PreHydrationConfig {
    custom_patterns: vec![],
    resolution_tools: [
        ("url".into(), "web_fetch".into()),
        ("file".into(), "file_read".into()),
    ].into(),
    timeout: Duration::from_secs(15),
    max_references: 10,
    max_context_tokens: 4000,  // 1 token ~ 4 chars
};
```

### 組み込みパターン

| パターン | タイプ | マッチ例 |
|---------|--------|---------|
| URL | `url` | `https://example.com/api`、`http://localhost:3000` |
| ファイルパス | `file` | `./src/main.rs`、`~/config.toml` |
| イシュー | `issue` | `#42`、`#100` |
| プルリクエスト | `pr` | `PR #55`、`pr #12` |

### カスタムパターン

```rust
use symbi_runtime::reasoning::pre_hydrate::ReferencePattern;

let config = PreHydrationConfig {
    custom_patterns: vec![
        ReferencePattern {
            ref_type: "jira".into(),
            pattern: r"[A-Z]+-\d+".into(),  // PROJ-123
        },
    ],
    ..Default::default()
};
```

### 解決フロー

1. **Extract** -- 正規表現パターンがタスク入力をスキャンし、マッチを重複排除
2. **Resolve** -- 各参照は設定されたツール（例：URLの場合は `web_fetch`）で解決
3. **Budget** -- 結果は `max_context_tokens` 内に収まるよう刈り込み
4. **Inject** -- `[PRE_HYDRATED_CONTEXT]` システムメッセージとしてフォーマット（ナレッジブリッジの `[KNOWLEDGE_CONTEXT]` スロットとは別）

### LoopConfigとの統合

```rust
let config = LoopConfig {
    pre_hydration: Some(PreHydrationConfig {
        resolution_tools: [("url".into(), "web_fetch".into())].into(),
        ..Default::default()
    }),
    ..Default::default()
};
```

プリハイドレーションはメインの推論ループが始まる前に `run_inner()` の開始時に自動的に実行されます。抽出と解決の統計を含む `LoopEvent::PreHydrationComplete` ジャーナルイベントが発行されます。

---

## ディレクトリスコープのコンベンション

特定のディレクトリにスコープされたコーディングコンベンションを取得するための `directory` と `scope` パラメータで `recall_knowledge` ツールを拡張します。

### 動作原理

`scope: "conventions"` と `directory` で呼び出された場合、ナレッジブリッジは：

1. ディレクトリパスにマッチするコンベンションを検索
2. 親ディレクトリを遡る（例：`src/api/` -> `src/` -> プロジェクトルート）
3. 言語レベルのコンベンションにフォールバック
4. すべてのレベルでコンテンツによる重複排除
5. リクエストされた上限に切り詰め

### LLMツール呼び出し

```json
{
  "name": "recall_knowledge",
  "arguments": {
    "query": "rust",
    "directory": "src/api/handlers",
    "scope": "conventions"
  }
}
```

### 後方互換性

`directory` と `scope` パラメータはオプションです。これらなしでは、`recall_knowledge` は標準バージョンと同一に動作します -- `query` と `limit` によるプレーンなナレッジ検索です。

---

## LoopConfigフィールド

`symbi-dev` featureが有効な場合、`LoopConfig` に3つのオプションフィールドが追加されます：

```rust
pub struct LoopConfig {
    // ... 既存フィールド ...

    /// LLMに見えるツールをフィルタリングするためのツールプロファイル。
    pub tool_profile: Option<ToolProfile>,
    /// スタックループ検出のためのステップごとのイテレーション制限。
    pub step_iteration: Option<StepIterationConfig>,
    /// 決定的コンテキストプリフェッチのためのプリハイドレーション設定。
    pub pre_hydration: Option<PreHydrationConfig>,
}
```

すべてデフォルトは `None` で、後方互換性のために `#[serde(default, skip_serializing_if = "Option::is_none")]` でシリアライズされます。

## ジャーナルイベント

2つの新しい `LoopEvent` バリアントが利用可能です：

```rust
pub enum LoopEvent {
    // ... 既存バリアント ...

    /// ステップがリトライ上限に達した（コーディネーターにより発行）。
    StepLimitReached {
        step_id: String,
        attempts: u32,
        reason: String,
    },
    /// プリハイドレーションフェーズが完了。
    PreHydrationComplete {
        references_found: usize,
        references_resolved: usize,
        references_failed: usize,
        total_tokens: usize,
    },
}
```

---

## テスト

```bash
# featureなし（回帰なし）
cargo clippy --workspace -j2
cargo test --workspace -j2

# featureあり
cargo clippy --workspace -j2 --features symbi-dev
cargo test --workspace -j2 --features symbi-dev
```

すべてのテストはインラインの `#[cfg(test)]` モジュールです -- 外部テストフィクスチャは不要です。

---

## モジュールマップ

| モジュール | 公開型 | 説明 |
|-----------|--------|------|
| `tool_profile` | `ToolProfile` | verifiedフラグとmax capを備えたグロブベースのツールフィルタリング |
| `progress_tracker` | `ProgressTracker`、`StepIterationConfig`、`StepDecision`、`LimitAction` | レーベンシュタインスタック検出を備えたステップごとのイテレーション追跡 |
| `pre_hydrate` | `PreHydrationEngine`、`PreHydrationConfig`、`HydratedContext` | 参照抽出、並行解決、トークンバジェット刈り込み |
| `knowledge_bridge` | （拡張） | `retrieve_scoped_conventions()`、拡張 `recall_knowledge` ツール |

---

## 次のステップ

- **[推論ループガイド](reasoning-loop.md)** -- コアORGAサイクルのドキュメント
- **[ランタイムアーキテクチャ](runtime-architecture.md)** -- 完全なシステムアーキテクチャ概要
- **[APIリファレンス](api-reference.md)** -- 完全なAPIドキュメント

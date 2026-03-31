<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | [Português](README.pt.md) | **日本語** | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

## 🚀 Symbiontとは？

**Symbi** は自律的でポリシー対応の AI エージェントを構築するための **Rust ネイティブ、ゼロトラストエージェントフレームワーク**です。
LangChain や AutoGPT などの既存フレームワークの最大の欠陥を、以下に焦点を当てることで修正します：

* **セキュリティファースト**：暗号化監査証跡、強制ポリシー、サンドボックス化。
* **ゼロトラスト**：すべての入力はデフォルトで信頼できないものとして扱われます。
* **エンタープライズグレードコンプライアンス**：規制業界（HIPAA、SOC2、金融）向けに設計。

Symbiont エージェントは人間、ツール、LLM と安全に協働します — セキュリティやパフォーマンスを犠牲にすることなく。

---

## ⚡ なぜ Symbiont？

| 機能         | Symbiont                          | LangChain      | AutoGPT   |
| ------------ | --------------------------------- | -------------- | --------- |
| 言語         | Rust（安全性、パフォーマンス）    | Python         | Python    |
| セキュリティ | ゼロトラスト、暗号化監査          | 最小限         | なし      |
| ポリシーエンジン | 組み込み DSL                  | 限定的         | なし      |
| デプロイメント | REPL、Docker、HTTP API         | Python スクリプト | CLI ハック |
| 監査証跡     | 暗号化ログ                        | なし           | なし      |

---

## 🏁 クイックスタート

### 前提条件

* Docker（推奨）または Rust 1.82+
* 外部ベクターデータベース不要（LanceDB 組み込み済み。スケールデプロイメントには Qdrant もオプションで利用可能）

### ビルド済みコンテナで実行

```bash
# エージェント DSL ファイルを解析
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# MCP Server を実行
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# インタラクティブ開発シェル
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### ソースからビルド

```bash
# 開発環境をビルド
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# 統合バイナリをビルド
cargo build --release

# REPL を実行
cargo run -- repl

# DSL を解析して MCP を実行
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080
```

---

## 🔧 主要機能

* ✅ **DSL 文法** – 組み込みセキュリティポリシーでエージェントを宣言的に定義。
* ✅ **エージェントランタイム** – タスクスケジューリング、リソース管理、ライフサイクル制御。
* 🔒 **サンドボックス化** – エージェント実行のための Tier-1 Docker 隔離。
* 🔒 **SchemaPin セキュリティ** – ツールとスキーマの暗号化検証。
* 🔒 **シークレット管理** – HashiCorp Vault / OpenBao 統合、AES-256-GCM 暗号化ストレージ。
* 📊 **RAG エンジン** – ハイブリッドセマンティック + キーワード検索によるベクター検索（LanceDB 組み込み）。スケールデプロイメント向けに Qdrant バックエンドもオプションで対応。
* 🧩 **MCP 統合** – モデルコンテキストプロトコルツールのネイティブサポート。
* 📡 **オプション HTTP API** – 外部統合のための機能ゲート REST インターフェース。

---

## 📐 Symbiont DSL 例

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]
    
    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }
    
    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

---

## 🔒 セキュリティモデル

* **ゼロトラスト** – すべてのエージェント入力はデフォルトで信頼できません。
* **サンドボックス実行** – プロセスの Docker ベース封じ込め。
* **監査ログ** – 暗号化的に改ざん防止されたログ。
* **シークレット制御** – Vault/OpenBao バックエンド、暗号化ローカルストレージ、エージェント名前空間。

---

## 📚 ドキュメント

* [はじめに](https://docs.symbiont.dev/getting-started)
* [DSL ガイド](https://docs.symbiont.dev/dsl-guide)
* [ランタイムアーキテクチャ](https://docs.symbiont.dev/runtime-architecture)
* [セキュリティモデル](https://docs.symbiont.dev/security-model)
* [API リファレンス](https://docs.symbiont.dev/api-reference)

---

## 🎯 使用例

* **開発と自動化**

  * 安全なコード生成とリファクタリング。
  * 強制ポリシーによる AI エージェントデプロイメント。
  * セマンティック検索による知識管理。

* **エンタープライズと規制業界**

  * ヘルスケア（HIPAA 準拠処理）。
  * 金融（監査対応ワークフロー）。
  * 政府（機密コンテキスト処理）。
  * 法務（機密文書分析）。

---

## 📄 ライセンス

* **Community エディション**：Apache 2.0 ライセンス
* **Enterprise エディション**：商用ライセンスが必要

エンタープライズライセンスについては [ThirdKey](https://thirdkey.ai) にお問い合わせください。

---

*Symbiont は、インテリジェントなポリシー適用、暗号化検証、包括的な監査証跡を通じて、AI エージェントと人間の安全な協働を可能にします。*


<div align="right">
  <img src="symbi-trans.png" alt="Symbi ロゴ" width="120">
</div>
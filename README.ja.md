<img src="logo-hz.png" alt="Symbi">

**Symbi**は、人間、他のエージェント、大規模言語モデルと安全に協働できる自律的でポリシー対応エージェントを構築するための、AI ネイティブなエージェントフレームワークです。Community エディションでは、高度なセキュリティ、監視、コラボレーションのためのオプションの Enterprise 機能と共に、コア機能を提供します。

## 🚀 クイックスタート

### 前提条件
- Docker（推奨）または Rust 1.88+
- Qdrant ベクターデータベース（セマンティック検索用）

### ビルド済みコンテナでの実行

**GitHub Container Registry の使用（推奨）:**

```bash
# symbi 統合 CLI の実行
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# MCP Server の実行
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# インタラクティブ開発
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### ソースからのビルド

```bash
# 開発環境のビルド
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# symbi 統合バイナリのビルド
cargo build --release

# コンポーネントのテスト
cargo test

# サンプルエージェントの実行（crates/runtime から）
cd crates/runtime && cargo run --example basic_agent
cd crates/runtime && cargo run --example full_system
cd crates/runtime && cargo run --example rag_example

# symbi 統合 CLI の使用
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080

# HTTP API の有効化（オプション）
cd crates/runtime && cargo run --features http-api --example full_system
```

### オプションの HTTP API

外部統合のための RESTful HTTP API を有効化：

```bash
# HTTP API 機能付きでビルド
cargo build --features http-api

# または Cargo.toml に追加
[dependencies]
symbi-runtime = { version = "0.1.2", features = ["http-api"] }
```

**主要エンドポイント:**
- `GET /api/v1/health` - ヘルスチェックとシステムステータス
- `GET /api/v1/agents` - 全アクティブエージェントのリスト
- `POST /api/v1/workflows/execute` - ワークフローの実行
- `GET /api/v1/metrics` - システムメトリクス

## 📁 プロジェクト構造

```
symbi/
├── src/                   # symbi 統合 CLI バイナリ
├── crates/                # ワークスペースクレート
│   ├── dsl/              # Symbi DSL 実装
│   │   ├── src/          # パーサーとライブラリコード
│   │   ├── tests/        # DSL テストスイート
│   │   └── tree-sitter-symbiont/ # 文法定義
│   └── runtime/          # エージェントランタイムシステム（Community）
│       ├── src/          # コアランタイムコンポーネント
│       ├── examples/     # 使用例
│       └── tests/        # 統合テスト
├── docs/                 # ドキュメント
└── Cargo.toml           # ワークスペース設定
```

## 🔧 機能

### ✅ Community 機能（OSS）
- **DSL 文法**: エージェント定義用の完全な Tree-sitter 文法
- **エージェントランタイム**: タスクスケジューリング、リソース管理、ライフサイクル制御
- **Tier 1 隔離**: エージェント操作のための Docker によるコンテナ隔離
- **MCP 統合**: 外部ツール用のモデルコンテキストプロトコルクライアント
- **SchemaPin セキュリティ**: ツールの基本的な暗号化検証
- **RAG エンジン**: ベクター検索による検索拡張生成
- **コンテキスト管理**: エージェントの永続メモリと知識保存
- **ベクターデータベース**: セマンティック検索のための Qdrant 統合
- **包括的シークレット管理**: 複数認証方法での HashiCorp Vault 統合
- **暗号化ファイルバックエンド**: OS キーリング統合による AES-256-GCM 暗号化
- **シークレット CLI ツール**: 監査証跡付きの完全な暗号化/復号化/編集操作
- **HTTP API**: オプションの RESTful インターフェース（機能制御）

### 🏢 Enterprise 機能（ライセンス必要）
- **高度な隔離**: gVisor および Firecracker 隔離 **（Enterprise）**
- **AI ツールレビュー**: 自動化されたセキュリティ分析ワークフロー **（Enterprise）**
- **暗号化監査**: Ed25519 署名付きの完全な監査証跡 **（Enterprise）**
- **マルチエージェント通信**: エージェント間の暗号化メッセージング **（Enterprise）**
- **リアルタイム監視**: SLA メトリクスとパフォーマンスダッシュボード **（Enterprise）**
- **プロフェッショナルサービスとサポート**: カスタム開発とサポート **（Enterprise）**

## 📐 Symbiont DSL

組み込みポリシーと機能を持つインテリジェントエージェントを定義：

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

## 🔐 シークレット管理

Symbi は複数のバックエンドオプションを持つエンタープライズグレードのシークレット管理を提供：

### バックエンドオプション
- **HashiCorp Vault**: 複数認証方法によるプロダクション対応シークレット管理
  - トークンベース認証
  - Kubernetes サービスアカウント認証
- **暗号化ファイル**: OS キーリング統合による AES-256-GCM ローカル暗号化ストレージ
- **エージェント名前空間**: 隔離のためのエージェントスコープシークレットアクセス

### CLI 操作
```bash
# シークレットファイルの暗号化
symbi secrets encrypt config.json --output config.enc

# シークレットファイルの復号化
symbi secrets decrypt config.enc --output config.json

# 暗号化シークレットの直接編集
symbi secrets edit config.enc

# Vault バックエンドの設定
symbi secrets configure vault --endpoint https://vault.company.com
```

### 監査とコンプライアンス
- 全シークレット操作の完全な監査証跡
- 暗号化整合性検証
- エージェントスコープアクセス制御
- 改ざん防止ログ

## 🔒 セキュリティモデル

### 基本セキュリティ（Community）
- **Tier 1 隔離**: Docker によるコンテナ化エージェント実行
- **スキーマ検証**: SchemaPin による暗号化ツール検証
- **ポリシーエンジン**: 基本的なリソースアクセス制御
- **シークレット管理**: Vault と暗号化ファイルストレージ統合
- **監査ログ**: 操作追跡とコンプライアンス

### 高度なセキュリティ（Enterprise）
- **強化隔離**: gVisor（Tier2）および Firecracker（Tier3）隔離 **（Enterprise）**
- **AI セキュリティレビュー**: 自動化ツール分析と承認 **（Enterprise）**
- **暗号化通信**: エージェント間セキュアメッセージング **（Enterprise）**
- **包括的監査**: 暗号化整合性保証 **（Enterprise）**

## 🧪 テスト

```bash
# 全テストの実行
cargo test

# 特定コンポーネントの実行
cd crates/dsl && cargo test          # DSL パーサー
cd crates/runtime && cargo test     # ランタイムシステム

# 統合テスト
cd crates/runtime && cargo test --test integration_tests
cd crates/runtime && cargo test --test rag_integration_tests
cd crates/runtime && cargo test --test mcp_client_tests
```

## 📚 ドキュメント

- **[はじめに](https://docs.symbiont.dev/getting-started)** - インストールと最初のステップ
- **[DSL ガイド](https://docs.symbiont.dev/dsl-guide)** - 完全な言語リファレンス
- **[ランタイムアーキテクチャ](https://docs.symbiont.dev/runtime-architecture)** - システム設計
- **[セキュリティモデル](https://docs.symbiont.dev/security-model)** - セキュリティ実装
- **[API リファレンス](https://docs.symbiont.dev/api-reference)** - 完全な API ドキュメント
- **[貢献](https://docs.symbiont.dev/contributing)** - 開発ガイドライン

### 技術リファレンス
- [`crates/runtime/README.md`](crates/runtime/README.md) - ランタイム固有のドキュメント
- [`crates/runtime/API_REFERENCE.md`](crates/runtime/API_REFERENCE.md) - 完全な API リファレンス
- [`crates/dsl/README.md`](crates/dsl/README.md) - DSL 実装詳細

## 🤝 貢献

貢献を歓迎します！ガイドラインについては [`docs/contributing.md`](docs/contributing.md) を参照してください。

**開発原則:**
- セキュリティファースト - 全機能はセキュリティレビューを通過する必要があります
- ゼロトラスト - 全入力は潜在的に悪意があるものと仮定
- 包括的テスト - 高いテストカバレッジの維持
- 明確なドキュメント - 全機能と API のドキュメント化

## 🎯 使用例

### 開発と自動化
- セキュアなコード生成とリファクタリング
- ポリシーコンプライアンス付き自動テスト
- ツール検証付き AI エージェントデプロイ
- セマンティック検索による知識管理

### エンタープライズと規制産業
- HIPAA コンプライアンス付きヘルスケアデータ処理 **（Enterprise）**
- 監査要件付き金融サービス **（Enterprise）**
- セキュリティクリアランス付き政府系システム **（Enterprise）**
- 機密性付き法的文書分析 **（Enterprise）**

## 📄 ライセンス

**Community エディション**: MIT ライセンス  
**Enterprise エディション**: 商用ライセンスが必要

Enterprise ライセンスについては [ThirdKey](https://thirdkey.ai) にお問い合わせください。

## 🔗 リンク

- [ThirdKey ウェブサイト](https://thirdkey.ai)
- [ランタイム API リファレンス](crates/runtime/API_REFERENCE.md)

---

*Symbi は、インテリジェントなポリシー適用、暗号化検証、包括的な監査証跡を通じて、AI エージェントと人間の安全な協働を可能にします。*

<div align="right">
  <img src="symbi-trans.png" alt="Symbi 透明ロゴ" width="120">
</div>
# Dockerコンテナガイド

## 他の言語


## 利用可能なイメージ

### 統合Symbiコンテナ
- **イメージ**：`ghcr.io/thirdkeyai/symbi:latest`
- **用途**：DSLパーシング、エージェントランタイム、MCPサーバーを含むオールインワンコンテナ
- **サイズ**：約80MB（ベクトルDBとHTTP APIサポートを含む）
- **CLI**：異なる操作のためのサブコマンドを備えた統合 `symbi` コマンド

## クイックスタート

### プロジェクトをスキャフォールドして実行（推奨）

`symbi init` はコンテナ内で動作し、すぐに実行可能な `docker-compose.yml` と、新しく生成された `SYMBIONT_MASTER_KEY` を含む `.env` を含むプロジェクトをホストディレクトリに書き込みます：

```bash
# 1. ホストにプロジェクトファイルを作成
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. ランタイムを起動（.env を自動的に読み込む）
docker compose up
```

`--dir /workspace` フラグは、イメージの WORKDIR ではなくマウントされたボリュームに書き込むよう `symbi init` に指示します。この実行後、カレントディレクトリに `symbiont.toml`、`agents/`、`policies/`、`.symbiont/audit/`、`AGENTS.md`、`docker-compose.yml`、`.env`、および `.env.example` が作成されます。

コンポーズファイルの生成をスキップするには：

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile minimal --no-interact --no-docker-compose --dir /workspace
```

### ビルド済みイメージの使用（アドホック）

```bash
# 最新イメージをプル
docker pull ghcr.io/thirdkeyai/symbi:latest

# DSLファイルをパース
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl --file /workspace/agent.dsl

# MCPサーバーを実行（stdioベース、ポート不要）
docker run --rm -i \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp

# プロジェクトなしでランタイムを実行（エフェメラル、マスターキーなし）
docker run --rm -p 8080:8080 -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0
```

### 開発ワークフロー

```bash
# インタラクティブ開発
docker run --rm -it -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest bash

# ボリュームマウントとポート付きの開発
docker run --rm -it \
  -v $(pwd):/workspace \
  -p 8080:8080 \
  -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest bash
```

## 利用可能なタグ

- `latest` - 最新の安定リリース
- `main` - 最新の開発ビルド
- `v1.0.0` - 特定バージョンのリリース
- `sha-<commit>` - 特定コミットのビルド

## ローカルビルド

### 統合Symbiコンテナ

```bash
# プロジェクトルートから
docker build -t symbi:latest .

# ビルドをテスト
docker run --rm symbi:latest --version

# DSLパーシングをテスト
docker run --rm -v $(pwd):/workspace symbi:latest dsl --help

# MCPサーバーをテスト
docker run --rm symbi:latest mcp
```

## マルチアーキテクチャサポート

イメージは以下のアーキテクチャ向けにビルドされます：
- `linux/amd64`（x86_64）
- `linux/arm64`（ARM64/Apple Silicon）

Dockerはプラットフォームに応じて正しいアーキテクチャを自動的にプルします。

## セキュリティ機能

### 非rootユーザー実行
- コンテナは非rootユーザー `symbi`（UID 1000）として実行
- セキュリティ強化されたベースイメージによる最小限の攻撃対象

### 脆弱性スキャン
- すべてのイメージはTrivyで自動スキャン
- セキュリティアドバイザリーはGitHub Securityタブに公開
- 詳細な脆弱性分析のためのSARIFレポート

## 設定

### 環境変数

**Symbiコンテナ：**
- `SYMBIONT_MASTER_KEY` - **永続状態には必須。** ローカルストアの暗号化に使用される 32 バイトの 16 進数キー。`openssl rand -hex 32` で生成します。`symbi init` は自動的に `.env` に書き込みます。
- `RUST_LOG` - ログレベルの設定（debug、info、warn、error）
- `SYMBIONT_VECTOR_BACKEND` - ベクトルバックエンド：`lancedb`（デフォルト）または `qdrant`
- `QDRANT_URL` - QdrantベクトルデータベースURL（オプションのQdrantバックエンド使用時のみ）
- `OPENROUTER_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` - オプションの LLM 資格情報。いずれか一つで Coordinator Chat エンドポイントが有効になります。

### ボリュームマウント

イメージは `symbi` ユーザー（UID 1000）として `WORKDIR=/var/lib/symbi` で実行されます。プロジェクトファイルはそのディレクトリに読み取り専用でマウントされます。永続状態（ローカル SQLite ストアと監査ログ）は、コンテナの再起動後も残るよう名前付きボリュームに格納されます。

```bash
# プロジェクトファイル（読み取り専用）
-v $(pwd)/symbiont.toml:/var/lib/symbi/symbiont.toml:ro
-v $(pwd)/agents:/var/lib/symbi/agents:ro
-v $(pwd)/policies:/var/lib/symbi/policies:ro
-v $(pwd)/tools:/var/lib/symbi/tools:ro

# 永続状態
-v symbi-data:/var/lib/symbi/.symbi
-v symbi-audit:/var/lib/symbi/.symbiont
```

## Docker Composeの例

`symbi init` は、このセクションの他の部分と一致するすぐに実行可能な `docker-compose.yml` を生成します — コンポーズファイルを手書きするよりもこれを優先してください。参考のため、または `init` なしで始める場合：

デフォルトでは、Symbiontは**LanceDB**を組み込みベクトルデータベースとして使用します -- 外部サービスは不要です。スケールされたデプロイメント向けに分散ベクトルバックエンドが必要な場合は、オプションでQdrantを追加できます。

### 最小構成（LanceDBデフォルト -- Qdrant不要）

これを `SYMBIONT_MASTER_KEY` を設定する `.env` ファイルと組み合わせてください：

```yaml
services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    command: ["up", "--http-bind", "0.0.0.0"]
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./symbiont.toml:/var/lib/symbi/symbiont.toml:ro
      - ./agents:/var/lib/symbi/agents:ro
      - ./policies:/var/lib/symbi/policies:ro
      - ./tools:/var/lib/symbi/tools:ro
      - symbi-data:/var/lib/symbi/.symbi
      - symbi-audit:/var/lib/symbi/.symbiont
    environment:
      SYMBIONT_MASTER_KEY: ${SYMBIONT_MASTER_KEY:?set SYMBIONT_MASTER_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
    restart: unless-stopped

volumes:
  symbi-data:
  symbi-audit:
```

### オプションのQdrantバックエンド付き

```yaml
services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    command: ["up", "--http-bind", "0.0.0.0"]
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./symbiont.toml:/var/lib/symbi/symbiont.toml:ro
      - ./agents:/var/lib/symbi/agents:ro
      - ./policies:/var/lib/symbi/policies:ro
      - symbi-data:/var/lib/symbi/.symbi
      - symbi-audit:/var/lib/symbi/.symbiont
    environment:
      SYMBIONT_MASTER_KEY: ${SYMBIONT_MASTER_KEY:?set SYMBIONT_MASTER_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
      SYMBIONT_VECTOR_BACKEND: qdrant
      QDRANT_URL: http://qdrant:6334
    depends_on:
      - qdrant
    restart: unless-stopped

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbi-data:
  symbi-audit:
  qdrant-data:
```

## トラブルシューティング

### よくある問題

**権限拒否：**
```bash
# 正しいオーナーシップを確認
sudo chown -R 1000:1000 ./data

# または別のユーザーを使用
docker run --user $(id -u):$(id -g) ...
```

**ポートの競合：**
```bash
# 別のポートを使用
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbi:latest
```

**ビルドの失敗：**
```bash
# Dockerキャッシュをクリア
docker builder prune -a

# キャッシュなしで再ビルド
docker build --no-cache -f runtime/Dockerfile .
```

### ヘルスチェック

```bash
# コンテナのヘルスを確認
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest up --http-bind 0.0.0.0:8080
docker exec symbi-test /usr/local/bin/symbi --version
docker rm -f symbi-test
```

## パフォーマンス最適化

### リソース制限

```bash
# メモリとCPU制限を設定
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbi:latest mcp
```

### ビルド最適化

```bash
# BuildKitを使用してビルドを高速化
DOCKER_BUILDKIT=1 docker build .

# マルチステージキャッシング
docker build --target builder -t symbi-builder .
docker build --cache-from symbi-builder .
```

## CI/CDインテグレーション

GitHub Actionsは以下のタイミングで自動的にコンテナをビルドおよび公開します：
- `main` ブランチへのプッシュ
- 新しいバージョンタグ（`v*`）
- プルリクエスト（ビルドのみ）

イメージにはメタデータが含まれます：
- GitコミットSHA
- ビルドタイムスタンプ
- 脆弱性スキャン結果
- SBOM（ソフトウェア部品表）

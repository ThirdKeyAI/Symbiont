---
layout: default
title: Dockerガイド
description: "Symbiontを実行するためのDockerコンテナガイド"
nav_exclude: true
---

# Dockerコンテナガイド

## 他の言語
{: .no_toc}

[English](docker.md) | [中文简体](docker.zh-cn.md) | [Español](docker.es.md) | [Português](docker.pt.md) | **日本語** | [Deutsch](docker.de.md)

---

Symbiはすべての機能を含む統合Dockerコンテナを提供しており、GitHub Container Registryから利用できます。

## 利用可能なイメージ

### 統合Symbiコンテナ
- **イメージ**：`ghcr.io/thirdkeyai/symbi:latest`
- **用途**：DSLパーシング、エージェントランタイム、MCPサーバーを含むオールインワンコンテナ
- **サイズ**：約80MB（ベクトルDBとHTTP APIサポートを含む）
- **CLI**：異なる操作のためのサブコマンドを備えた統合 `symbi` コマンド

## クイックスタート

### ビルド済みイメージの使用

```bash
# 最新イメージをプル
docker pull ghcr.io/thirdkeyai/symbi:latest

# DSLファイルをパース
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl parse /workspace/agent.dsl

# MCPサーバーを実行
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp --port 8080

# HTTP API付きで実行
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp --http-api --port 8080
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
  -p 3000:3000 \
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
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help

# MCPサーバーをテスト
docker run --rm symbi:latest mcp --help
```

## マルチアーキテクチャサポート

イメージは以下のアーキテクチャ向けにビルドされます：
- `linux/amd64`（x86_64）
- `linux/arm64`（ARM64/Apple Silicon）

Dockerはプラットフォームに応じて正しいアーキテクチャを自動的にプルします。

## セキュリティ機能

### 非rootユーザー実行
- コンテナは非rootユーザー `symbiont`（UID 1000）として実行
- セキュリティ強化されたベースイメージによる最小限の攻撃対象

### 脆弱性スキャン
- すべてのイメージはTrivyで自動スキャン
- セキュリティアドバイザリーはGitHub Securityタブに公開
- 詳細な脆弱性分析のためのSARIFレポート

## 設定

### 環境変数

**Symbiコンテナ：**
- `SYMBI_LOG_LEVEL` - ログレベルの設定（debug、info、warn、error）
- `SYMBI_HTTP_PORT` - HTTP APIポート（デフォルト：8080）
- `SYMBI_MCP_PORT` - MCPサーバーポート（デフォルト：3000）
- `SYMBIONT_VECTOR_BACKEND` - ベクトルバックエンド：`lancedb`（デフォルト）または `qdrant`
- `QDRANT_URL` - QdrantベクトルデータベースURL（オプションのQdrantバックエンド使用時のみ）
- `DSL_OUTPUT_FORMAT` - DSL出力フォーマット（json、yaml、text）

### ボリュームマウント

```bash
# エージェント定義をマウント
-v $(pwd)/agents:/var/lib/symbi/agents

# 設定をマウント
-v $(pwd)/config:/etc/symbi

# データディレクトリをマウント
-v symbi-data:/var/lib/symbi/data
```

## Docker Composeの例

デフォルトでは、Symbiontは**LanceDB**を組み込みベクトルデータベースとして使用します -- 外部サービスは不要です。スケールされたデプロイメント向けに分散ベクトルバックエンドが必要な場合は、オプションでQdrantを追加できます。

### 最小構成（LanceDBデフォルト -- Qdrant不要）

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbi/agents
      - ./config:/etc/symbi
      - symbi-data:/var/lib/symbi/data
    environment:
      - SYMBI_LOG_LEVEL=info
    command: ["mcp", "--http-api", "--port", "8080"]

volumes:
  symbi-data:
```

### オプションのQdrantバックエンド付き

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbi/agents
      - ./config:/etc/symbi
      - symbi-data:/var/lib/symbi/data
    environment:
      - SYMBI_LOG_LEVEL=info
      - SYMBIONT_VECTOR_BACKEND=qdrant
      - QDRANT_URL=http://qdrant:6334
    depends_on:
      - qdrant
    command: ["mcp", "--http-api", "--port", "8080"]

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbi-data:
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
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest mcp --port 8080
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

---
layout: default
title: Docker 指南
nav_exclude: true
description: "运行 Symbiont 的 Docker 容器指南"
---

# Docker 容器指南

## 其他语言
{: .no_toc}

[English](docker.md) | **中文简体** | [Español](docker.es.md) | [Português](docker.pt.md) | [日本語](docker.ja.md) | [Deutsch](docker.de.md)

---

Symbi 提供了一个统一的 Docker 容器，包含所有功能，可通过 GitHub Container Registry 获取。

## 可用镜像

### 统一 Symbi 容器
- **镜像**：`ghcr.io/thirdkeyai/symbi:latest`
- **用途**：包含 DSL 解析、智能体运行时和 MCP 服务器的一体化容器
- **大小**：约 80MB（包含向量数据库和 HTTP API 支持）
- **CLI**：统一的 `symbi` 命令，带有不同操作的子命令

## 快速开始

### 使用预构建镜像

```bash
# Pull latest image
docker pull ghcr.io/thirdkeyai/symbi:latest

# Parse a DSL file
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl --file /workspace/agent.dsl

# Run MCP server (stdio-based, no port needed)
docker run --rm -i \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp

# Run with HTTP API
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0:8080
```

### 开发工作流

```bash
# Interactive development
docker run --rm -it -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest bash

# Development with volume mounts and ports
docker run --rm -it \
  -v $(pwd):/workspace \
  -p 8080:8080 \
  -p 3000:3000 \
  ghcr.io/thirdkeyai/symbi:latest bash
```

## 可用标签

- `latest` - 最新稳定版本
- `main` - 最新开发构建
- `v1.0.0` - 特定版本发布
- `sha-<commit>` - 特定提交构建

## 本地构建

### 统一 Symbi 容器

```bash
# From project root
docker build -t symbi:latest .

# Test the build
docker run --rm symbi:latest --version

# Test DSL parsing
docker run --rm -v $(pwd):/workspace symbi:latest dsl --help

# Test MCP server
docker run --rm symbi:latest mcp
```

## 多架构支持

镜像构建支持以下架构：
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

Docker 会自动为您的平台拉取正确的架构。

## 安全特性

### 非 Root 执行
- 容器以非 root 用户 `symbi`（UID 1000）运行
- 使用安全加固的基础镜像，攻击面最小化

### 漏洞扫描
- 所有镜像自动使用 Trivy 扫描
- 安全公告发布到 GitHub Security 标签页
- 提供 SARIF 报告用于详细的漏洞分析

## 配置

### 环境变量

**Symbi 容器：**
- `RUST_LOG` - 设置日志级别（debug、info、warn、error）
- `SYMBIONT_VECTOR_BACKEND` - 向量后端：`lancedb`（默认）或 `qdrant`
- `QDRANT_URL` - Qdrant 向量数据库 URL（仅在使用可选的 Qdrant 后端时需要）

### 卷挂载

```bash
# Mount agent definitions
-v $(pwd)/agents:/var/lib/symbi/agents

# Mount configuration
-v $(pwd)/config:/etc/symbi

# Mount data directory
-v symbi-data:/var/lib/symbi/data
```

## Docker Compose 示例

默认情况下，Symbiont 使用 **LanceDB** 作为嵌入式向量数据库——不需要外部服务。如果您需要分布式向量后端用于规模化部署，可以选择添加 Qdrant。

### 最小配置（LanceDB 默认——不需要 Qdrant）

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
      - RUST_LOG=info
    command: ["up", "--http-bind", "0.0.0.0:8080"]

volumes:
  symbi-data:
```

### 带可选 Qdrant 后端

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
      - RUST_LOG=info
      - SYMBIONT_VECTOR_BACKEND=qdrant
      - QDRANT_URL=http://qdrant:6334
    depends_on:
      - qdrant
    command: ["up", "--http-bind", "0.0.0.0:8080"]

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

## 故障排除

### 常见问题

**权限被拒绝：**
```bash
# Ensure correct ownership
sudo chown -R 1000:1000 ./data

# Or use different user
docker run --user $(id -u):$(id -g) ...
```

**端口冲突：**
```bash
# Use different ports
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbi:latest
```

**构建失败：**
```bash
# Clear Docker cache
docker builder prune -a

# Rebuild without cache
docker build --no-cache -f runtime/Dockerfile .
```

### 健康检查

```bash
# Check container health
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest up --http-bind 0.0.0.0:8080
docker exec symbi-test /usr/local/bin/symbi --version
docker rm -f symbi-test
```

## 性能优化

### 资源限制

```bash
# Set memory and CPU limits
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbi:latest mcp
```

### 构建优化

```bash
# Use BuildKit for faster builds
DOCKER_BUILDKIT=1 docker build .

# Multi-stage caching
docker build --target builder -t symbi-builder .
docker build --cache-from symbi-builder .
```

## CI/CD 集成

GitHub Actions 在以下情况自动构建和发布容器：
- 推送到 `main` 分支
- 新版本标签 (`v*`)
- Pull request（仅构建）

镜像包含以下元数据：
- Git 提交 SHA
- 构建时间戳
- 漏洞扫描结果
- SBOM（软件物料清单）

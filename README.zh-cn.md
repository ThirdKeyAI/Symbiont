<img src="logo-hz.png" alt="Symbi">

[English](README.md) | **中文简体** | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

**Symbi** 是一个AI原生智能体框架，用于构建能够与人类、其他智能体和大型语言模型安全协作的自主、策略感知智能体。社区版提供核心功能，企业功能提供高级安全、监控和协作。

## 🚀 快速开始

### 前提条件
- Docker（推荐）或 Rust 1.88+
- Qdrant 向量数据库（用于语义搜索）

### 使用预构建容器运行

**使用 GitHub Container Registry（推荐）：**

```bash
# 运行统一的 symbi CLI
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# 运行 MCP 服务器
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# 交互式开发
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### 从源码构建

```bash
# 构建开发环境
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# 构建统一的 symbi 二进制文件
cargo build --release

# 测试组件
cargo test

# 运行示例智能体（从 crates/runtime）
cd crates/runtime && cargo run --example basic_agent
cd crates/runtime && cargo run --example full_system
cd crates/runtime && cargo run --example rag_example

# 使用统一的 symbi CLI
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080

# 启用 HTTP API（可选）
cd crates/runtime && cargo run --features http-api --example full_system
```

### 可选的 HTTP API

启用用于外部集成的 RESTful HTTP API：

```bash
# 使用 HTTP API 功能构建
# 或添加到 Cargo.toml
[dependencies]
symbi-runtime = { version = "0.1.2", features = ["http-api"] }
```

**主要端点：**
- `GET /api/v1/health` - 健康检查和系统状态
- `GET /api/v1/agents` - 列出所有活跃代理
- `POST /api/v1/workflows/execute` - 执行工作流
- `GET /api/v1/metrics` - 系统指标

## 📁 项目结构

```
symbi/
├── src/                   # 统一的 symbi CLI 二进制文件
├── crates/                # 工作区 crates
│   ├── dsl/              # Symbi DSL 实现
│   │   ├── src/          # 解析器和库代码
│   │   ├── tests/        # DSL 测试套件
│   │   └── tree-sitter-symbiont/ # 语法定义
│   └── runtime/          # 代理运行时系统（社区版）
│       ├── src/          # 核心运行时组件
│       ├── examples/     # 使用示例
│       └── tests/        # 集成测试
├── docs/                 # 文档
└── Cargo.toml           # 工作区配置
```

## 🔧 功能特性

### ✅ 社区功能（开源）
- **DSL 语法**：用于代理定义的完整 Tree-sitter 语法
- **代理运行时**：任务调度、资源管理、生命周期控制
- **一级沙盒隔离**：基于 Docker 容器的代理操作隔离
- **MCP 集成**：用于外部工具的模型上下文协议客户端
- **SchemaPin 安全**：基础的密码学工具验证
- **RAG 引擎**：具有向量搜索的检索增强生成
- **上下文管理**：持久代理内存和知识存储
- **向量数据库**：用于语义搜索的 Qdrant 集成
- **全面的密钥管理**：HashiCorp Vault 集成，支持多种认证方法
- **加密文件后端**：AES-256-GCM 加密，集成操作系统密钥链
- **密钥 CLI 工具**：完整的加密/解密/编辑操作，具有审计追踪
- **HTTP API**：可选的 RESTful 接口（功能门控）

### 🏢 企业功能（需要许可证）
- **高级沙盒隔离**：gVisor 和 Firecracker 隔离 **（企业版）**
- **AI 工具审查**：自动化安全分析工作流 **（企业版）**
- **密码学审计**：使用 Ed25519 签名的完整审计追踪 **（企业版）**
- **多代理通信**：加密的代理间消息传递 **（企业版）**
- **实时监控**：SLA 指标和性能仪表板 **（企业版）**
- **专业服务和支持**：定制开发和支持 **（企业版）**

## 📐 Symbiont DSL

定义具有内置策略和能力的智能代理：

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

## 🔐 密钥管理

Symbi 提供企业级密钥管理，支持多种后端选项：

### 后端选项
- **HashiCorp Vault**：生产就绪的密钥管理，支持多种认证方法
  - 基于令牌的认证
  - Kubernetes 服务账户认证
- **加密文件**：本地 AES-256-GCM 加密存储，集成操作系统密钥链
- **代理命名空间**：按代理隔离的密钥访问作用域

### CLI 操作
```bash
# 加密密钥文件
symbi secrets encrypt config.json --output config.enc

# 解密密钥文件
symbi secrets decrypt config.enc --output config.json

# 直接编辑加密的密钥
symbi secrets edit config.enc

# 配置 Vault 后端
symbi secrets configure vault --endpoint https://vault.company.com
```

### 审计与合规
- 所有密钥操作的完整审计追踪
- 密码学完整性验证
- 按代理范围的访问控制
- 防篡改日志记录

## 🔒 安全模型

### 基础安全（社区版）
- **一级隔离**：基于 Docker 容器的代理执行
- **模式验证**：使用 SchemaPin 的密码学工具验证
- **策略引擎**：基础资源访问控制
- **密钥管理**：Vault 集成和加密文件存储
- **审计日志**：操作跟踪和合规性

### 高级安全（企业版）
- **增强沙盒隔离**：gVisor（二级）和 Firecracker（三级）隔离 **（企业版）**
- **AI 安全审查**：自动化工具分析和批准 **（企业版）**
- **加密通信**：安全的代理间消息传递 **（企业版）**
- **全面审计**：密码学完整性保证 **（企业版）**

## 🧪 测试

```bash
# 运行所有测试
cargo test

# 运行特定组件
cd crates/dsl && cargo test          # DSL 解析器
cd crates/runtime && cargo test     # 运行时系统

# 集成测试
cd crates/runtime && cargo test --test integration_tests
cd crates/runtime && cargo test --test rag_integration_tests
cd crates/runtime && cargo test --test mcp_client_tests
```

## 📚 文档

- **[快速开始](https://docs.symbiont.dev/getting-started)** - 安装和第一步
- **[DSL 指南](https://docs.symbiont.dev/dsl-guide)** - 完整的语言参考
- **[运行时架构](https://docs.symbiont.dev/runtime-architecture)** - 系统设计
- **[安全模型](https://docs.symbiont.dev/security-model)** - 安全实现
- **[API 参考](https://docs.symbiont.dev/api-reference)** - 完整的 API 文档
- **[贡献指南](https://docs.symbiont.dev/contributing)** - 开发指南

### 技术参考
- [`crates/runtime/README.md`](crates/runtime/README.md) - 运行时专属文档
- [`crates/runtime/API_REFERENCE.md`](crates/runtime/API_REFERENCE.md) - 完整的 API 参考
- [`crates/dsl/README.md`](crates/dsl/README.md) - DSL 实现详情

## 🤝 贡献

欢迎贡献！请查看 [`docs/contributing.md`](docs/contributing.md) 了解指导原则。

**开发原则：**
- 安全第一 - 所有功能必须通过安全审查
- 零信任 - 假设所有输入都有潜在恶意
- 全面测试 - 维持高测试覆盖率
- 清晰文档 - 记录所有功能和 API

## 🎯 使用场景

### 开发与自动化
- 安全代码生成和重构
- 符合策略的自动化测试
- 具有工具验证的 AI 代理部署
- 具有语义搜索的知识管理

### 企业与监管行业
- 符合 HIPAA 的医疗数据处理 **（企业版）**
- 具有审计要求的金融服务 **（企业版）**
- 具有安全许可的政府系统 **（企业版）**
- 具有保密性的法律文件分析 **（企业版）**

## 📄 许可证

**社区版**：MIT 许可证  
**企业版**：需要商业许可证

联系 [ThirdKey](https://thirdkey.ai) 获取企业版许可。

## 🔗 链接

- [ThirdKey 网站](https://thirdkey.ai)
- [运行时 API 参考](crates/runtime/API_REFERENCE.md)

---

*Symbi 通过智能策略执行、密码学验证和全面审计追踪，实现 AI 代理与人类之间的安全协作。*

<div align="right">
  <img src="symbi-trans.png" alt="Symbi 透明标志" width="120">
</div>
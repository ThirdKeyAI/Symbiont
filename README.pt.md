<img src="logo-hz.png" alt="Symbi">

**Symbi** é um framework de agentes nativo de IA para construir agentes autônomos e conscientes de políticas que podem colaborar com segurança com humanos, outros agentes e grandes modelos de linguagem. A edição Community fornece funcionalidade central com recursos Enterprise opcionais para segurança avançada, monitoramento e colaboração.

## 🚀 Início Rápido

### Pré-requisitos
- Docker (recomendado) ou Rust 1.88+
- Banco de dados vetorial Qdrant (para busca semântica)

### Executando com Contêineres Pré-construídos

**Usando GitHub Container Registry (Recomendado):**

```bash
# Executar CLI unificado do symbi
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# Executar MCP Server
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Desenvolvimento interativo
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### Construindo a partir do Código-fonte

```bash
# Construir ambiente de desenvolvimento
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# Construir o binário unificado do symbi
cargo build --release

# Testar os componentes
cargo test

# Executar agentes de exemplo (a partir de crates/runtime)
cd crates/runtime && cargo run --example basic_agent
cd crates/runtime && cargo run --example full_system
cd crates/runtime && cargo run --example rag_example

# Usar o CLI unificado do symbi
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080

# Habilitar HTTP API (opcional)
cd crates/runtime && cargo run --features http-api --example full_system
```

### API HTTP Opcional

Habilitar API HTTP RESTful para integração externa:

```bash
# Construir com recurso HTTP API
cargo build --features http-api

# Ou adicionar ao Cargo.toml
[dependencies]
symbi-runtime = { version = "0.1.2", features = ["http-api"] }
```

**Endpoints Principais:**
- `GET /api/v1/health` - Verificação de saúde e status do sistema
- `GET /api/v1/agents` - Listar todos os agentes ativos
- `POST /api/v1/workflows/execute` - Executar fluxos de trabalho
- `GET /api/v1/metrics` - Métricas do sistema

## 📁 Estrutura do Projeto

```
symbi/
├── src/                   # Binário CLI unificado do symbi
├── crates/                # Crates do workspace
│   ├── dsl/              # Implementação do DSL Symbi
│   │   ├── src/          # Código do parser e biblioteca
│   │   ├── tests/        # Suíte de testes do DSL
│   │   └── tree-sitter-symbiont/ # Definição da gramática
│   └── runtime/          # Sistema Runtime de Agentes (Community)
│       ├── src/          # Componentes centrais do runtime
│       ├── examples/     # Exemplos de uso
│       └── tests/        # Testes de integração
├── docs/                 # Documentação
└── Cargo.toml           # Configuração do workspace
```

## 🔧 Recursos

### ✅ Recursos Community (OSS)
- **Gramática DSL**: Gramática Tree-sitter completa para definições de agentes
- **Runtime de Agentes**: Agendamento de tarefas, gerenciamento de recursos, controle do ciclo de vida
- **Isolamento Tier 1**: Isolamento containerizado com Docker para operações de agentes
- **Integração MCP**: Cliente do Protocolo de Contexto de Modelo para ferramentas externas
- **Segurança SchemaPin**: Verificação criptográfica básica de ferramentas
- **Engine RAG**: Geração aumentada por recuperação com busca vetorial
- **Gerenciamento de Contexto**: Memória persistente de agentes e armazenamento de conhecimento
- **Banco de Dados Vetorial**: Integração com Qdrant para busca semântica
- **Gerenciamento Abrangente de Segredos**: Integração com HashiCorp Vault com múltiplos métodos de autenticação
- **Backend de Arquivos Criptografados**: Criptografia AES-256-GCM com integração de chaveiro do SO
- **Ferramentas CLI de Segredos**: Operações completas de criptografar/descriptografar/editar com trilhas de auditoria
- **API HTTP**: Interface RESTful opcional (controlada por recursos)

### 🏢 Recursos Enterprise (Licença Necessária)
- **Isolamento Avançado**: Isolamento gVisor e Firecracker **(Enterprise)**
- **Revisão de Ferramentas IA**: Fluxo de trabalho de análise de segurança automatizado **(Enterprise)**
- **Auditoria Criptográfica**: Trilhas de auditoria completas com assinaturas Ed25519 **(Enterprise)**
- **Comunicação Multi-Agente**: Mensageria criptografada entre agentes **(Enterprise)**
- **Monitoramento em Tempo Real**: Métricas SLA e dashboards de desempenho **(Enterprise)**
- **Serviços Profissionais e Suporte**: Desenvolvimento personalizado e suporte **(Enterprise)**

## 📐 DSL Symbiont

Defina agentes inteligentes com políticas e capacidades integradas:

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

## 🔐 Gerenciamento de Segredos

Symbi oferece gerenciamento de segredos de nível empresarial com múltiplas opções de backend:

### Opções de Backend
- **HashiCorp Vault**: Gerenciamento de segredos pronto para produção com múltiplos métodos de autenticação
  - Autenticação baseada em token
  - Autenticação de conta de serviço Kubernetes
- **Arquivos Criptografados**: Armazenamento local criptografado AES-256-GCM com integração de chaveiro do SO
- **Namespaces de Agentes**: Acesso a segredos com escopo por agente para isolamento

### Operações CLI
```bash
# Criptografar arquivo de segredos
symbi secrets encrypt config.json --output config.enc

# Descriptografar arquivo de segredos
symbi secrets decrypt config.enc --output config.json

# Editar segredos criptografados diretamente
symbi secrets edit config.enc

# Configurar backend Vault
symbi secrets configure vault --endpoint https://vault.company.com
```

### Auditoria e Conformidade
- Trilhas de auditoria completas para todas as operações de segredos
- Verificação de integridade criptográfica
- Controles de acesso com escopo por agente
- Logging à prova de adulteração

## 🔒 Modelo de Segurança

### Segurança Básica (Community)
- **Isolamento Tier 1**: Execução de agentes containerizada com Docker
- **Verificação de Esquemas**: Validação criptográfica de ferramentas com SchemaPin
- **Engine de Políticas**: Controle básico de acesso a recursos
- **Gerenciamento de Segredos**: Integração com Vault e armazenamento de arquivos criptografados
- **Logging de Auditoria**: Rastreamento de operações e conformidade

### Segurança Avançada (Enterprise)
- **Isolamento Aprimorado**: Isolamento gVisor (Tier2) e Firecracker (Tier3) **(Enterprise)**
- **Revisão de Segurança IA**: Análise automatizada de ferramentas e aprovação **(Enterprise)**
- **Comunicação Criptografada**: Mensageria segura entre agentes **(Enterprise)**
- **Auditorias Abrangentes**: Garantias de integridade criptográfica **(Enterprise)**

## 🧪 Testes

```bash
# Executar todos os testes
cargo test

# Executar componentes específicos
cd crates/dsl && cargo test          # Parser DSL
cd crates/runtime && cargo test     # Sistema de runtime

# Testes de integração
cd crates/runtime && cargo test --test integration_tests
cd crates/runtime && cargo test --test rag_integration_tests
cd crates/runtime && cargo test --test mcp_client_tests
```

## 📚 Documentação

- **[Primeiros Passos](https://docs.symbiont.dev/getting-started)** - Instalação e primeiros passos
- **[Guia do DSL](https://docs.symbiont.dev/dsl-guide)** - Referência completa da linguagem
- **[Arquitetura do Runtime](https://docs.symbiont.dev/runtime-architecture)** - Design do sistema
- **[Modelo de Segurança](https://docs.symbiont.dev/security-model)** - Implementação de segurança
- **[Referência da API](https://docs.symbiont.dev/api-reference)** - Documentação completa da API
- **[Contribuição](https://docs.symbiont.dev/contributing)** - Diretrizes de desenvolvimento

### Referências Técnicas
- [`crates/runtime/README.md`](crates/runtime/README.md) - Documentação específica do runtime
- [`crates/runtime/API_REFERENCE.md`](crates/runtime/API_REFERENCE.md) - Referência completa da API
- [`crates/dsl/README.md`](crates/dsl/README.md) - Detalhes de implementação do DSL

## 🤝 Contribuindo

Contribuições são bem-vindas! Por favor consulte [`docs/contributing.md`](docs/contributing.md) para diretrizes.

**Princípios de Desenvolvimento:**
- Segurança em primeiro lugar - todos os recursos devem passar por revisão de segurança
- Confiança zero - assumir que todas as entradas são potencialmente maliciosas
- Testes abrangentes - manter alta cobertura de testes
- Documentação clara - documentar todos os recursos e APIs

## 🎯 Casos de Uso

### Desenvolvimento e Automação
- Geração segura de código e refatoração
- Testes automatizados com conformidade de políticas
- Deploy de agentes IA com verificação de ferramentas
- Gerenciamento de conhecimento com busca semântica

### Empresas e Indústrias Regulamentadas
- Processamento de dados de saúde com conformidade HIPAA **(Enterprise)**
- Serviços financeiros com requisitos de auditoria **(Enterprise)**
- Sistemas governamentais com autorizações de segurança **(Enterprise)**
- Análise de documentos legais com confidencialidade **(Enterprise)**

## 📄 Licença

**Edição Community**: Licença MIT  
**Edição Enterprise**: Licença comercial necessária

Entre em contato com [ThirdKey](https://thirdkey.ai) para licenciamento Enterprise.

## 🔗 Links

- [Site da ThirdKey](https://thirdkey.ai)
- [Referência da API do Runtime](crates/runtime/API_REFERENCE.md)

---

*Symbi permite colaboração segura entre agentes IA e humanos através de aplicação inteligente de políticas, verificação criptográfica e trilhas de auditoria abrangentes.*

<div align="right">
  <img src="symbi-trans.png" alt="Logo Transparente Symbi" width="120">
</div>
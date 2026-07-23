# Começando

Este guia irá orientá-lo na configuração do Symbi e na criação do seu primeiro agente de IA.

▶ **Assista ao tutorial de introdução:**

[![Symbiont — get started](https://img.youtube.com/vi/RPyKpqKz5ik/hqdefault.jpg)](https://www.youtube.com/watch?v=RPyKpqKz5ik)

## Índice


---

## Pré-requisitos

O que você precisa depende de como instala e executa o Symbi.

### Para executar o binário pré-compilado

Os binários pré-compilados já estão compilados — você **não** precisa de Rust, protobuf ou Git para instalá-los ou executá-los. Instale com o Homebrew, o script de instalação (`curl`) ou um download manual a partir do GitHub Releases.

- **Docker** só é necessário em *tempo de execução* se você executar agentes sob o sandbox tier padrão (`tier1`, baseado em Docker). **Não** é necessário para instalar o Symbi ou para executar `symbi init`, `symbi dsl` ou `symbi --version`.

### Para compilar a partir do código-fonte

Necessário apenas se você instalar via `cargo install` ou compilar o repositório você mesmo:

- **Rust 1.82+**
- **protobuf-compiler** (`apt install protobuf-compiler` no Ubuntu, `brew install protobuf` no macOS)
- **Git** (para clonar o repositório)

### Opcional

- **[symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code)** (plugin de governança do Claude Code)
- **[symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli)** (extensão de governança do Gemini CLI)

> **Nota:** A busca vetorial é integrada. O Symbi inclui o [LanceDB](https://lancedb.com/) como banco de dados vetorial embutido -- nenhum serviço externo é necessário.

---

## Instalação

### Opção 1: Docker (Recomendado)

A maneira mais rápida de obter um runtime funcional é deixar o container fazer o scaffolding do projeto para você:

```bash
# 1. Gera symbiont.toml, agents/, policies/, docker-compose.yml e
#    um .env com SYMBIONT_MASTER_KEY recém-gerada.
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Inicia o runtime. Lê o .env automaticamente.
docker compose up
```

A API do runtime ficará em `http://localhost:8080` e a HTTP Input em `http://localhost:8081`.

Se você prefere trabalhar a partir de um clone (para construir a imagem ou rodar testes):

```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Construir o container unificado symbi
docker build -t symbi:latest .

# Executar o ambiente de desenvolvimento
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Opção 2: Instalação Local

Para desenvolvimento local:

```bash
# Clonar o repositório
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Instalar dependências do Rust e compilar
cargo build --release

# Executar testes para verificar a instalação
cargo test
```

### Verificar Instalação

Testar se tudo está funcionando corretamente:

```bash
# Testar o analisador DSL
cd crates/dsl && cargo run && cargo test

# Testar o sistema de runtime
cd ../runtime && cargo test

# Executar agentes de exemplo
cargo run --example basic_agent
cargo run --example full_system

# Testar o CLI unificado symbi
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Testar com container Docker
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## Inicialização de Projeto

A maneira mais rápida de iniciar um novo projeto Symbiont é `symbi init`:

```bash
symbi init
```

Isso inicia um assistente interativo que o guia por:
- **Seleção de perfil**: `minimal`, `assistant`, `dev-agent` ou `multi-agent`
- **Modo SchemaPin**: `tofu` (Trust-On-First-Use), `strict` ou `disabled`
- **Camada de sandbox**: `tier0` (nenhuma, somente desenvolvimento), `tier1` (Docker), `tier2` (gVisor / `runsc`) ou `tier3` (microVM Firecracker)

### O que `init` produz

Toda execução escreve:

| Arquivo | Propósito |
|---------|-----------|
| `symbiont.toml` | Configuração de runtime e políticas |
| `policies/default.cedar` | Política Cedar deny-by-default |
| `agents/*.symbi` | Definições de agentes específicas do perfil (a extensão legada `.dsl` também é reconhecida; exceto `minimal`) |
| `AGENTS.md` | Índice gerado automaticamente dos agentes declarados |
| `.symbiont/audit/` | Diretório do log de auditoria à prova de adulteração |
| `.gitignore` | Acrescido com entradas específicas do Symbiont, incluindo `.env` |
| `.env` | `SYMBIONT_MASTER_KEY` gerada a partir de `/dev/urandom` (permissões 0600) |
| `.env.example` | Template seguro para commit mostrando as variáveis de ambiente necessárias |
| `docker-compose.yml` | Arquivo compose pronto para execução com montagens de volume e fiação de env corretas |

Passe `--no-docker-compose` para pular o arquivo compose, e `--dir <PATH>` para escrever em um diretório diferente do atual (essencial dentro de um container Docker — veja abaixo).

### Modo não interativo

Para CI/CD ou configurações por script:

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

### Executando `init` dentro do Docker

Como o WORKDIR da imagem é `/var/lib/symbi`, use `--dir` para escrever no volume montado:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace
```

Isso popula o diretório atual do host com a árvore completa do projeto.

### Perfis

| Perfil | O que cria |
|--------|-----------|
| `minimal` | `symbiont.toml` + política Cedar padrão |
| `assistant` | + agente assistente governado individual |
| `dev-agent` | + agente CliExecutor com políticas de segurança |
| `multi-agent` | + agentes coordenador/worker com políticas inter-agente |

### Importando do catálogo

Importe agentes pré-construídos junto com qualquer perfil:

```bash
symbi init --profile minimal --no-interact
symbi init --catalog assistant,dev
```

Listar agentes disponíveis no catálogo:

```bash
symbi init --catalog list
```

Após a inicialização, valide e inicie:

```bash
symbi dsl -f agents/assistant.symbi   # validar seu agente
symbi run assistant -i '{"query": "hello"}'  # testar um único agente
symbi up                             # iniciar o runtime localmente
docker compose up                    # ...ou iniciar no Docker (lê o .env)
```

### Executando um único agente

Use `symbi run` para executar um agente sem iniciar o servidor de runtime completo:

```bash
symbi run <nome-do-agente-ou-arquivo> --input <json>
```

O comando resolve nomes de agentes pesquisando: caminho direto, depois o diretório `agents/`. Ele configura a inferência em nuvem a partir de variáveis de ambiente (`OPENROUTER_API_KEY`, `OPENAI_API_KEY` ou `ANTHROPIC_API_KEY`), executa o loop de raciocínio ORGA e encerra.

```bash
symbi run assistant -i 'Summarize this document'
symbi run agents/recon.symbi -i '{"target": "10.0.1.5"}' --max-iterations 5
```

### Partindo de um template (`symbi new`)

`symbi init` gera um projeto genérico; `symbi new` gera um projeto em torno de um dos vários templates orientados por tarefa. Útil quando você sabe que tipo de agente precisa antes de saber exatamente quais agentes precisa.

```bash
symbi new --list                     # mostra os templates disponíveis
symbi new <template> <project-name>  # cria um novo projeto a partir de um template
```

Templates incluídos:

| Template | O que você obtém |
|----------|------------------|
| `webhook-min` | Agente mínimo acionado por webhook — configuração de HTTP Input + uma DSL de handler |
| `webscraper-agent` | Agente de scraping com políticas de acesso Cedar e uma ferramenta de scraping ToolClad |
| `slm-first` | Padrão de roteador + allow-list SLM + fallback por confiança |
| `rag-lite` | Scripts de ingestão baseados em Qdrant mais um agente de busca |

`symbi new` e `symbi init` são complementares: `new` fornece um ponto de partida específico para a tarefa, `init` (+ `--catalog`) fornece um ponto de partida específico para governança. Você também pode combiná-los — gere o scaffold com `new` e, em seguida, `symbi init --catalog ...` para incorporar agentes pré-construídos adicionais do catálogo.

---

## Seu Primeiro Agente

Vamos criar um agente simples de análise de dados para entender os conceitos básicos do Symbi.

### 1. Criar Definição do Agente

Crie um novo arquivo `my_agent.symbi`:

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

### 2. Executar o Agente

```bash
# Analisar e validar a definição do agente
cargo run -- dsl parse my_agent.symbi

# Executar o agente no runtime
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.symbi
```

---

## Entendendo o DSL

O DSL do Symbi tem vários componentes principais:

### Bloco de Metadados

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

Fornece informações essenciais sobre o seu agente para documentação e gerenciamento do runtime.

### Definição do Agente

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // implementação do agente
}
```

Define a interface, capacidades e comportamento do agente.

### Definições de Política

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Políticas de segurança declarativas que são aplicadas em tempo de execução.

### Contexto de Execução

```rust
with memory = "persistent", privacy = "high" {
    // implementação do agente
}
```

Especifica a configuração de runtime para gerenciamento de memória e requisitos de privacidade.

---

## Próximos Passos

### Explorar Exemplos

O repositório inclui vários agentes de exemplo:

```bash
# Exemplo de agente básico
cd crates/runtime && cargo run --example basic_agent

# Demonstração completa do sistema
cd crates/runtime && cargo run --example full_system

# Exemplo de contexto e memória
cd crates/runtime && cargo run --example context_example

# Agente potenciado por RAG
cd crates/runtime && cargo run --example rag_example
```

### Habilitar Recursos Avançados

#### API HTTP (Opcional)

```bash
# Habilitar o recurso de API HTTP
cd crates/runtime && cargo build --features http-api

# Executar com endpoints de API
cd crates/runtime && cargo run --features http-api --example full_system
```

**Principais Endpoints da API:**
- `GET /api/v1/health` - Verificação de saúde e status do sistema
- `GET /api/v1/agents` - Listar todos os agentes ativos com status de execução em tempo real
- `GET /api/v1/agents/{id}/status` - Obter métricas detalhadas de execução do agente
- `POST /api/v1/workflows/execute` - Executar fluxos de trabalho

**Novos Recursos de Gerenciamento de Agentes:**
- Monitoramento de processos em tempo real e verificações de saúde
- Capacidades de desligamento gracioso para agentes em execução
- Métricas de execução abrangentes e rastreamento de uso de recursos
- Suporte para diferentes modos de execução (efêmero, persistente, agendado, orientado a eventos)

#### Inferência LLM em Nuvem

Conecte a provedores de LLM em nuvem via OpenRouter:

```bash
# Habilitar inferência em nuvem
cargo build --features cloud-llm

# Definir chave de API e modelo
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # opcional
```

#### Modo Agente Autônomo

Linha única para agentes cloud-native com inferência LLM:

```bash
cargo build --features standalone-agent
# Habilita: cloud-llm
```

> **Note:** Composio MCP and SymbiBot integration were removed in this version due to security concerns — see SECURITY_AUDIT.md C3 for context.

#### Primitivas de Raciocínio Avançado

Habilite curadoria de ferramentas, detecção de loops travados, pré-busca de contexto e convenções com escopo:

```bash
cargo build --features orga-adaptive
```

Veja o [guia orga-adaptive](/orga-adaptive) para documentação completa.

#### Motor de Políticas Cedar

Autorização formal com a linguagem de políticas Cedar. **Habilitado por padrão desde v1.14.x**: os binários `symbi` publicados (crates.io, Docker, tarballs do GitHub Release) incluem o Cedar, e `symbi up` / `symbi run` auto-conectam o `CedarPolicyGate` a partir de arquivos `policies/*.cedar` no startup; se nenhum estiver presente, o runtime recorre ao `DefaultPolicyGate` fail-closed. Para compilar sem o Cedar (por exemplo, quando você pretende conectar o `OpaPolicyGateBridge` ou um `ReasoningPolicyGate` personalizado), use:

```bash
cargo build --no-default-features --features "keychain,vector-lancedb"  # remove cedar
```

#### Banco de Dados Vetorial (Integrado)

O Symbi inclui o LanceDB como banco de dados vetorial embutido sem configuração. A busca semântica e o RAG funcionam imediatamente -- nenhum serviço separado para iniciar:

```bash
# Executar agente com capacidades RAG (a busca vetorial funciona automaticamente)
cd crates/runtime && cargo run --example rag_example

# Testar gerenciamento de contexto com busca avançada
cd crates/runtime && cargo run --example context_example
```

> **Build mínimo:** O LanceDB é incluído por padrão, mas pode ser excluído para binários mais leves: `cargo build --no-default-features`. O runtime recorre de forma transparente a um backend vetorial no-op.
>
> **Implantações em escala:** O Qdrant está disponível como backend opcional. Compile com `--features vector-qdrant` e defina `SYMBIONT_VECTOR_BACKEND=qdrant`.

**Recursos de Gerenciamento de Contexto:**
- **Busca Multi-Modal**: Modos de busca por palavra-chave, temporal, similaridade e híbrido
- **Cálculo de Importância**: Algoritmo de pontuação sofisticado considerando padrões de acesso, recência e feedback do usuário
- **Controle de Acesso**: Integração com motor de políticas e controles de acesso com escopo de agente
- **Arquivamento Automático**: Políticas de retenção com armazenamento comprimido e limpeza
- **Compartilhamento de Conhecimento**: Compartilhamento seguro de conhecimento entre agentes com pontuações de confiança

#### Referência de Feature Flags

| Feature | Descrição | Padrão |
|---------|-----------|--------|
| `keychain` | Integração com chaveiro do SO para segredos | Sim |
| `vector-lancedb` | Backend vetorial embutido LanceDB | Sim |
| `vector-qdrant` | Backend vetorial distribuído Qdrant | Não |
| `embedding-models` | Modelos de embedding locais via Candle | Não |
| `http-api` | API REST com Swagger UI | Não |
| `http-input` | Servidor de webhook com autenticação JWT | Não |
| `cloud-llm` | Inferência LLM em nuvem (OpenRouter) | Não |
| `standalone-agent` | Meta-feature Cloud LLM | Não |
| `cedar` | Motor de políticas Cedar — auto-conecta a partir de `policies/*.cedar` no startup | **Sim** |
| `orga-adaptive` | Primitivas de raciocínio avançado | Não |
| `cron` | Agendamento cron persistente | Não |
| `cli-executor` | Subprocessos de CLI de IA governados (Claude Code etc.) — Modo B | **Sim** |
| `native-sandbox` | Sandboxing nativo de processos | Não |
| `metrics` | Métricas/rastreamento OpenTelemetry | Não |
| `mcp-client` | Execução de ferramentas ToolClad baseada em MCP via stdio (verificado via SchemaPin) | No |
| `toolclad-browser` | Backend ToolClad de navegador (CDP) — apenas esqueleto de implementação; retorna um erro explícito até que o backend CDP esteja disponível | No |
| `interactive` | Prompts interativos para `symbi init` (dialoguer) | Sim |
| `full` | Todos os recursos opcionais de runtime, vetor e política | Não |

```bash
# Compilar com features específicas
cargo build --features "cloud-llm,orga-adaptive,cedar"

# Compilar com tudo
cargo build --features full
```

---

## Plugins de Assistente de IA

O Symbiont fornece plugins de governança de primeira mão para assistentes de codificação de IA populares, com três camadas progressivas de proteção:

1. **Awareness** (padrão) — registro consultivo de todas as chamadas de ferramenta que modificam estado
2. **Protection** — um hook de bloqueio aplica uma deny list local (`.symbiont/local-policy.toml`)
3. **Governance** — avaliação de políticas Cedar quando o `symbi` está no PATH

A configuração da deny list é agnóstica em relação à ferramenta — o mesmo `.symbiont/local-policy.toml` funciona com ambos os plugins:

```toml
[deny]
paths = [".env", ".ssh/", ".aws/"]
commands = ["rm -rf", "git push --force"]
branches = ["main", "master", "production"]
```

### Claude Code

```bash
# Instalar a partir do marketplace
/plugin marketplace add https://github.com/thirdkeyai/symbi-claude-code

# Skills disponíveis: /symbi-init, /symbi-policy, /symbi-verify, /symbi-audit, /symbi-dsl
```

Veja [symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code) para detalhes.

#### Modo B: subprocesso Claude Code governado

Além dos hooks dentro do editor, o Symbiont pode executar o Claude Code como um
*subprocesso governado* — o caminho do "Modo B" (gerenciado por ORGA). Um agente cujos
metadados declaram `executor = "claude_code"` é executado ao gerar o Claude Code sob o
`CliExecutor` do runtime, em vez do loop de raciocínio do LLM. O agente `code_reviewer`
incluído é o exemplo de referência:

```bash
# Revisar uma working tree com um subprocesso Claude Code governado
symbi run code_reviewer --target /path/to/repo

# Limites: --max-turns é o limite primário (cooperativo); --budget-timeout é um
# backstop rígido de wall-clock (SIGTERM gracioso -> SIGKILL).
symbi run code_reviewer --target . --max-turns 12 --budget-timeout 15m
```

A cada execução, o Symbiont:

- avalia o spawn através do **Gate** de políticas (fail-closed — permita-o via uma
  política Cedar, ou `SYMBI_INSECURE_ALLOW_ALL=1` para desenvolvimento local);
- define o handshake de env (`SYMBIONT_MANAGED=true`, `SYMBIONT_SESSION_ID`,
  `SYMBIONT_BUDGET_TOKENS`, `SYMBIONT_BUDGET_TIMEOUT`, `CLAUDE_PROJECT_DIR`) para que o
  plugin symbi-claude-code **delegue** seus hooks ao Gate externo;
- carrega o plugin via `--plugin-dir` e conecta o canal reverso stdio `symbi mcp`
  via `--mcp-config --strict-mcp-config`;
- executa o Claude Code em modo headless (`--print --output-format json --permission-mode dontAsk`).

| Variable / flag | Propósito | Padrão |
|---|---|---|
| `SYMBIONT_CLAUDE_PLUGIN_DIR` | Caminho para o plugin symbi-claude-code | autodetecta repositório irmão |
| `--plugin-dir` | Sobrescreve o caminho do plugin para uma execução | — |
| `--target` | Diretório de trabalho sobre o qual operar | diretório atual |
| `--max-turns` | Limite cooperativo primário (turnos agênticos) | 12 |
| `--budget-timeout` | Backstop de wall-clock, ex. `15m` / `900s` | 15m |
| `--budget-tokens` | Sugestão de orçamento de tokens passada ao subprocesso (awareness) | 100000 |

> **Auth:** o subprocesso usa a própria autenticação do Claude Code — uma sessão
> logada (`claude /login`) ou `ANTHROPIC_API_KEY`. A feature `cli-executor` está
> ativada por padrão.

### Gemini CLI

```bash
# Instalar a extensão
gemini extensions install https://github.com/thirdkeyai/symbi-gemini-cli
```

A extensão do Gemini CLI fornece defesa em profundidade adicional via bloqueio de manifesto `excludeTools` e aplicação nativa de `policies/*.toml` no nível da plataforma.

Veja [symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli) para detalhes.

---

## Configuração

### Variáveis de Ambiente

Configure seu ambiente para performance ideal:

```bash
# Obrigatório: chave hex de 32 bytes usada para criptografar o estado persistente.
# Gere com: openssl rand -hex 32
# `symbi init` escreve uma no .env automaticamente.
export SYMBIONT_MASTER_KEY="..."

# Configuração básica
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# A busca vetorial funciona automaticamente com o backend LanceDB integrado.
# Para usar o Qdrant em vez disso (opcional, habilite a feature `vector-qdrant`):
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# Integração MCP (opcional)
export MCP_SERVER_URLS="http://localhost:8080"
```

#### Variáveis de Ambiente Relacionadas à Segurança (após auditoria v1.13.0)

| Variável | Padrão | Efeito |
|---|---|---|
| `SYMBI_INSECURE_ALLOW_ALL` | não definida | Quando definida como `1`, `symbi up` / `symbi run` usam o policy gate permissivo (toda chamada de ferramenta e delegação é permitida). Equivalente à flag `--insecure-allow-all`. Um banner ruidoso é impresso em stderr. **Apenas para desenvolvimento local.** Sem isso, o loop de raciocínio é fail-closed e rejeita chamadas de ferramentas e delegações até que um backend de política explícito seja conectado. |
| `SYMBI_REJECT_LEGACY_API_KEYS` | não definida | Quando definida como `1`, o validador de chaves de API curto-circuita o escaneamento O(n) Argon2 depreciado para chaves sem prefixo. Use isso imediatamente após reemitir cada chave no formato `keyid.secret`. O caminho legado será removido na próxima release minor de qualquer forma. |
| `SYMBI_UNSAFE_NATIVE_SANDBOX` | não definida | Necessária (além de `SYMBI_ENV=production`-não-definida) para construir o runner de sandbox `native`. A feature Cargo `native-sandbox` também falha em compilar em builds de release. O runner nativo não oferece isolamento e destina-se apenas a depuração local. |
| `SYMBI_TRUSTED_PROXIES` | não definida | Allowlist CIDR para proxies reversos confiáveis; `X-Forwarded-For` é honrado apenas a partir desses endereços. |

As seguintes variáveis de ambiente foram **removidas**:

- `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` — o verificador JWT agora sempre exige `aud`. (Removida na auditoria pós-v1.13.0; era um escape hatch inseguro.)
- `COMPOSIO_API_KEY`, `COMPOSIO_MCP_URL` — a integração Composio MCP foi removida por completo. Veja `SECURITY_AUDIT.md` C3.

### Configuração de Runtime

Crie um arquivo de configuração `symbi.toml`:

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
backend = "lancedb"              # padrão; também suporta "qdrant"
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # necessário apenas quando backend = "qdrant"
```

---

## Problemas Comuns

### Problemas com Docker

**Problema**: Construção do Docker falha com erros de permissão
```bash
# Solução: Garantir que o daemon do Docker está rodando e o usuário tem permissões
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problema**: Container sai imediatamente
```bash
# Solução: Verificar logs do Docker
docker logs <container_id>
```

### Problemas de Construção com Rust

**Problema**: Construção do Cargo falha com erros de dependência
```bash
# Solução: Atualizar Rust e limpar cache de construção
rustup update
cargo clean
cargo build
```

**Problema**: Dependências do sistema em falta
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### Problemas de Runtime

**Problema**: Agente falha ao iniciar
```bash
# Verificar sintaxe da definição do agente
cargo run -- dsl parse your_agent.symbi

# Habilitar logging de debug
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Obtendo Ajuda

### Documentação

- **[Guia DSL](/dsl-guide)** - Referência completa do DSL
- **[Arquitetura de Runtime](/runtime-architecture)** - Detalhes da arquitetura do sistema
- **[Modelo de Segurança](/security-model)** - Documentação de segurança e políticas

### Suporte da Comunidade

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discussões**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Documentação**: [Referência Completa da API](https://docs.symbiont.dev/api-reference)

### Modo de Debug

Para solução de problemas, habilite logging detalhado:

```bash
# Habilitar logging de debug
export RUST_LOG=symbi=debug

# Executar com saída detalhada
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## O Que Vem a Seguir?

Agora que você tem o Symbi rodando, explore estes tópicos avançados:

1. **[Guia DSL](/dsl-guide)** - Aprenda recursos avançados do DSL
2. **[Guia do Loop de Raciocínio](/reasoning-loop)** - Entenda o ciclo ORGA
3. **[Raciocínio Avançado (orga-adaptive)](/orga-adaptive)** - Curadoria de ferramentas, detecção de loops travados, pré-hidratação
4. **[Arquitetura de Runtime](/runtime-architecture)** - Entenda os internos do sistema
5. **[Modelo de Segurança](/security-model)** - Implemente políticas de segurança
6. **[Contribuindo](/contributing)** - Contribua para o projeto

Pronto para construir algo incrível? Comece com nossos [projetos de exemplo](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) ou mergulhe na [especificação completa](/dsl-specification).

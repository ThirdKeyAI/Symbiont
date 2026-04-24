# Guia do REPL do Symbiont

## Outros idiomas


> **Procurando uma TUI interativa?** [`symbi shell`](/symbi-shell) (Beta) envolve o mesmo motor `repl_core` que este guia cobre, mais um orquestrador LLM, um catálogo completo de comandos (`/spawn`, `/run`, `/chain`, …) e attach remoto. Use o REPL quando quiser uma superfície JSON-RPC scriptável para integração com IDEs; use o shell quando quiser autoria conversacional contra o mesmo runtime.

## Funcionalidades

- **Avaliação Interativa de DSL**: Execute código DSL do Symbiont em tempo real
- **Gerenciamento de Ciclo de Vida de Agentes**: Criar, iniciar, parar, pausar, retomar e destruir agentes
- **Monitoramento de Execução**: Monitoramento em tempo real da execução de agentes com estatísticas e traces
- **Imposição de Políticas**: Verificação de políticas e controle de capacidades integrados
- **Gerenciamento de Sessão**: Snapshot e restauração de sessões do REPL
- **Protocolo JSON-RPC**: Acesso programático via JSON-RPC sobre stdio
- **Suporte LSP**: Language Server Protocol para integração com IDEs

## Primeiros Passos

### Iniciando o REPL

```bash
# Modo REPL interativo
symbi repl

# Modo servidor JSON-RPC sobre stdio (para integração com IDE)
symbi repl --stdio
```

> **Nota:** A flag `--config` ainda não é suportada. A configuração é lida do local padrão `symbiont.toml`. Suporte a configuração personalizada está planejado para uma versão futura.

### Uso Básico

```rust
# Definir um agente
agent GreetingAgent {
  name: "Greeting Agent"
  version: "1.0.0"
  description: "A simple greeting agent"
}

# Definir um comportamento
behavior Greet {
  input { name: string }
  output { greeting: string }
  steps {
    let greeting = format("Hello, {}!", name)
    return greeting
  }
}

# Executar expressões
let message = "Welcome to Symbiont"
print(message)
```

## Comandos do REPL

### Gerenciamento de Agentes

| Comando | Descrição |
|---------|-----------|
| `:agents` | Listar todos os agentes |
| `:agent list` | Listar todos os agentes |
| `:agent start <id>` | Iniciar um agente |
| `:agent stop <id>` | Parar um agente |
| `:agent pause <id>` | Pausar um agente |
| `:agent resume <id>` | Retomar um agente pausado |
| `:agent destroy <id>` | Destruir um agente |
| `:agent execute <id> <behavior> [args]` | Executar comportamento do agente |
| `:agent debug <id>` | Mostrar informações de depuração de um agente |

### Comandos de Monitoramento

| Comando | Descrição |
|---------|-----------|
| `:monitor stats` | Mostrar estatísticas de execução |
| `:monitor traces [limit]` | Mostrar traces de execução |
| `:monitor report` | Mostrar relatório detalhado de execução |
| `:monitor clear` | Limpar dados de monitoramento |

### Comandos de Memória

| Comando | Descrição |
|---------|-----------|
| `:memory inspect <agent-id>` | Inspecionar estado de memória de um agente |
| `:memory compact <agent-id>` | Compactar armazenamento de memória de um agente |
| `:memory purge <agent-id>` | Limpar toda a memória de um agente |

### Comandos de Webhook

| Comando | Descrição |
|---------|-----------|
| `:webhook list` | Listar webhooks configurados |
| `:webhook add` | Adicionar um novo webhook |
| `:webhook remove` | Remover um webhook |
| `:webhook test` | Testar um webhook |
| `:webhook logs` | Mostrar logs de webhook |

### Comandos de Gravação

| Comando | Descrição |
|---------|-----------|
| `:record on <file>` | Iniciar gravação da sessão em um arquivo |
| `:record off` | Parar gravação da sessão |

### Comandos de Sessão

| Comando | Descrição |
|---------|-----------|
| `:snapshot` | Criar snapshot da sessão |
| `:clear` | Limpar a sessão |
| `:help` ou `:h` | Mostrar mensagem de ajuda |
| `:version` | Mostrar informações de versão |

## Funcionalidades da DSL

### Definições de Agentes

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

### Definições de Comportamento

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
    # Verificar requisitos de privacidade de dados
    require capability("data_read")

    if (data.contains_pii) {
      return error("Cannot process data with PII")
    }

    # Realizar análise
    # NOTA: analyze() é uma função integrada planejada (ainda não implementada).
    # Este exemplo ilustra o padrão pretendido de definição de comportamento.
    let results = analyze(data, options)
    emit analysis_completed { results: results }

    return results
  }
}
```

### Funções Integradas

| Função | Descrição | Exemplo |
|--------|-----------|---------|
| `print(...)` | Imprimir valores na saída | `print("Hello", name)` |
| `len(value)` | Obter comprimento de string, lista ou mapa | `len("hello")` -> `5` |
| `upper(string)` | Converter string para maiúsculas | `upper("hello")` -> `"HELLO"` |
| `lower(string)` | Converter string para minúsculas | `lower("HELLO")` -> `"hello"` |
| `format(template, ...)` | Formatar string com argumentos | `format("Hello, {}!", name)` |

> **Funções integradas planejadas:** Funções avançadas de E/S como `read_file()`, `read_csv()`, `write_results()`, `analyze()` e `transform_data()` ainda não foram implementadas. Estão planejadas para uma versão futura.

### Tipos de Dados

```rust
# Tipos básicos
let name = "Alice"          # String
let age = 30               # Integer
let height = 5.8           # Number
let active = true          # Boolean
let empty = null           # Null

# Coleções
let items = [1, 2, 3]      # List
let config = {             # Map
  "host": "localhost",
  "port": 8080
}

# Unidades de tempo e tamanho
let timeout = 30s          # Duration
let max_size = 100MB       # Size
```

## Arquitetura

### Componentes

```
symbi repl
├── repl-cli/          # Interface CLI e servidor JSON-RPC
├── repl-core/         # Motor principal do REPL e avaliador
├── repl-proto/        # Definições do protocolo JSON-RPC
└── repl-lsp/          # Implementação do Language Server Protocol
```

### Componentes Principais

- **DslEvaluator**: Executa programas DSL com integração ao runtime
- **ReplEngine**: Coordena avaliação e tratamento de comandos
- **ExecutionMonitor**: Rastreia estatísticas e traces de execução
- **RuntimeBridge**: Integra com o runtime do Symbiont para imposição de políticas
- **SessionManager**: Gerencia snapshots e estado da sessão

### Protocolo JSON-RPC

O REPL suporta JSON-RPC 2.0 para acesso programático:

```json
// Avaliar código DSL
{
  "jsonrpc": "2.0",
  "method": "evaluate",
  "params": {"input": "let x = 42"},
  "id": 1
}

// Resposta
{
  "jsonrpc": "2.0",
  "result": {"value": "42", "type": "integer"},
  "id": 1
}
```

## Segurança e Imposição de Políticas

### Verificação de Capacidades

O REPL impõe requisitos de capacidade definidos nos blocos de segurança dos agentes:

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
    # Isto verificará se o agente possui a capacidade "filesystem"
    require capability("filesystem")
    # NOTA: read_file() é uma função integrada planejada (ainda não implementada).
    # Este exemplo ilustra como a verificação de capacidades funciona.
    let content = read_file(path)
    return content
  }
}
```

### Integração com Políticas

O REPL integra-se com o motor de políticas do Symbiont para impor controles de acesso e requisitos de auditoria.

## Depuração e Monitoramento

### Traces de Execução

```
:monitor traces 10

Recent Execution Traces:
  14:32:15.123 - AgentCreated [Agent: abc-123] (2ms)
  14:32:15.125 - AgentStarted [Agent: abc-123] (1ms)
  14:32:15.130 - BehaviorExecuted [Agent: abc-123] (5ms)
  14:32:15.135 - AgentPaused [Agent: abc-123]
```

### Estatísticas

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

### Depuração de Agentes

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

## Integração com IDEs

### Language Server Protocol

O REPL oferece suporte LSP para integração com IDEs através do crate `repl-lsp`. O servidor LSP é iniciado separadamente do REPL em si:

```bash
# O servidor LSP é fornecido pelo crate repl-lsp e iniciado
# pela configuração do cliente LSP do seu editor (não via flags do symbi repl).
```

> **Nota:** A flag `--lsp` não é suportada no `symbi repl`. O LSP é implementado no crate `repl-lsp` e deve ser configurado através das configurações de LSP do seu editor.

### Funcionalidades Suportadas

- Destaque de sintaxe
- Diagnósticos de erros
- Sincronização de texto

**Funcionalidades planejadas** (ainda não implementadas):
- Autocompletar código
- Informações ao passar o cursor
- Ir para definição
- Busca de símbolos

## Boas Práticas

### Fluxo de Desenvolvimento

1. **Comece com Expressões Simples**: Teste construções básicas da DSL
2. **Defina Agentes Incrementalmente**: Comece com definições mínimas de agentes
3. **Teste Comportamentos Separadamente**: Defina e teste comportamentos antes da integração
4. **Use o Monitoramento**: Aproveite o monitoramento de execução para depuração
5. **Crie Snapshots**: Salve estados importantes da sessão

### Dicas de Desempenho

- Use `:monitor clear` periodicamente para limpar dados de monitoramento
- Limite o histórico de traces com `:monitor traces <limit>`
- Destrua agentes não utilizados para liberar recursos
- Use snapshots para estados complexos de sessão

### Considerações de Segurança

- Sempre defina capacidades apropriadas para os agentes
- Teste a imposição de políticas durante o desenvolvimento
- Use o modo sandbox para código não confiável
- Monitore traces de execução para eventos de segurança

## Solução de Problemas

### Problemas Comuns

**Falha na Criação do Agente**
```
Error: Missing capability: filesystem
```
*Solução*: Adicione as capacidades necessárias ao bloco de segurança do agente

**Timeout de Execução**
```
Error: Maximum execution depth exceeded
```
*Solução*: Verifique se há recursão infinita na lógica do comportamento

**Violação de Política**
```
Error: Policy violation: data access denied
```
*Solução*: Verifique se o agente possui as permissões apropriadas

### Comandos de Depuração

```rust
# Verificar estado do agente
:agent debug <agent-id>

# Ver traces de execução
:monitor traces 50

# Verificar estatísticas do sistema
:monitor stats

# Criar snapshot de depuração
:snapshot
```

## Exemplos

### Agente Simples

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

# Testar o comportamento
let result = Add(5, 3)
print("5 + 3 =", result)
```

### Agente de Processamento de Dados

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

    # NOTA: read_csv(), transform_data() e write_results() são funções
    # integradas planejadas (ainda não implementadas). Este exemplo ilustra
    # o padrão pretendido para comportamentos de processamento de dados.
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

## Veja Também

- [Guia DSL](dsl-guide.md) - Referência completa da linguagem DSL
- [Arquitetura de Runtime](runtime-architecture.md) - Visão geral da arquitetura do sistema
- [Modelo de Segurança](security-model.md) - Detalhes da implementação de segurança
- [Referência da API](api-reference.md) - Documentação completa da API

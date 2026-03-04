---
layout: default
title: Primitivas de Raciocínio Avançado (symbi-dev)
description: "Primitivas avançadas do loop de raciocínio: curadoria de ferramentas, detecção de loops travados, pré-busca de contexto e convenções com escopo de diretório"
nav_exclude: true
---

# Primitivas de Raciocínio Avançado
{: .no_toc }

## Outros idiomas
{: .no_toc}

[English](symbi-dev.md) | [中文简体](symbi-dev.zh-cn.md) | [Español](symbi-dev.es.md) | **Português** | [日本語](symbi-dev.ja.md) | [Deutsch](symbi-dev.de.md)

---

Primitivas de runtime com feature gate que aprimoram o loop de raciocínio com curadoria de ferramentas, detecção de loops travados, pré-busca determinística de contexto e recuperação de convenções com escopo de diretório.
{: .fs-6 .fw-300 }

## Índice
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Visão Geral

O feature gate `symbi-dev` adiciona quatro capacidades avançadas ao loop de raciocínio:

| Primitiva | Problema Resolvido | Módulo |
|-----------|-------------------|--------|
| **Tool Profile** | LLM vê muitas ferramentas, desperdiça tokens em irrelevantes | `tool_profile.rs` |
| **Progress Tracker** | Loops ficam travados retentando o mesmo passo falhando | `progress_tracker.rs` |
| **Pre-Hydration** | Lacuna de contexto no cold-start — agente precisa descobrir referências sozinho | `pre_hydrate.rs` |
| **Scoped Conventions** | Recuperação de convenções é ampla por linguagem, não específica por diretório | `knowledge_bridge.rs` |

### Habilitando

```toml
# No seu Cargo.toml
[dependencies]
symbi-runtime = { version = "1.6", features = ["symbi-dev"] }
```

Ou compile a partir do código-fonte:

```bash
cargo build --features symbi-dev
cargo test --features symbi-dev
```

Todas as primitivas são aditivas e retrocompatíveis — código existente compila e executa identicamente sem o feature gate.

---

## Filtragem de Perfil de Ferramentas

Filtra definições de ferramentas antes que o LLM as veja. Reduz desperdício de tokens e impede que o modelo selecione ferramentas irrelevantes.

### Configuração

```rust
use symbi_runtime::reasoning::ToolProfile;

// Incluir apenas ferramentas relacionadas a arquivos
let profile = ToolProfile::include_only(&["file_*", "code_*"]);

// Excluir ferramentas de debug
let profile = ToolProfile::exclude_only(&["debug_*", "internal_*"]);

// Combinado: incluir ferramentas web, excluir experimentais, limitar a 10
let profile = ToolProfile {
    include: vec!["web_*".into(), "search_*".into()],
    exclude: vec!["web_experimental_*".into()],
    max_tools: Some(10),
    require_verified: false,
};
```

### Pipeline de Filtragem

O pipeline aplica na ordem:

1. **Include** — Se não vazio, apenas ferramentas correspondendo a qualquer glob de inclusão passam
2. **Exclude** — Ferramentas correspondendo a qualquer glob de exclusão são removidas
3. **Verified** — Se `require_verified` for true, apenas ferramentas com `[verified]` na descrição passam
4. **Max cap** — Trunca para `max_tools` se definido

### Sintaxe de Glob

| Padrão | Corresponde |
|--------|------------|
| `web_*` | `web_search`, `web_fetch`, `web_scrape` |
| `tool_?` | `tool_a`, `tool_1` (caractere único) |
| `exact_name` | Apenas `exact_name` |

### Integração com LoopConfig

```rust
let config = LoopConfig {
    tool_profile: Some(ToolProfile::include_only(&["search_*", "file_*"])),
    ..Default::default()
};
```

O perfil é aplicado automaticamente em `ReasoningLoopRunner::run()` após as definições de ferramentas serem populadas pelo executor e pela ponte de conhecimento.

---

## Rastreador de Progresso

Rastreia contagens de retentativa por passo e detecta loops travados comparando saídas de erro consecutivas usando similaridade Levenshtein normalizada.

### Configuração

```rust
use symbi_runtime::reasoning::{ProgressTracker, StepIterationConfig, LimitAction};

let config = StepIterationConfig {
    max_reattempts_per_step: 2,    // Parar após 2 tentativas falhadas
    similarity_threshold: 0.85,    // Erros 85%+ similares = travado
    on_limit_reached: LimitAction::SkipStep,
};

let mut tracker = ProgressTracker::new(config);
```

### Uso (Nível de Coordenador)

O rastreador de progresso **não é conectado ao loop de raciocínio diretamente** — é uma preocupação de ordem superior para coordenadores que orquestram tarefas de múltiplos passos.

```rust
// Iniciar rastreamento de um passo
tracker.begin_step("extract_data");

// Após cada tentativa, registrar o erro e verificar
let decision = tracker.record_and_check("extract_data", &error_output);

match decision {
    StepDecision::Continue => { /* retentar */ }
    StepDecision::Stop { reason } => {
        // Emitir LoopEvent::StepLimitReached e seguir em frente
        match tracker.limit_action() {
            LimitAction::SkipStep => { /* pular para próximo passo */ }
            LimitAction::AbortTask => { /* abortar tarefa inteira */ }
            LimitAction::Escalate => { /* passar para humano */ }
        }
    }
}
```

### Detecção de Travamento

O rastreador calcula a distância Levenshtein normalizada entre saídas de erro consecutivas. Se a similaridade exceder o limite (padrão 85%), o passo é considerado travado — mesmo que a contagem máxima de retentativas não tenha sido atingida.

Isto captura cenários onde o agente continua encontrando o mesmo erro com redação ligeiramente diferente.

---

## Motor de Pre-Hydration

Extrai referências da entrada da tarefa (URLs, caminhos de arquivo, issues/PRs do GitHub) e as resolve em paralelo antes que o loop de raciocínio inicie. Isto elimina a latência de cold-start onde o agente precisaria descobrir e buscar essas referências sozinho.

### Configuração

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

### Padrões Integrados

| Padrão | Tipo | Exemplos de Correspondência |
|--------|------|---------------------------|
| URLs | `url` | `https://example.com/api`, `http://localhost:3000` |
| Caminhos de arquivo | `file` | `./src/main.rs`, `~/config.toml` |
| Issues | `issue` | `#42`, `#100` |
| Pull requests | `pr` | `PR #55`, `pr #12` |

### Padrões Personalizados

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

### Fluxo de Resolução

1. **Extrair** — Padrões regex varrem a entrada da tarefa, deduplicando correspondências
2. **Resolver** — Cada referência é resolvida via a ferramenta configurada (ex.: `web_fetch` para URLs)
3. **Orçamento** — Resultados são podados para caber dentro de `max_context_tokens`
4. **Injetar** — Formatado como uma mensagem de sistema `[PRE_HYDRATED_CONTEXT]` (separada do slot `[KNOWLEDGE_CONTEXT]` da ponte de conhecimento)

### Integração com LoopConfig

```rust
let config = LoopConfig {
    pre_hydration: Some(PreHydrationConfig {
        resolution_tools: [("url".into(), "web_fetch".into())].into(),
        ..Default::default()
    }),
    ..Default::default()
};
```

A pre-hydration executa automaticamente no início de `run_inner()` antes que o loop de raciocínio principal comece. Um evento de journal `LoopEvent::PreHydrationComplete` é emitido com estatísticas de extração e resolução.

---

## Convenções com Escopo de Diretório

Estende a ferramenta `recall_knowledge` com parâmetros `directory` e `scope` para recuperar convenções de codificação com escopo em um diretório específico.

### Como Funciona

Quando chamado com `scope: "conventions"` e um `directory`, a ponte de conhecimento:

1. Busca convenções correspondendo ao caminho do diretório
2. Sobe por diretórios pais (ex.: `src/api/` -> `src/` -> raiz do projeto)
3. Recorre a convenções no nível da linguagem
4. Deduplica por conteúdo em todos os níveis
5. Trunca para o limite solicitado

### Chamada de Ferramenta do LLM

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

### Retrocompatibilidade

Os parâmetros `directory` e `scope` são opcionais. Sem eles, `recall_knowledge` se comporta identicamente à versão padrão — uma busca simples de conhecimento com `query` e `limit`.

---

## Campos de LoopConfig

Quando o feature `symbi-dev` está habilitado, `LoopConfig` ganha três campos opcionais:

```rust
pub struct LoopConfig {
    // ... campos existentes ...

    /// Perfil de ferramentas para filtrar ferramentas visíveis ao LLM.
    pub tool_profile: Option<ToolProfile>,
    /// Limites de iteração por passo para detecção de loops travados.
    pub step_iteration: Option<StepIterationConfig>,
    /// Configuração de pre-hydration para pré-busca determinística de contexto.
    pub pre_hydration: Option<PreHydrationConfig>,
}
```

Todos têm padrão `None` e são serializados com `#[serde(default, skip_serializing_if = "Option::is_none")]` para retrocompatibilidade.

## Eventos de Journal

Duas novas variantes de `LoopEvent` estão disponíveis:

```rust
pub enum LoopEvent {
    // ... variantes existentes ...

    /// Um passo atingiu seu limite de retentativas (emitido por coordenadores).
    StepLimitReached {
        step_id: String,
        attempts: u32,
        reason: String,
    },
    /// Fase de pre-hydration concluída.
    PreHydrationComplete {
        references_found: usize,
        references_resolved: usize,
        references_failed: usize,
        total_tokens: usize,
    },
}
```

---

## Testes

```bash
# Sem feature (sem regressões)
cargo clippy --workspace -j2
cargo test --workspace -j2

# Com feature
cargo clippy --workspace -j2 --features symbi-dev
cargo test --workspace -j2 --features symbi-dev
```

Todos os testes são módulos `#[cfg(test)]` inline — nenhum fixture de teste externo é necessário.

---

## Mapa de Módulos

| Módulo | Tipos Públicos | Descrição |
|--------|---------------|-----------|
| `tool_profile` | `ToolProfile` | Filtragem de ferramentas baseada em glob com flag verified e limite máximo |
| `progress_tracker` | `ProgressTracker`, `StepIterationConfig`, `StepDecision`, `LimitAction` | Rastreamento de iteração por passo com detecção de travamento via Levenshtein |
| `pre_hydrate` | `PreHydrationEngine`, `PreHydrationConfig`, `HydratedContext` | Extração de referências, resolução paralela, poda por orçamento de tokens |
| `knowledge_bridge` | (estendido) | `retrieve_scoped_conventions()`, ferramenta `recall_knowledge` estendida |

---

## Próximos Passos

- **[Guia do Loop de Raciocínio](reasoning-loop.md)** — Documentação do ciclo ORGA principal
- **[Arquitetura de Runtime](runtime-architecture.md)** — Visão geral completa da arquitetura do sistema
- **[Referência da API](api-reference.md)** — Documentação completa da API

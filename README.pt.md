<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | **Português** | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**Runtime de agentes governado por politicas para producao.**

Symbiont e um runtime nativo em Rust para executar agentes de IA, ferramentas e workflows sob controles explicitos de politica, identidade e auditoria.

A maioria dos frameworks de agentes foca em orquestracao. Symbiont foca no que acontece quando agentes precisam rodar em ambientes reais com riscos reais: ferramentas nao confiaveis, dados sensiveis, limites de aprovacao, requisitos de auditoria e aplicacao repetivel de regras.

---

## Por que Symbiont

Agentes de IA sao faceis de demonstrar e dificeis de confiar.

Uma vez que um agente pode chamar ferramentas, acessar arquivos, enviar mensagens ou invocar servicos externos, voce precisa de mais do que prompts e codigo improvisado. Voce precisa de:

* **Aplicacao de politicas** para o que um agente pode fazer — DSL integrado e autorizacao [Cedar](https://www.cedarpolicy.com/)
* **Verificacao de ferramentas** para que a execucao nao seja confianca cega — verificacao criptografica [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) de ferramentas MCP
* **Identidade de agente** para saber quem esta agindo — identidade ES256 ancorada em dominio [AgentPin](https://github.com/ThirdKeyAI/AgentPin)
* **Sandboxing** para cargas de trabalho arriscadas — isolamento Docker com limites de recursos
* **Trilhas de auditoria** para o que aconteceu e por que — logs criptograficamente a prova de adulteracao
* **Workflows de revisao** para acoes que requerem aprovacao — gates de humano-no-loop no ciclo de raciocinio

Symbiont foi construido para essa camada.

---

## Inicio rapido

### Pre-requisitos

* Docker (recomendado) ou Rust 1.82+
* Nenhum banco de dados vetorial externo necessario (LanceDB embutido; Qdrant opcional para implantacoes em escala)

### Executar com Docker

```bash
# Iniciar o runtime (API em :8080, HTTP input em :8081)
docker run --rm -p 8080:8080 -p 8081:8081 ghcr.io/thirdkeyai/symbi:latest up

# Executar apenas o servidor MCP
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Analisar um arquivo DSL de agente
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl
```

### Compilar a partir do codigo-fonte

```bash
cargo build --release
./target/release/symbi --help

# Executar o runtime
cargo run -- up

# REPL interativo
cargo run -- repl
```

> Para implantacoes em producao, revise `SECURITY.md` e o [guia de implantacao](https://docs.symbiont.dev/getting-started) antes de habilitar execucao de ferramentas nao confiaveis.

---

## Como funciona

Symbiont separa a intencao do agente da autoridade de execucao:

1. **Agentes propoem** acoes atraves do ciclo de raciocinio ORGA (Observe-Reason-Gate-Act)
2. **O runtime avalia** cada acao contra verificacoes de politica, identidade e confianca
3. **A politica decide** — acoes permitidas sao executadas; acoes negadas sao bloqueadas ou encaminhadas para aprovacao
4. **Tudo e registrado** — trilha de auditoria a prova de adulteracao para cada decisao

Isso significa que a saida do modelo nunca e tratada como autoridade de execucao. O runtime controla o que realmente acontece.

### Exemplo: ferramenta nao confiavel bloqueada por politica

Um agente tenta chamar uma ferramenta MCP nao verificada. O runtime:

1. Verifica o status de verificacao SchemaPin — assinatura da ferramenta ausente ou invalida
2. Avalia politica Cedar — `forbid(action == Action::"tool_call") when { !resource.verified }`
3. Bloqueia a execucao e registra a negacao com contexto completo
4. Opcionalmente encaminha para um operador para aprovacao manual

Nenhuma alteracao de codigo necessaria. A politica governa a execucao.

---

## Exemplo de DSL

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

## Capacidades principais

| Capacidade | O que faz |
|-----------|-------------|
| **Cedar policy engine** | Autorizacao granular para acoes de agentes, chamadas de ferramentas e acesso a recursos |
| **SchemaPin verification** | Verificacao criptografica de schemas de ferramentas MCP antes da execucao |
| **AgentPin identity** | Identidade ES256 ancorada em dominio para agentes e tarefas agendadas |
| **ORGA reasoning loop** | Ciclo Observe-Reason-Gate-Act com enforcement de typestate, gates de politica e circuit breakers |
| **Sandboxing** | Isolamento baseado em Docker com limites de recursos para cargas de trabalho nao confiaveis |
| **Audit logging** | Logs a prova de adulteracao com registros estruturados para cada decisao de politica |
| **ClawHavoc scanning** | 40 regras em 10 categorias de ataque para analise de conteudo de skills/ferramentas |
| **Secrets management** | Integracao Vault/OpenBao, armazenamento criptografado AES-256-GCM, escopo por agente |
| **Cron scheduling** | Agendador com backend SQLite, jitter, guardas de concorrencia e filas dead-letter |
| **Persistent memory** | Memoria de agente baseada em Markdown com extracao de fatos, procedimentos e compactacao |
| **RAG engine** | Busca hibrida semantica + palavra-chave via LanceDB (embutido) ou Qdrant (escala) |
| **MCP integration** | Suporte nativo ao Model Context Protocol com acesso governado a ferramentas |
| **Webhook verification** | Verificacao HMAC-SHA256 e JWT com presets para GitHub, Stripe e Slack |
| **Delivery routing** | Encaminhar saida do agente para webhooks, Slack, email ou canais personalizados |
| **Metrics & telemetry** | Exportacao OTLP com spans de tracing OpenTelemetry para o ciclo de raciocinio |
| **HTTP security** | Binding somente em loopback, allow-lists CORS, validacao JWT EdDSA, chaves API por agente |
| **AI assistant plugins** | Plugins de governanca para [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) e [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli) |

Desempenho: avaliacao de politica <1ms, verificacao ECDSA P-256 <5ms, agendamento de 10k agentes com <2% de overhead de CPU. Veja [benchmarks](crates/runtime/benches/performance_claims.rs) e [testes de limiar](crates/runtime/tests/performance_claims.rs).

---

## Modelo de seguranca

Symbiont e projetado em torno de um principio simples: **a saida do modelo nunca deve ser confiada como autoridade de execucao.**

Acoes passam por controles do runtime:

* **Zero trust** — todas as entradas de agentes sao nao confiaveis por padrao
* **Verificacoes de politica** — autorizacao Cedar antes de cada chamada de ferramenta e acesso a recurso
* **Verificacao de ferramentas** — verificacao criptografica SchemaPin de schemas de ferramentas
* **Limites de sandbox** — isolamento Docker para execucao nao confiavel
* **Aprovacao do operador** — gates de humano-no-loop para acoes sensiveis
* **Controle de segredos** — backends Vault/OpenBao, armazenamento local criptografado, namespaces de agentes
* **Log de auditoria** — registros criptograficamente a prova de adulteracao de cada decisao

Se voce esta executando codigo nao confiavel ou ferramentas arriscadas, nao dependa de um modelo de execucao local fraco como sua unica barreira. Veja [`SECURITY.md`](SECURITY.md) e a [documentacao do modelo de seguranca](https://docs.symbiont.dev/security-model).

---

## Workspace

| Crate | Descricao |
|-------|-------------|
| `symbi` | Binario CLI unificado |
| `symbi-runtime` | Runtime principal de agentes e motor de execucao |
| `symbi-dsl` | Parser e avaliador de DSL |
| `symbi-channel-adapter` | Adaptadores para Slack/Teams/Mattermost |
| `repl-core` / `repl-proto` / `repl-cli` | REPL interativo e servidor JSON-RPC |
| `repl-lsp` | Suporte a Language Server Protocol |
| `symbi-a2ui` | Painel administrativo (Lit/TypeScript, alpha) |

Plugins de governanca: [`symbi-claude-code`](https://github.com/thirdkeyai/symbi-claude-code) | [`symbi-gemini-cli`](https://github.com/thirdkeyai/symbi-gemini-cli)

---

## Documentacao

* [Primeiros Passos](https://docs.symbiont.dev/getting-started)
* [Modelo de Seguranca](https://docs.symbiont.dev/security-model)
* [Arquitetura do Runtime](https://docs.symbiont.dev/runtime-architecture)
* [Guia do Ciclo de Raciocinio](https://docs.symbiont.dev/reasoning-loop)
* [Guia do DSL](https://docs.symbiont.dev/dsl-guide)
* [Referencia da API](https://docs.symbiont.dev/api-reference)
* [Primitivas Avancadas de Raciocinio](https://docs.symbiont.dev/orga-adaptive)

Se voce esta avaliando Symbiont para producao, comece pela documentacao do modelo de seguranca e primeiros passos.

---

## Licenca

* **Community Edition** (Apache 2.0): Runtime principal, DSL, ciclo de raciocinio ORGA, Cedar policy engine, verificacao SchemaPin/AgentPin, sandboxing Docker, memoria persistente, agendamento cron, integracao MCP, RAG (LanceDB), log de auditoria, verificacao de webhooks, escaneamento ClawHavoc de skills e todas as ferramentas CLI/REPL.
* **Enterprise Edition** (licenca comercial): Sandboxing multi-nivel (gVisor, Firecracker, E2B), trilhas de auditoria criptograficas com exportacoes de conformidade (HIPAA, SOX, PCI-DSS), revisao de ferramentas e deteccao de ameacas com IA, colaboracao multi-agente criptografada, dashboards de monitoramento em tempo real e suporte dedicado. Veja [`enterprise/README.md`](enterprise/README.md) para detalhes.

Entre em contato com [ThirdKey](https://thirdkey.ai) para licenciamento empresarial.

---

*Mesmo agente. Runtime seguro.*

<div align="right">
  <img src="symbi-trans.png" alt="Logo Symbi" width="120">
</div>

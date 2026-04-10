<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | **Português** | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**Runtime de agentes governado por politicas para producao.**
*Mesmo agente. Runtime seguro.*

Symbiont e um runtime nativo em Rust para executar agentes de IA e ferramentas sob controles explicitos de politica, identidade e auditoria.

A maioria dos frameworks de agentes foca em orquestracao. Symbiont foca no que acontece quando agentes precisam rodar em ambientes reais com riscos reais: ferramentas nao confiaveis, dados sensiveis, limites de aprovacao, requisitos de auditoria e aplicacao repetivel de regras.

---

## Por que Symbiont

Agentes de IA sao faceis de demonstrar e dificeis de confiar.

Uma vez que um agente pode chamar ferramentas, acessar arquivos, enviar mensagens ou invocar servicos externos, voce precisa de mais do que prompts e codigo improvisado. Voce precisa de:

* **Aplicacao de politicas** para o que um agente pode fazer — DSL integrado e autorizacao [Cedar](https://www.cedarpolicy.com/)
* **Verificacao de ferramentas** para que a execucao nao seja confianca cega — verificacao criptografica [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) de ferramentas MCP
* **Contratos de ferramentas** para regular como as ferramentas executam — [ToolClad](https://github.com/ThirdKeyAI/ToolClad) com validacao declarativa de argumentos, aplicacao de escopo e prevencao de injecao
* **Identidade de agente** para saber quem esta agindo — identidade ES256 ancorada em dominio [AgentPin](https://github.com/ThirdKeyAI/AgentPin)
* **Sandboxing** para cargas de trabalho arriscadas — isolamento Docker com limites de recursos
* **Trilhas de auditoria** para o que aconteceu e por que — logs criptograficamente a prova de adulteracao
* **Gates de aprovacao** para acoes sensiveis — revisao humana antes da execucao quando a politica exigir

Symbiont foi construido para essa camada.

---

## Inicio rapido

### Pre-requisitos

* Docker (recomendado) ou Rust 1.82+

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

1. **Agentes propoem** acoes atraves do ciclo de raciocinio (Observe-Reason-Gate-Act)
2. **O runtime avalia** cada acao contra verificacoes de politica, identidade e confianca
3. **A politica decide** — acoes permitidas sao executadas; acoes negadas sao bloqueadas ou encaminhadas para aprovacao
4. **Tudo e registrado** — trilha de auditoria a prova de adulteracao para cada decisao

A saida do modelo nunca e tratada como autoridade de execucao. O runtime controla o que realmente acontece.

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
agent secure_analyst(input: DataSet) -> Result {
    policy access_control {
        allow: read(input) if input.verified == true
        deny: send_email without approval
        audit: all_operations
    }

    with memory = "persistent", requires = "approval" {
        result = analyze(input);
        return result;
    }
}
```

Consulte o [guia do DSL](https://docs.symbiont.dev/dsl-guide) para a gramatica completa incluindo blocos `metadata`, `schedule`, `webhook` e `channel`.

---

## Capacidades principais

| Capacidade | O que faz |
|-----------|-------------|
| **Policy engine** | Autorizacao granular [Cedar](https://www.cedarpolicy.com/) para acoes de agentes, chamadas de ferramentas e acesso a recursos |
| **Verificacao de ferramentas** | Verificacao criptografica [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) de schemas de ferramentas MCP antes da execucao |
| **Contratos de ferramentas** | Contratos declarativos [ToolClad](https://github.com/ThirdKeyAI/ToolClad) com validacao de argumentos, aplicacao de escopo e geracao de politicas Cedar |
| **Identidade de agente** | Identidade ES256 ancorada em dominio [AgentPin](https://github.com/ThirdKeyAI/AgentPin) para agentes e tarefas agendadas |
| **Ciclo de raciocinio** | Ciclo Observe-Reason-Gate-Act com enforcement de typestate, gates de politica e circuit breakers |
| **Sandboxing** | Isolamento baseado em Docker com limites de recursos para cargas de trabalho nao confiaveis |
| **Log de auditoria** | Logs a prova de adulteracao com registros estruturados para cada decisao de politica |
| **Gerenciamento de segredos** | Integracao Vault/OpenBao, armazenamento criptografado AES-256-GCM, escopo por agente |
| **Integracao MCP** | Suporte nativo ao Model Context Protocol com acesso governado a ferramentas |

Capacidades adicionais: escaneamento de ameacas para conteudo de ferramentas/skills (40 regras, 10 categorias de ataque), agendamento cron, memoria persistente de agentes, busca hibrida RAG (LanceDB/Qdrant), verificacao de webhooks, roteamento de entrega, telemetria OTLP, hardening de seguranca HTTP e plugins de governanca para [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) e [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli). Consulte a [documentacao completa](https://docs.symbiont.dev) para detalhes.

Benchmarks representativos estao disponiveis no [harness de benchmarks](crates/runtime/benches/performance_claims.rs) e [testes de limiar](crates/runtime/tests/performance_claims.rs).

---

## Modelo de seguranca

Symbiont e projetado em torno de um principio simples: **a saida do modelo nunca deve ser confiada como autoridade de execucao.**

Acoes passam por controles do runtime:

* **Zero trust** — todas as entradas de agentes sao nao confiaveis por padrao
* **Verificacoes de politica** — autorizacao Cedar antes de cada chamada de ferramenta e acesso a recurso
* **Verificacao de ferramentas** — verificacao criptografica SchemaPin de schemas de ferramentas
* **Limites de sandbox** — isolamento Docker para execucao nao confiavel
* **Aprovacao do operador** — gates de revisao humana para acoes sensiveis
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

Se voce esta avaliando Symbiont para producao, comece pela documentacao do modelo de seguranca e primeiros passos.

---

## SDKs

SDKs oficiais para integrar o runtime do Symbiont a partir da sua aplicacao:

| Linguagem | Pacote | Repositorio |
|-----------|--------|-------------|
| **JavaScript/TypeScript** | [symbiont-sdk-js](https://www.npmjs.com/package/symbiont-sdk-js) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-js) |
| **Python** | [symbiont-sdk](https://pypi.org/project/symbiont-sdk/) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-python) |

---

## Licenca

* **Community Edition** (Apache 2.0): Runtime principal, DSL, policy engine, verificacao de ferramentas, sandboxing, memoria de agentes, agendamento, integracao MCP, RAG, log de auditoria e todas as ferramentas CLI/REPL.
* **Enterprise Edition** (comercial): Backends avancados de sandbox, exportacoes de auditoria de conformidade, revisao de ferramentas com IA, colaboracao multi-agente criptografada, dashboards de monitoramento e suporte dedicado.

Entre em contato com [ThirdKey](https://thirdkey.ai) para licenciamento empresarial.

---

<div align="right">
  <img src="symbi-trans.png" alt="Logo Symbi" width="120">
</div>

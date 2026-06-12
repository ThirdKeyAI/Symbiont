<img src="logo-hz.png" alt="Symbi">

[English](README.md) | [中文简体](README.zh-cn.md) | [Español](README.es.md) | **Português** | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)
[![YouTube](https://img.shields.io/badge/YouTube-%40ThirdKeyAI-FF0000?logo=youtube&logoColor=white)](https://www.youtube.com/@ThirdKeyAI)

[![OATS Reference Implementation](https://img.shields.io/badge/OATS-Reference%20Implementation-1f6feb)](https://openagenttruststack.org)
[![DOI Typestate Loops](https://zenodo.org/badge/DOI/10.5281/zenodo.19896446.svg)](https://doi.org/10.5281/zenodo.19896446)
[![DOI ToolClad](https://zenodo.org/badge/DOI/10.5281/zenodo.19957596.svg)](https://doi.org/10.5281/zenodo.19957596)
[![DOI Empirical Eval](https://zenodo.org/badge/DOI/10.5281/zenodo.20043247.svg)](https://doi.org/10.5281/zenodo.20043247)

---

**Runtime de agentes governado por políticas para produção.**
*Mesmo agente. Runtime seguro.*

![A Cedar policy denies a live agent's privileged tool call](https://raw.githubusercontent.com/ThirdKeyAI/Symbiont/main/docs/media/cedar-demo.gif)

> **O que você está vendo:** um modelo real (`claude-haiku-4.5`) pede para listar a frota de agentes. Uma regra Cedar `forbid` nega a chamada em **toda nova tentativa** — sem alteração de código, apenas policy. [Reproduza em um comando ↓](#veja-o-policy-gate-negar-uma-ferramenta--um-comando-sem-configuração) · [▶ Walkthrough completo](https://www.youtube.com/watch?v=RPyKpqKz5ik)

Symbiont é um runtime nativo em Rust para executar agentes de IA e ferramentas sob controles explícitos de política, identidade e auditoria.

A maioria dos frameworks de agentes foca em orquestração. Symbiont foca no que acontece quando agentes precisam rodar em ambientes reais com riscos reais: ferramentas não confiáveis, dados sensíveis, limites de aprovação, requisitos de auditoria e aplicação repetível de regras.

---

## Por que Symbiont

Agentes de IA são fáceis de demonstrar e difíceis de confiar.

Uma vez que um agente pode chamar ferramentas, acessar arquivos, enviar mensagens ou invocar serviços externos, você precisa de mais do que prompts e código improvisado. Você precisa de:

* **Aplicação de políticas** para o que um agente pode fazer — DSL integrado e autorização [Cedar](https://www.cedarpolicy.com/)
* **Verificação de ferramentas** para que a execução não seja confiança cega — verificação criptográfica [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) de ferramentas MCP
* **Contratos de ferramentas** para regular como as ferramentas executam — [ToolClad](https://github.com/ThirdKeyAI/ToolClad) com validação declarativa de argumentos, aplicação de escopo e prevenção de injeção
* **Identidade de agente** para saber quem está agindo — identidade ES256 ancorada em domínio [AgentPin](https://github.com/ThirdKeyAI/AgentPin)
* **Sandboxing** para cargas de trabalho arriscadas — isolamento Docker com limites de recursos
* **Trilhas de auditoria** para o que aconteceu e por quê — logs criptograficamente à prova de adulteração
* **Gates de aprovação** para ações sensíveis — revisão humana antes da execução quando a política exigir

Symbiont foi construído para essa camada.

### Open Agent Trust Stack (OATS) — implementação de referência

Symbiont é a **implementação de referência do [Open Agent Trust Stack (OATS)](https://openagenttruststack.org)** — uma especificação aberta (CC BY 4.0) para proteger a execução de agentes de IA através de enforcement estrutural em vez de interceptação posterior ("definir o que é permitido e tornar todo o resto estruturalmente inexprimível"). A especificação OATS é fundamentada na experiência operacional de produção do Symbiont, e o design do Symbiont segue diretamente as camadas do OATS:

| Camada OATS | Mapeamento no Symbiont |
|---|---|
| **Layer 1 — ORGA Loop** (Observe-Reason-Gate-Act com enforcement de typestate) | `crates/runtime/src/reasoning/` — fases com enforcement de typestate; o policy gate é não-ignorável em tempo de compilação. Veja [Wanger 2026 / DOI 10.5281/zenodo.19896446](https://doi.org/10.5281/zenodo.19896446). |
| **Layer 2 — Tool Contracts** | Manifestos declarativos `.clad.toml` do [ToolClad](https://github.com/ThirdKeyAI/ToolClad) + a fence de typestate `agent_summary` em `crates/runtime/src/toolclad/`. Veja [Wanger 2026 / DOI 10.5281/zenodo.19957596](https://doi.org/10.5281/zenodo.19957596). |
| **Layer 3 — Identity** | [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) para ferramentas MCP + identidade de agente ES256 ancorada em domínio [AgentPin](https://github.com/ThirdKeyAI/AgentPin). |
| **Layer 4 — Policy Engine** | Policy gate Cedar (`crates/runtime/src/reasoning/cedar_gate.rs`) + `CommunicationPolicyGate` para chamadas entre agentes; ambos fail-closed por padrão desde a v1.14.0. |
| **Layer 5 — Audit Journal** | `BufferedJournal` com hash encadeado e assinado com Ed25519 no loop de raciocínio; logs de I/O do modelo criptografados em `crates/runtime/src/logging.rs`. |

Symbiont está em conformidade com **OATS Extended** (C1–C7 + E1–E8). A comparação empírica de runtimes de enforcement estrutural que fundamenta a especificação é [Wanger 2026 / DOI 10.5281/zenodo.20043247](https://doi.org/10.5281/zenodo.20043247).

---

## Início rápido

### Veja o policy gate negar uma ferramenta — um comando, sem configuração

Um `forbid` Cedar bloqueia uma ferramenta privilegiada enquanto uma segura passa. Copie e cole isto contra a imagem publicada (sem clone, sem build):

```bash
docker run --rm --entrypoint sh ghcr.io/thirdkeyai/symbi:latest -c '
mkdir -p /tmp/p && cat > /tmp/p/policy.cedar <<EOF
forbid(principal, action == Symbi::Action::"tool_call::list_agents",   resource);
permit(principal, action == Symbi::Action::"tool_call::system_health", resource);
EOF
echo "{\"tool_name\":\"list_agents\"}"   | symbi policy evaluate --stdin --policies /tmp/p --json
echo "{\"tool_name\":\"system_health\"}" | symbi policy evaluate --stdin --policies /tmp/p --json'
```

```json
{"decision":"deny","reason":"deny policies matched: policy_0","tool":"list_agents", ...}
{"decision":"allow","reason":"allow policies matched: policy_1","tool":"system_health", ...}
```

É o mesmo gate Cedar que o runtime conecta ao loop de raciocínio ao vivo — exatamente a negação mostrada na demo acima.

### Instale o CLI

```bash
# Linux / macOS — installs the `symbi` binary to /usr/local/bin
curl -fsSL https://symbiont.dev/install.sh | bash
symbi --help
```

O instalador baixa o binário de release pré-compilado para a sua plataforma. Fixe uma versão com `bash -s -- --version v1.15.2` ou altere o destino com `--dir`. Prefere Docker ou [compilar a partir do código-fonte](#compilar-a-partir-do-código-fonte)? Ambos estão abaixo.

### Pré-requisitos

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

### Compilar a partir do código-fonte

```bash
cargo build --release
./target/release/symbi --help

# Executar o runtime
cargo run -- up

# REPL interativo
cargo run -- repl
```

> Para implantações em produção, revise `SECURITY.md` e o [guia de implantação](https://docs.symbiont.dev/getting-started) antes de habilitar execução de ferramentas não confiáveis.

---

## Como funciona

Symbiont separa a intenção do agente da autoridade de execução:

1. **Agentes propõem** ações através do ciclo de raciocínio (Observe-Reason-Gate-Act)
2. **O runtime avalia** cada ação contra verificações de política, identidade e confiança
3. **A política decide** — ações permitidas são executadas; ações negadas são bloqueadas ou encaminhadas para aprovação
4. **Tudo é registrado** — trilha de auditoria à prova de adulteração para cada decisão

A saída do modelo nunca é tratada como autoridade de execução. O runtime controla o que realmente acontece.

### Exemplo: ferramenta não confiável bloqueada por política

Um agente tenta chamar uma ferramenta MCP não verificada. O runtime:

1. Verifica o status de verificação SchemaPin — assinatura da ferramenta ausente ou inválida
2. Avalia política Cedar — `forbid(action == Action::"tool_call") when { !resource.verified }`
3. Bloqueia a execução e registra a negação com contexto completo
4. Opcionalmente encaminha para um operador para aprovação manual

Nenhuma alteração de código necessária. A política governa a execução.

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

Consulte o [guia do DSL](https://docs.symbiont.dev/dsl-guide) para a gramática completa incluindo blocos `metadata`, `schedule`, `webhook` e `channel`.

---

## Capacidades principais

| Capacidade | O que faz |
|-----------|-------------|
| **Policy engine** | Autorização granular [Cedar](https://www.cedarpolicy.com/) para ações de agentes, chamadas de ferramentas e acesso a recursos |
| **Verificação de ferramentas** | Verificação criptográfica [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) de schemas de ferramentas MCP antes da execução |
| **Contratos de ferramentas** | Contratos declarativos [ToolClad](https://github.com/ThirdKeyAI/ToolClad) com validação de argumentos, aplicação de escopo e geração de políticas Cedar |
| **Identidade de agente** | Identidade ES256 ancorada em domínio [AgentPin](https://github.com/ThirdKeyAI/AgentPin) para agentes e tarefas agendadas |
| **Ciclo de raciocínio** | Ciclo Observe-Reason-Gate-Act com enforcement de typestate, gates de política e circuit breakers |
| **Sandboxing** | Isolamento baseado em Docker com limites de recursos para cargas de trabalho não confiáveis |
| **Log de auditoria** | Logs à prova de adulteração com registros estruturados para cada decisão de política |
| **Gerenciamento de segredos** | Integração Vault/OpenBao, armazenamento criptografado AES-256-GCM, escopo por agente |
| **Integração MCP** | Suporte nativo ao Model Context Protocol com acesso governado a ferramentas |

Capacidades adicionais: escaneamento de ameaças para conteúdo de ferramentas/skills (40 regras, 10 categorias de ataque), agendamento cron, memória persistente de agentes, busca híbrida RAG (LanceDB/Qdrant), verificação de webhooks, roteamento de entrega, telemetria OTLP, hardening de segurança HTTP e plugins de governança para [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) e [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli). Consulte a [documentação completa](https://docs.symbiont.dev) para detalhes.

Benchmarks representativos estão disponíveis no [harness de benchmarks](crates/runtime/benches/performance_claims.rs) e [testes de limiar](crates/runtime/tests/performance_claims.rs).

---

## Modelo de segurança

Symbiont é projetado em torno de um princípio simples: **a saída do modelo nunca deve ser confiada como autoridade de execução.**

Ações passam por controles do runtime:

* **Zero trust** — todas as entradas de agentes são não confiáveis por padrão
* **Verificações de política** — autorização Cedar antes de cada chamada de ferramenta e acesso a recurso
* **Verificação de ferramentas** — verificação criptográfica SchemaPin de schemas de ferramentas
* **Limites de sandbox** — isolamento Docker para execução não confiável
* **Aprovação do operador** — gates de revisão humana para ações sensíveis
* **Controle de segredos** — backends Vault/OpenBao, armazenamento local criptografado, namespaces de agentes
* **Log de auditoria** — registros criptograficamente à prova de adulteração de cada decisão

Se você está executando código não confiável ou ferramentas arriscadas, não dependa de um modelo de execução local fraco como sua única barreira. Veja [`SECURITY.md`](SECURITY.md) e a [documentação do modelo de segurança](https://docs.symbiont.dev/security-model).

---

## Workspace

| Crate | Descrição |
|-------|-------------|
| `symbi` | Binário CLI unificado |
| `symbi-runtime` | Runtime principal de agentes e motor de execução |
| `symbi-dsl` | Parser e avaliador de DSL |
| `symbi-channel-adapter` | Adaptadores para Slack/Teams/Mattermost |
| `repl-core` / `repl-proto` / `repl-cli` | REPL interativo e servidor JSON-RPC |
| `repl-lsp` | Suporte a Language Server Protocol |
| `symbi-shell` | TUI interativa para autoria, orquestração e attach remoto (beta) |
| `symbi-a2ui` | Painel administrativo (Lit/TypeScript, alpha) |

Plugins de governança: [`symbi-claude-code`](https://github.com/thirdkeyai/symbi-claude-code) | [`symbi-gemini-cli`](https://github.com/thirdkeyai/symbi-gemini-cli)

---

## Documentação

* [Primeiros Passos](https://docs.symbiont.dev/getting-started)
* [Modelo de Segurança](https://docs.symbiont.dev/security-model)
* [Arquitetura do Runtime](https://docs.symbiont.dev/runtime-architecture)
* [Guia do Ciclo de Raciocínio](https://docs.symbiont.dev/reasoning-loop)
* [Guia do DSL](https://docs.symbiont.dev/dsl-guide)
* [Referência da API](https://docs.symbiont.dev/api-reference)

Se você está avaliando Symbiont para produção, comece pela documentação do modelo de segurança e primeiros passos.

---

## SDKs

SDKs oficiais para integrar o runtime do Symbiont a partir da sua aplicação:

| Linguagem | Pacote | Repositório |
|-----------|--------|-------------|
| **JavaScript/TypeScript** | [symbiont-sdk-js](https://www.npmjs.com/package/symbiont-sdk-js) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-js) |
| **Python** | [symbiont-sdk](https://pypi.org/project/symbiont-sdk/) | [GitHub](https://github.com/ThirdKeyAI/symbiont-sdk-python) |

---

## Licença

* **Community Edition** (Apache 2.0): Runtime principal, DSL, policy engine, verificação de ferramentas, sandboxing, memória de agentes, agendamento, integração MCP, RAG, log de auditoria e todas as ferramentas CLI/REPL.
* **Enterprise Edition** (comercial): Backends avançados de sandbox, exportações de auditoria de conformidade, revisão de ferramentas com IA, colaboração multi-agente criptografada, dashboards de monitoramento e suporte dedicado.

Entre em contato com [ThirdKey](https://thirdkey.ai) para licenciamento empresarial.

---

<div align="right">
  <img src="symbi-trans.png" alt="Logo Symbi" width="120">
</div>

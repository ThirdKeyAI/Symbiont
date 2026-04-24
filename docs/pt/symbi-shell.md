---
nav_exclude: true
---

# Symbi Shell — Orquestração Interativa de Agentes

> **Status: Beta.** O `symbi shell` é utilizável no dia a dia, mas a superfície de comandos, atalhos de teclado e formatos de persistência ainda podem mudar entre versões menores. Reporte issues em [thirdkeyai/symbiont](https://github.com/thirdkeyai/symbiont/issues) com o rótulo `shell`.

O `symbi shell` é uma interface de terminal baseada em [ratatui](https://ratatui.rs) para construir, orquestrar e operar agentes Symbiont. Ele se apoia no mesmo runtime que `symbi up` e `symbi run`, mas o expõe como uma sessão interativa com autoria conversacional, orquestração ao vivo e attach remoto.

## Quando usar o shell

| Caso de uso | Comando |
|-------------|---------|
| Fazer scaffolding de um projeto e iterar em agentes, ferramentas e políticas com assistência de LLM | `symbi shell` |
| Executar um agente até a conclusão sem um laço interativo | `symbi run <agent> -i <json>` |
| Iniciar o runtime completo para webhooks, cron e adaptadores de chat | `symbi up` |

O shell é o ponto de entrada padrão para autoria. Os comandos não interativos são melhores em CI, jobs cron e pipelines de deploy.

## Iniciando

```bash
symbi shell                    # inicia uma sessão nova
symbi shell --list-sessions    # mostra as sessões salvas e sai
symbi shell --resume <id>      # reabre uma sessão por UUID
```

`--resume` aceita tanto um UUID quanto um nome de snapshot salvo anteriormente com `/snapshot`.

## Layout

O shell usa uma viewport inline que compartilha o terminal com o scrollback existente. De cima para baixo, você verá:

- **Barra lateral de estrutura do projeto** (alternável) — árvore de arquivos do projeto atual, destacando agentes, políticas e ferramentas.
- **Linha do tempo de trace** — cartões coloridos por fase ORGA para Observe, Reason, Gate e Act, transmitidos em tempo real durante chamadas ao LLM.
- **Cartão do agente** — os metadados, políticas e invocações recentes do agente atualmente selecionado.
- **Linha de entrada** — digite `/command` ou prosa livre. `@mention` traz caminhos e agentes via autocompletar fuzzy.

O destaque de sintaxe cobre o DSL do Symbiont, Cedar e manifestos ToolClad via gramáticas tree-sitter.

### Atalhos de teclado

| Atalho | Ação |
|--------|------|
| `Enter` | Submeter a entrada (funciona mesmo quando o popup de autocompletar está visível) |
| `/` ou `@` | Abrir automaticamente o popup de autocompletar |
| `↑` / `↓` | Navegar pelo histórico de entrada ou pelas entradas do popup |
| `Ctrl+R` | Busca reversa no histórico |
| `Tab` | Aceitar a sugestão destacada |
| `Esc` | Fechar o popup / cancelar uma chamada de LLM em andamento |
| `Ctrl+L` | Limpar o buffer de saída visível |
| `Ctrl+D` | Sair do shell |

Sob o Zellij, o shell detecta o multiplexador e imprime um aviso de compatibilidade de viewport inline; use `--full-screen` se quiser rodar em um buffer de tela alternativo.

## Catálogo de comandos

Os comandos são agrupados por propósito. Todo comando aceita `help` / `--help` / `-h` para imprimir um breve resumo de uso sem despachar para o orquestrador.

### Autoria

| Comando | O que faz |
|---------|-----------|
| `/init [profile\|description]` | Faz scaffolding de um projeto Symbiont. Nomes de perfil conhecidos (`minimal`, `assistant`, `dev-agent`, `multi-agent`) executam um scaffold determinístico; qualquer outra string é tratada como uma descrição livre que o orquestrador usa para escolher um perfil. |
| `/spawn <description>` | Gera um agente DSL a partir de prosa. O resultado é validado contra as restrições do projeto antes de ser gravado em `agents/`. |
| `/policy <requirement>` | Gera uma política Cedar para o requisito descrito e a valida. |
| `/tool <description>` | Gera um manifesto ToolClad `.clad.toml` e o valida. |
| `/behavior <description>` | Gera um bloco de comportamento DSL reutilizável e o valida. |

Os comandos de autoria gravam em disco apenas após a validação passar. Violações de restrição são explicadas na linha do tempo de trace com erros precisos de linha.

### Orquestração

| Comando | Padrão |
|---------|--------|
| `/run <agent> [input]` | Iniciar ou reexecutar um agente. |
| `/ask <agent> <message>` | Enviar uma mensagem a um agente e esperar pela resposta. |
| `/send <agent> <message>` | Enviar uma mensagem sem esperar pela resposta. |
| `/chain <a,b,c> <input>` | Encadear a saída de cada agente na entrada do próximo. |
| `/parallel <a,b,c> <input>` | Executar agentes em paralelo com a mesma entrada; agregar resultados. |
| `/race <a,b,c> <input>` | Executar em paralelo; a primeira resposta bem-sucedida vence, o restante é cancelado. |
| `/debate <a,b,c> <topic>` | Debate estruturado entre múltiplos agentes sobre um tópico. |
| `/exec <command>` | Executar um comando de shell dentro do agente dev em sandbox. |

### Operações

| Comando | O que faz |
|---------|-----------|
| `/agents` | Listar agentes ativos. |
| `/monitor [agent]` | Transmitir status ao vivo para o agente indicado (ou todos). |
| `/logs [agent]` | Mostrar logs recentes. |
| `/audit [filter]` | Mostrar entradas recentes da trilha de auditoria; filtrar por agente, decisão ou intervalo de tempo. |
| `/doctor` | Diagnosticar o ambiente de runtime local. |
| `/memory <agent> [query]` | Consultar a memória de um agente. |
| `/debug <agent>` | Inspecionar o estado interno de um agente. |
| `/pause`, `/resume-agent`, `/stop`, `/destroy` | Controles de ciclo de vida do agente. |

### Ferramentas, skills e verificação

| Comando | O que faz |
|---------|-----------|
| `/tools [list\|add\|remove]` | Gerenciar ferramentas ToolClad disponíveis para os agentes. |
| `/skills [list\|install\|remove]` | Gerenciar skills disponíveis para os agentes. |
| `/verify <artifact>` | Verificar um artefato assinado (manifesto de ferramenta, skill) contra sua assinatura SchemaPin. |

### Agendamento

| Comando | O que faz |
|---------|-----------|
| `/cron list` | Listar jobs agendados de agentes. |
| `/cron add` / `/cron remove` | Criar ou excluir jobs agendados. |
| `/cron history` | Mostrar execuções recentes. |

`/cron` funciona tanto localmente quanto sobre um attach remoto (ver abaixo). Consulte o [guia de Agendamento](/scheduling) para o motor cron completo.

### Canais

| Comando | O que faz |
|---------|-----------|
| `/channels` | Listar adaptadores de canal registrados (Slack, Teams, Mattermost). |
| `/connect <channel>` | Registrar um novo adaptador de canal. |
| `/disconnect <channel>` | Remover um adaptador. |

O gerenciamento de canais requer um attach remoto quando direcionado a um runtime implantado.

### Segredos

| Comando | O que faz |
|---------|-----------|
| `/secrets list\|set\|get\|remove` | Gerenciar segredos no armazenamento local criptografado do runtime. |

Os segredos são criptografados em repouso com `SYMBIONT_MASTER_KEY` e escopados por agente.

### Deploy (Beta)

> **Status: Beta.** A stack de deploy é de agente único na edição OSS. Deploys multi-agente e gerenciados estão no roadmap.

| Comando | Alvo |
|---------|------|
| `/deploy local` | Docker com um sandbox runner endurecido no daemon Docker local. |
| `/deploy cloudrun` | Google Cloud Run — compila uma imagem, faz push e implanta um serviço. |
| `/deploy aws` | AWS App Runner. |

`/deploy` lê o agente ativo e a configuração do projeto e produz um artefato de deploy reprodutível. Para topologias multi-agente, implante o coordenador e cada worker separadamente e conecte-os com mensagens entre instâncias (ver [Arquitetura do Runtime](/runtime-architecture#cross-instance-agent-messaging)).

### Attach remoto

| Comando | O que faz |
|---------|-----------|
| `/attach <url>` | Conectar este shell a um runtime remoto via HTTP. |
| `/detach` | Desconectar do runtime remoto atualmente conectado. |

Uma vez conectado, `/cron`, `/channels`, `/agents`, `/audit` e a maioria dos comandos de operações agem no runtime remoto em vez do local. `/secrets` permanece local — segredos remotos ficam no armazenamento do runtime remoto.

### Gerenciamento de sessão

| Comando | O que faz |
|---------|-----------|
| `/snapshot [name]` | Salvar a sessão atual. |
| `/resume <snapshot>` | Restaurar um snapshot salvo. |
| `/export <path>` | Exportar o transcript da conversa para o disco. |
| `/new` | Iniciar uma nova sessão, descartando a atual. |
| `/compact [limit]` | Compactar o histórico da conversa para caber em um orçamento de tokens. |
| `/context` | Mostrar a janela de contexto atual e o uso de tokens. |

As sessões são armazenadas em `.symbi/sessions/<uuid>/`. O shell dispara compactação automática quando o contexto ultrapassa o orçamento configurado.

### Controles de sessão

| Comando | O que faz |
|---------|-----------|
| `/model [name]` | Mostrar ou alternar o modelo de inferência ativo. |
| `/cost` | Mostrar totais de tokens e custo de API da sessão. |
| `/status` | Mostrar status do runtime e da sessão. |
| `/dsl` | Alternar entre os modos de entrada DSL e orquestrador — o modo DSL avalia em processo. |
| `/clear` | Limpar o buffer de saída visível (o histórico é preservado). |
| `/quit` / `/exit` | Sair do shell. |
| `/help` | Mostrar o catálogo de comandos. |

## Modo DSL

Pressione `/dsl` para alternar a linha de entrada para o modo DSL. No modo DSL, o shell analisa e avalia a entrada contra o interpretador DSL em processo, com autocompletar e erros apoiados por tree-sitter, sem rotear pelo orquestrador. Alterne de volta com `/dsl` novamente.

## Restrições e validação

Os comandos de autoria aplicam um pipeline de validação local:

1. Os artefatos gerados são analisados contra a gramática do DSL Symbiont, Cedar ou ToolClad, conforme apropriado.
2. Um carregador de restrições verifica o resultado contra restrições em nível de projeto (por exemplo, capacidades proibidas, políticas obrigatórias).
3. Apenas após ambas as etapas terem sucesso o artefato é gravado em disco.

O LLM orquestrador pode ver os efeitos do arquivo de restrições através de erros de validação, mas não pode modificar o próprio arquivo — é o mesmo modelo de confiança usado pelo pipeline `symbi tools validate`.

## Ressalvas Beta

As partes do shell a seguir ainda estão em desenvolvimento ativo e podem mudar sem janela de depreciação:

- `/branch` e `/copy` (ramificação de sessão) são comandos reservados e atualmente imprimem um stub "planejado para uma versão futura".
- `/deploy cloudrun` e `/deploy aws` são apenas de agente único.
- O formato de snapshot e o layout de `.symbi/sessions/` podem mudar entre versões menores; use `/export` se precisar de transcripts duráveis.
- Heurísticas de autocompletar fuzzy e o layout da linha do tempo de trace são ajustados com base em feedback e podem mudar.

Se você precisa de uma superfície estável hoje, prefira `symbi up`, `symbi run` e a [API HTTP](/api-reference) — esses são cobertos pelas garantias de compatibilidade em `SECURITY.md`.

## Veja também

- [Introducao](/getting-started) — instalação e `symbi init`
- [Guia DSL](/dsl-guide) — referência da linguagem de definição de agentes
- [ToolClad](/toolclad) — contratos declarativos de ferramentas
- [Agendamento](/scheduling) — motor cron e roteamento de entregas
- [Modelo de seguranca](/security-model) — limites de confiança e aplicação de políticas

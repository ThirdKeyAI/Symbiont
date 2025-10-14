# Symbiont V1 Plan

**Goal:** Transform Symbiont into a developer-first platform where you can go from zero to live agent + webhook in under 60 seconds.

**Vision:** Deliver "batteries-in" DX with security & routing baked in, while keeping the mental model small (3 verbs: agent / route / io).

---

## Success Criteria

After V1 implementation, a developer should be able to:

1. Install Symbiont with one command
2. Scaffold a working project from templates in seconds
3. Understand the system through 3 core primitives (agent, route, io)
4. Deploy with production-ready defaults (security, routing, observability)
5. Get helpful, actionable error messages that teach

---

## 1. One-liner Dev Stack

**Goal:** Get to a live agent + webhook in <60 seconds.

### Deliverable

```bash
curl -fsSL https://get.symbi.sh | bash
```

This script should:

- Pull a tiny `symbi` binary (statically linked, ~10-20MB)
- Drop a `symbi.quick.toml` with sensible defaults and dev secrets
- Run `symbi up` → starts runtime, enables HTTP Input on `:8081`, hot-reloads agents

### Implementation Notes

**Binary distribution:**
- Host on GitHub releases or CDN
- Support major platforms: Linux (x64, arm64), macOS (x64, arm64), Windows
- Script should detect platform and download appropriate binary

**Quick start config (`symbi.quick.toml`):**
```toml
[runtime]
mode = "dev"
hot_reload = true

[http]
enabled = true
port = 8081
dev_token = "dev"  # Rotated on each install

[storage]
type = "sqlite"
path = "./symbi.db"

[logging]
level = "info"
format = "pretty"
```

**DX polish commands:**
- `symbi doctor` - checks ports, Docker daemon, Qdrant availability, OS compatibility
- `symbi logs -f` - live tail of runtime logs with colored output
- `symbi status` - shows running agents, routes, and I/O handlers

### Acceptance Criteria

- [ ] Install script works on Linux, macOS, Windows (WSL2)
- [ ] First run creates config, starts runtime, and serves on :8081
- [ ] `symbi doctor` catches common issues (port conflicts, missing deps)
- [ ] Total time from curl to working webhook: <60s

---

## 2. Scaffolds That Actually Work Day-1

**Goal:** Provide production-shaped templates, not toy demos.

### CLI Generator

```bash
symbi new <template> [project-name]
```

Creates a runnable project with:
- Tests (unit + integration)
- Policy definitions
- Example routes
- README with next steps

### Recommended Templates

#### 1. `webhook-min`
**Description:** Minimal webhook handler with bearer token auth + JSON echo

**Generated structure:**
```
my-webhook/
├── agents/
│   └── webhook_handler.dsl
├── policies/
│   └── webhook_policy.dsl
├── tests/
│   └── webhook_test.sh
├── symbi.toml
└── README.md
```

**Example usage:**
```bash
symbi new webhook-min my-webhook
cd my-webhook
symbi up
curl -H "Authorization: Bearer dev" localhost:8081/webhook -d '{"ping":"pong"}'
```

#### 2. `webscraper-agent`
**Description:** Web scraper tool + guard policy + sample prompt

**Features:**
- HTTP fetch tool with rate limiting
- URL validation policy
- Sample prompts for content extraction
- Test suite with mock responses

#### 3. `slm-first`
**Description:** Router + SLM allow-list + confidence fallback

**Features:**
- Pre-configured router with SLM (Llama 3.2 3B)
- LLM fallback (GPT-4 or Claude) for low-confidence tasks
- Cost tracking and metrics
- Example prompts for common tasks (summarize, classify, extract)

#### 4. `rag-lite`
**Description:** Qdrant + ingestion scripts + search agent

**Features:**
- Qdrant vector DB integration
- Two ingestion scripts (file watcher + API endpoint)
- RAG agent with hybrid search
- Sample documents and queries

### Implementation Notes

**Template storage:**
- Ship templates in binary or download from GitHub on first use
- Templates should be versionable and updatable (`symbi template update`)

**Template variables:**
- `{{project_name}}` - replaced in file names and content
- `{{author}}` - from git config or prompt
- `{{timestamp}}` - ISO 8601 timestamp
- `{{dev_token}}` - generated secure random token

**Post-creation hooks:**
- Run `symbi doctor` to check prerequisites
- Run initial tests to verify template integrity
- Print next steps and example commands

### Acceptance Criteria

- [ ] All 4 templates generate and run without errors
- [ ] Each template includes working tests
- [ ] README in each template explains architecture and next steps
- [ ] `symbi new --list` shows all available templates with descriptions

---

## 3. Zero-config HTTP Entrypoint

**Goal:** Make HTTP Input the default in dev mode with smart routing.

### Auto-routing Behavior

When you run `symbi up` with no explicit HTTP config:

1. Scan `agents/` directory for webhook-capable agents
2. Auto-expose `/webhook` → routes to the first matching agent
3. Log the auto-configuration for transparency

**Example output:**
```
✓ Runtime started on :8080
✓ HTTP Input enabled on :8081
→ Auto-routing /webhook → agents/webhook_handler.dsl
→ Dev token: dev (insecure, rotate in production)
```

### Production Flags

One command to unlock prod-ish behavior:

```bash
symbi up --http.token=env:DEV_TOKEN --http.cors --http.audit
```

**Flags:**
- `--http.token=<value>` - bearer token (literal, env, or file)
- `--http.cors` - enable CORS with sensible defaults
- `--http.audit` - log all requests to audit log
- `--http.tls=<cert>,<key>` - enable HTTPS

### Implementation Notes

**Config precedence:**
1. CLI flags (highest priority)
2. Environment variables (`SYMBI_HTTP_TOKEN`, etc.)
3. `symbi.toml` config file
4. Auto-detected defaults (lowest priority)

**Security defaults:**
- Dev mode: warn about insecure tokens, allow `localhost` only by default
- Prod mode: require strong token (min 32 chars), require explicit CORS origins

### Acceptance Criteria

- [ ] `symbi up` with no config creates working webhook endpoint
- [ ] Dev mode shows clear security warnings
- [ ] Prod flags disable auto-configuration and require explicit config
- [ ] CORS and audit logs work as expected

---

## 4. "Three Primitives" Mental Model

**Goal:** Shrink the surface area to 3 verbs that organize everything.

### The Three Primitives

#### 1. **agent** (define behavior + policy)

An agent is a unit of autonomous behavior with attached policies.

```bash
# Run an agent directly
symbi agent run agents/webhook.dsl

# List all agents
symbi agent list

# Validate agent definition
symbi agent validate agents/webhook.dsl
```

#### 2. **route** (SLM/LLM policy router)

A router decides which model (SLM vs LLM) handles a request based on complexity.

```bash
# Preview router decision for a prompt
symbi route preview --prompt prompt.txt

# Show routing stats
symbi route stats

# Test router with examples
symbi route test --examples prompts/
```

#### 3. **io** (how the world calls you)

I/O handlers expose your agents to the outside world.

```bash
# Enable HTTP I/O
symbi io http enable --path /webhook --agent webhook_handler

# List all I/O handlers
symbi io list

# Test an I/O handler
symbi io test http --payload '{"test":true}'
```

### Documentation Structure

Reorganize docs around these three primitives:

```
docs/
├── primitives/
│   ├── agents.md
│   ├── routes.md
│   └── io.md
├── guides/
│   ├── first-agent.md
│   ├── routing-strategies.md
│   └── io-handlers.md
└── reference/
    ├── cli.md
    ├── dsl-spec.md
    └── api.md
```

### Acceptance Criteria

- [ ] CLI commands grouped by primitive
- [ ] `symbi --help` shows clear grouping (agent/route/io)
- [ ] Docs explain the mental model upfront
- [ ] All examples use this vocabulary consistently

---

## 5. "Batteries-in" Defaults for Security & Routing

**Goal:** Ship presets for policies and routing so developers don't start from blank slate.

### Policy Presets

Located in `policy.presets/`:

#### `relaxed.dsl`
- Minimal restrictions
- Useful for prototyping
- Logs warnings instead of blocking

#### `balanced.dsl` (default for dev)
- Blocks dangerous operations (file system writes, network access to private IPs)
- Allows most common LLM operations
- Enforces rate limits

#### `strict.dsl`
- Production-ready security
- Explicit allow-lists only
- Requires audit logs
- No network access to untrusted domains

### Routing Presets

Located in `routing.presets/`:

#### `simple.toml` (default)
```toml
[router]
strategy = "confidence"

[slm]
model = "llama-3.2-3b"
confidence_threshold = 0.7
tasks = ["classify", "summarize", "extract"]

[llm]
model = "gpt-4o-mini"
fallback_on_low_confidence = true
```

#### `code.toml`
- Optimized for code generation
- Uses DeepSeek-Coder (SLM) for simple tasks
- Falls back to GPT-4 or Claude for complex refactoring

#### `analysis.toml`
- Optimized for data analysis
- Uses Llama for structured extraction
- Falls back to Claude for reasoning tasks

### SLM Presets

Located in `slm.presets/`:

#### `cpu.toml`
```toml
[slm]
models = ["llama-3.2-3b", "phi-3-mini"]
max_memory = "4GB"
threads = 4
```

#### `small-gpu.toml`
```toml
[slm]
models = ["llama-3.2-8b", "mistral-7b"]
max_memory = "8GB"
gpu_layers = 35
```

### Usage

```bash
# Start with a preset
symbi up --preset=dev-simple

# Combine presets
symbi up --policy=strict --routing=code --slm=small-gpu

# Generate custom preset from current config
symbi preset save my-config
```

### Acceptance Criteria

- [ ] All presets included in binary or downloadable
- [ ] Presets are well-documented with use cases
- [ ] `symbi preset list` shows all available presets
- [ ] Users can extend/override presets

---

## 6. SDK "Single-function" Quick Paths

**Goal:** Wrap the full client in a happy-path facade for instant productivity.

### JavaScript/TypeScript

```typescript
import { quick } from '@symbiont/core/quick';

const app = await quick({
  agent: 'webhook_handler',
  http: {
    path: '/webhook',
    token: process.env.DEV_TOKEN
  }
});

// Optional: run an ad-hoc prompt
const res = await app.ask('Summarize this JSON:', { json });
console.log(res);

// Access full client if needed
app.client.agents.list();
```

### Python

```python
from symbiont.quick import quick

app = quick(
    agent='webhook_handler',
    http={'path': '/webhook', 'token': 'dev'}
)

# Ad-hoc prompt
response = app.ask('health?')
print(response)

# Access full client
app.client.agents.list()
```

### Rust

```rust
use symbiont::quick;

#[tokio::main]
async fn main() -> Result<()> {
    let app = quick()
        .agent("webhook_handler")
        .http("/webhook", "dev")
        .build()
        .await?;

    let res = app.ask("Summarize this JSON:", json!({ "data": "..." })).await?;
    println!("{}", res);

    Ok(())
}
```

### Quick API Features

The `quick` facade should:

1. **Auto-discover the local runtime** (or start one in dev mode)
2. **Apply the balanced policy preset** by default
3. **Expose a tiny API:**
   - `ask(prompt, context?)` - run a prompt against the agent
   - `exec(tool, args?)` - execute a tool call
   - `stream(prompt, callback)` - stream responses
4. **Provide access to full client** via `.client` property

### Implementation Notes

**Auto-discovery:**
- Check for `SYMBI_RUNTIME_URL` env var
- Try `localhost:8080` (default runtime port)
- If not found and in dev mode, run `symbi up` as subprocess

**Error handling:**
- Wrap all errors in friendly messages
- Suggest fixes (e.g., "Runtime not found. Run: symbi up")

### Acceptance Criteria

- [ ] Quick API works in JS/TS, Python, and Rust
- [ ] Auto-discovers or starts local runtime
- [ ] `ask()` and `exec()` work with simple examples
- [ ] Full client accessible via `.client` property

---

## 7. VS Code Extension (MVP)

**Goal:** First-class editor support for Symbiont DSL and workflows.

### Features

#### 1. Syntax Highlighting
- Colorize DSL keywords (`agent`, `policy`, `with`, `tool`, etc.)
- Highlight string interpolation
- Mark deprecated syntax

#### 2. Snippets
- `agent-basic` - basic agent template
- `policy-allow` - allow policy rule
- `tool-def` - tool definition
- `route-config` - routing configuration

#### 3. Editor Gutter Actions
- **Run** button - execute agent directly from editor
- **Restart** button - reload agent with changes
- **Test** button - run agent tests

#### 4. Route Preview Panel
- Input: prompt or file
- Output: which model/agent would handle it
- Shows confidence score and reasoning

### Implementation Notes

**Extension structure:**
```
vscode-symbiont/
├── package.json
├── syntaxes/
│   └── symbiont.tmLanguage.json
├── snippets/
│   └── symbiont.json
├── src/
│   ├── extension.ts
│   ├── languageServer.ts
│   └── routePreview.ts
└── README.md
```

**Communication with runtime:**
- Use HTTP API (`localhost:8080`) for commands
- WebSocket for live logs and route preview

**Settings:**
- `symbiont.runtime.url` - runtime URL (default: `http://localhost:8080`)
- `symbiont.runtime.autoStart` - start runtime if not running (default: `true`)
- `symbiont.logs.level` - log level for extension (default: `info`)

### Acceptance Criteria

- [ ] Syntax highlighting works for `.dsl` files
- [ ] Snippets available via IntelliSense
- [ ] Run/Restart buttons work from editor
- [ ] Route preview panel shows model selection logic
- [ ] Extension published to VS Code marketplace

---

## 8. Tiny "Recipes" Gallery

**Goal:** Copy-paste wins for common use cases.

### Location

- Website: `https://symbi.sh/recipes/`
- CLI: `symbi recipe list` and `symbi recipe show <name>`
- Docs: `docs/recipes/`

### Recipe Format

Each recipe should have:

1. **Job-to-be-done title** (e.g., "Accept a webhook, push to Slack")
2. **10-line working example** (agent + policy + config)
3. **curl test command** to verify it works
4. **Next steps** (links to related docs)

### Recipe List

#### 1. Accept a webhook, push to Slack

```yaml
# webhook-to-slack.dsl
agent webhook_to_slack:
  on_http_post:
    parse: json
    validate: schema.json
    call: slack.send(channel="#alerts", message=body.text)
```

**Test:**
```bash
curl -H "Authorization: Bearer dev" localhost:8081/webhook \
  -d '{"text":"Hello from Symbiont!"}'
```

#### 2. Summarize a URL (with fetch tool)

```yaml
agent url_summarizer:
  tools: [http.fetch]
  prompt: |
    Fetch {{ url }} and summarize the main points in 3 bullet points.
```

**Test:**
```bash
curl localhost:8081/webhook -d '{"url":"https://example.com"}'
```

#### 3. SLM-first coding boilerplate

```yaml
agent code_helper:
  router:
    slm: llama-3.2-3b
    llm: gpt-4
    strategy: confidence

  on_prompt:
    if: confidence < 0.7
      use: llm
    else:
      use: slm
```

#### 4. RAG over a folder of MD files

```yaml
agent doc_search:
  vector_db: qdrant
  collection: docs

  on_query:
    embed: query
    search: collection=docs, top_k=5
    prompt: |
      Answer the question using these docs:
      {{ search_results }}

      Question: {{ query }}
```

#### 5. Guard a tool call with policy + audit

```yaml
agent guarded_api:
  tools: [external.api]
  policy: strict
  audit: enabled

  on_call:
    before:
      log: audit
      check: policy.allow(tool=external.api)
    after:
      log: result
```

### Implementation Notes

**CLI recipe command:**
```bash
# List all recipes
symbi recipe list

# Show recipe details
symbi recipe show webhook-to-slack

# Scaffold from recipe
symbi new --recipe webhook-to-slack my-project
```

**Recipe metadata:**
```yaml
# metadata.yml
name: webhook-to-slack
title: Accept a webhook, push to Slack
description: Receive JSON webhooks and forward them to Slack
tags: [webhook, integration, slack]
difficulty: beginner
time: 5 minutes
```

### Acceptance Criteria

- [ ] At least 5 recipes available at launch
- [ ] Each recipe works without modification
- [ ] Recipes indexed on website with search
- [ ] `symbi recipe` command browses and scaffolds from recipes

---

## 9. Docker Compose for "Full-but-friendly"

**Goal:** One command to bring up Symbiont + dependencies.

### docker-compose.yml

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"  # Runtime API
      - "8081:8081"  # HTTP Input
    environment:
      - SYMBI_MODE=production
      - SYMBI_QDRANT_URL=http://qdrant:6333
      - SYMBI_POSTGRES_URL=postgres://symbi:symbi@postgres:5432/symbi
    volumes:
      - ./agents:/app/agents
      - ./policies:/app/policies
      - ./symbi.toml:/app/symbi.toml
    depends_on:
      - qdrant
      - postgres

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
    volumes:
      - qdrant_data:/qdrant/storage

  postgres:
    image: postgres:15
    environment:
      - POSTGRES_USER=symbi
      - POSTGRES_PASSWORD=symbi
      - POSTGRES_DB=symbi
    volumes:
      - postgres_data:/var/lib/postgresql/data
    ports:
      - "5432:5432"

  # Optional: Observability
  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - prometheus_data:/prometheus

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
    volumes:
      - grafana_data:/var/lib/grafana
      - ./grafana/dashboards:/etc/grafana/provisioning/dashboards
    depends_on:
      - prometheus

volumes:
  qdrant_data:
  postgres_data:
  prometheus_data:
  grafana_data:
```

### Variants

**Minimal (Symbiont + Qdrant only):**
```bash
docker compose -f docker-compose.minimal.yml up
```

**Full (+ observability):**
```bash
docker compose up
```

### Observability Configs

**prometheus.yml:**
```yaml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'symbiont'
    static_configs:
      - targets: ['symbi:8080']
```

**Grafana dashboards:**
- Agent execution metrics
- Router decisions (SLM vs LLM)
- Policy violations
- HTTP I/O latency

### Acceptance Criteria

- [ ] `docker compose up` starts all services
- [ ] Health checks ensure proper startup order
- [ ] Grafana dashboards pre-configured
- [ ] Documentation includes docker compose quickstart

---

## 10. Error Messages That Teach

**Goal:** When something fails, show exactly what happened and how to fix it.

### Error Message Format

Every error should include:

1. **What Symbiont tried** (agent, route, policy invoked)
2. **Why it failed** (1-line root cause)
3. **Suggested fix** (actionable command or config change)
4. **Clickable fix** (if possible)

### Examples

#### Missing Bearer Token

```
✗ HTTP request rejected: missing bearer token

What was tried:
  → POST /webhook → agents/webhook_handler.dsl
  → Policy: balanced (requires authentication)

Why it failed:
  Request missing 'Authorization: Bearer <token>' header

Suggested fix:
  Add a bearer token to your request:
  curl -H "Authorization: Bearer dev" localhost:8081/webhook -d '{...}'

Or disable auth in dev (insecure):
  symbi up --http.auth=none
```

#### Agent Syntax Error

```
✗ Failed to load agent: syntax error in webhook_handler.dsl:12

What was tried:
  → Parsing agents/webhook_handler.dsl
  → Expected 'tool', 'prompt', or 'policy', found 'tools'

Why it failed:
  Incorrect keyword 'tools' (did you mean 'tool'?)

Suggested fix:
  Change line 12:
    tools: [http.fetch]
  To:
    tool http.fetch

Run validation:
  symbi agent validate agents/webhook_handler.dsl
```

#### CORS Blocked

```
✗ HTTP request blocked by CORS policy

What was tried:
  → Origin: https://example.com
  → HTTP Input CORS config: allow_origins = []

Why it failed:
  Origin 'https://example.com' not in allow list

Suggested fix:
  Add the origin to your CORS config:
  symbi up --http.cors.origins=https://example.com

Or allow all origins in dev (insecure):
  symbi up --http.cors.allow_all
```

#### Qdrant Not Running

```
✗ Vector search failed: Qdrant not reachable

What was tried:
  → Agent 'doc_search' attempted vector search
  → Qdrant URL: http://localhost:6333

Why it failed:
  Connection refused (is Qdrant running?)

Suggested fix:
  Start Qdrant:
  docker run -p 6333:6333 qdrant/qdrant

Or use Docker Compose:
  docker compose up qdrant

Check connectivity:
  symbi doctor
```

### Implementation Notes

**Error types:**
- `ConfigError` - invalid configuration
- `PolicyViolation` - blocked by policy
- `AgentError` - agent execution failed
- `IOError` - I/O handler error (HTTP, MCP, etc.)
- `DependencyError` - external dependency unavailable

**Structured error format:**
```rust
pub struct SymbiError {
    pub kind: ErrorKind,
    pub context: Vec<String>,  // What was tried
    pub cause: String,          // Why it failed
    pub suggestion: String,     // How to fix
    pub fix_command: Option<String>, // Clickable fix
}
```

**`symbi fix` command:**
```bash
# Apply suggested fix from last error
symbi fix

# Apply specific fix
symbi fix add-bearer
symbi fix enable-cors
```

### Acceptance Criteria

- [ ] All error types have structured format
- [ ] Errors include context, cause, and suggestion
- [ ] `symbi fix` can apply common fixes automatically
- [ ] Error messages tested with user feedback

---

## Implementation Roadmap

### Phase 1: Foundation (Weeks 1-2)
- [ ] One-liner installer script
- [ ] `symbi doctor` and `symbi logs`
- [ ] Basic HTTP auto-configuration
- [ ] Structured error messages

### Phase 2: Scaffolding & Templates (Weeks 3-4)
- [ ] `symbi new <template>` command
- [ ] All 4 templates (webhook-min, webscraper-agent, slm-first, rag-lite)
- [ ] Template testing and documentation

### Phase 3: Presets & SDK (Weeks 5-6)
- [ ] Policy, routing, and SLM presets
- [ ] Quick SDK for JS/TS and Python
- [ ] Documentation for presets and SDK

### Phase 4: Tooling & DX (Weeks 7-8)
- [ ] VS Code extension (syntax, snippets, gutter actions)
- [ ] Recipe gallery (5+ recipes)
- [ ] Docker Compose setup

### Phase 5: Polish & Launch (Week 9-10)
- [ ] End-to-end testing of all features
- [ ] Documentation review and completion
- [ ] Launch website updates
- [ ] Community feedback and iteration

---

## Success Metrics

After V1 launch, we should see:

1. **Time to first webhook:** <60 seconds (measured)
2. **Template adoption:** 70%+ of new projects use a template
3. **Error resolution:** 80%+ of errors resolved with suggested fixes
4. **Community engagement:** 100+ GitHub stars in first month
5. **Documentation clarity:** <5% of support questions about basic setup

---

## Open Questions & Future Considerations

### Out of Scope for V1
- Multi-language DSL support (only English for V1)
- Enterprise features (SSO, audit dashboard, team management)
- Visual agent builder (code-first for V1)
- Cloud hosting service (self-hosted only for V1)

### Future Enhancements
- **V1.1:** Language server protocol (LSP) for advanced editor features
- **V1.2:** Agent marketplace for sharing and discovering agents
- **V1.3:** Visual routing debugger
- **V2.0:** Distributed runtime for multi-node deployments

---

## References

- [Current Architecture Docs](runtime-architecture.md)
- [DSL Specification](dsl-specification.md)
- [Security Model](security-model.md)
- [Router Design](router_design.md)
- [SLM Config Design](slm_config_design.md)

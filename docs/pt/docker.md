# Guia de Containers Docker

## Outros idiomas


## Imagem Disponível

### Container Unificado Symbi
- **Imagem**: `ghcr.io/thirdkeyai/symbi:latest`
- **Propósito**: Container tudo-em-um com parsing DSL, runtime de agentes e servidor MCP
- **Tamanho**: ~80MB (inclui vector DB e suporte a API HTTP)
- **CLI**: Comando unificado `symbi` com subcomandos para diferentes operações

## Início Rápido

### Criar e executar um projeto (recomendado)

`symbi init` funciona dentro do container e escreve um projeto no diretório do host, incluindo um `docker-compose.yml` pronto para execução e um `.env` com `SYMBIONT_MASTER_KEY` recém-gerada:

```bash
# 1. Criar os arquivos do projeto no host
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. Iniciar o runtime (lê o .env automaticamente)
docker compose up
```

A flag `--dir /workspace` diz ao `symbi init` para escrever no volume montado em vez do WORKDIR da imagem. Após isso, você terá `symbiont.toml`, `agents/`, `policies/`, `.symbiont/audit/`, `AGENTS.md`, `docker-compose.yml`, `.env` e `.env.example` no diretório atual.

Para pular a geração do arquivo compose:

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile minimal --no-interact --no-docker-compose --dir /workspace
```

### Usando Imagem Pré-construída (ad-hoc)

```bash
# Baixar imagem mais recente
docker pull ghcr.io/thirdkeyai/symbi:latest

# Fazer parsing de um arquivo DSL
docker run --rm -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest \
  dsl --file /workspace/agent.dsl

# Executar servidor MCP (baseado em stdio, sem necessidade de porta)
docker run --rm -i \
  ghcr.io/thirdkeyai/symbi:latest \
  mcp

# Executar o runtime sem um projeto (efêmero, sem master key)
docker run --rm -p 8080:8080 -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0
```

### Fluxo de Desenvolvimento

```bash
# Desenvolvimento interativo
docker run --rm -it -v $(pwd):/workspace \
  ghcr.io/thirdkeyai/symbi:latest bash

# Desenvolvimento com montagem de volumes e portas
docker run --rm -it \
  -v $(pwd):/workspace \
  -p 8080:8080 \
  -p 8081:8081 \
  ghcr.io/thirdkeyai/symbi:latest bash
```

## Tags Disponíveis

- `latest` - Última versão estável
- `main` - Último build de desenvolvimento
- `v1.0.0` - Releases de versões específicas
- `sha-<commit>` - Builds de commits específicos

## Construção Local

### Container Unificado Symbi

```bash
# A partir da raiz do projeto
docker build -t symbi:latest .

# Testar o build
docker run --rm symbi:latest --version

# Testar parsing DSL
docker run --rm -v $(pwd):/workspace symbi:latest dsl --help

# Testar servidor MCP
docker run --rm symbi:latest mcp
```

## Suporte Multi-Arquitetura

Imagens são construídas para:
- `linux/amd64` (x86_64)
- `linux/arm64` (ARM64/Apple Silicon)

O Docker automaticamente baixa a arquitetura correta para sua plataforma.

## Funcionalidades de Segurança

### Execução como Não-Root
- Containers executam como usuário não-root `symbi` (UID 1000)
- Superfície de ataque mínima com imagens base com segurança reforçada

### Varredura de Vulnerabilidades
- Todas as imagens são automaticamente varridas com Trivy
- Alertas de segurança publicados na aba Security do GitHub
- Relatórios SARIF para análise detalhada de vulnerabilidades

## Configuração

### Variáveis de Ambiente

**Container Symbi:**
- `SYMBIONT_MASTER_KEY` - **Obrigatório para estado persistente.** Chave hex de 32 bytes usada para criptografar o armazenamento local. Gere com `openssl rand -hex 32`. `symbi init` escreve uma no `.env` automaticamente.
- `RUST_LOG` - Definir nível de log (debug, info, warn, error)
- `SYMBIONT_VECTOR_BACKEND` - Backend de vetores: `lancedb` (padrão) ou `qdrant`
- `QDRANT_URL` - URL do banco de dados vetorial Qdrant (apenas se usar backend Qdrant opcional)
- `OPENROUTER_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` - Credenciais LLM opcionais; qualquer uma delas habilita o endpoint Coordinator Chat.

### Montagem de Volumes

A imagem executa como usuário `symbi` (UID 1000) com `WORKDIR=/var/lib/symbi`. Os arquivos do projeto são montados somente-leitura nesse diretório; o estado persistente (o armazenamento SQLite local e os logs de auditoria) fica em volumes nomeados para sobreviver a reinícios do container.

```bash
# Arquivos do projeto (somente-leitura)
-v $(pwd)/symbiont.toml:/var/lib/symbi/symbiont.toml:ro
-v $(pwd)/agents:/var/lib/symbi/agents:ro
-v $(pwd)/policies:/var/lib/symbi/policies:ro
-v $(pwd)/tools:/var/lib/symbi/tools:ro

# Estado persistente
-v symbi-data:/var/lib/symbi/.symbi
-v symbi-audit:/var/lib/symbi/.symbiont
```

## Exemplo com Docker Compose

`symbi init` gera um `docker-compose.yml` pronto para execução que corresponde ao restante desta seção — prefira isso a escrever um compose à mão. Para referência, ou ao começar sem `init`:

Por padrão, o Symbiont usa **LanceDB** como banco de dados vetorial embarcado -- sem necessidade de serviços externos. Se você precisar de um backend vetorial distribuído para implantações em escala, pode opcionalmente adicionar o Qdrant.

### Mínimo (LanceDB padrão -- sem necessidade de Qdrant)

Combine isso com um arquivo `.env` que defina `SYMBIONT_MASTER_KEY`:

```yaml
services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    command: ["up", "--http-bind", "0.0.0.0"]
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./symbiont.toml:/var/lib/symbi/symbiont.toml:ro
      - ./agents:/var/lib/symbi/agents:ro
      - ./policies:/var/lib/symbi/policies:ro
      - ./tools:/var/lib/symbi/tools:ro
      - symbi-data:/var/lib/symbi/.symbi
      - symbi-audit:/var/lib/symbi/.symbiont
    environment:
      SYMBIONT_MASTER_KEY: ${SYMBIONT_MASTER_KEY:?set SYMBIONT_MASTER_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
    restart: unless-stopped

volumes:
  symbi-data:
  symbi-audit:
```

### Com Backend Qdrant Opcional

```yaml
services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    command: ["up", "--http-bind", "0.0.0.0"]
    ports:
      - "8080:8080"
      - "8081:8081"
    volumes:
      - ./symbiont.toml:/var/lib/symbi/symbiont.toml:ro
      - ./agents:/var/lib/symbi/agents:ro
      - ./policies:/var/lib/symbi/policies:ro
      - symbi-data:/var/lib/symbi/.symbi
      - symbi-audit:/var/lib/symbi/.symbiont
    environment:
      SYMBIONT_MASTER_KEY: ${SYMBIONT_MASTER_KEY:?set SYMBIONT_MASTER_KEY in .env}
      RUST_LOG: ${RUST_LOG:-info}
      SYMBIONT_VECTOR_BACKEND: qdrant
      QDRANT_URL: http://qdrant:6334
    depends_on:
      - qdrant
    restart: unless-stopped

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbi-data:
  symbi-audit:
  qdrant-data:
```

## Solução de Problemas

### Problemas Comuns

**Permissão Negada:**
```bash
# Garantir propriedade correta
sudo chown -R 1000:1000 ./data

# Ou usar usuário diferente
docker run --user $(id -u):$(id -g) ...
```

**Conflitos de Porta:**
```bash
# Usar portas diferentes
docker run -p 8081:8080 ghcr.io/thirdkeyai/symbi:latest
```

**Falhas de Build:**
```bash
# Limpar cache do Docker
docker builder prune -a

# Reconstruir sem cache
docker build --no-cache -f runtime/Dockerfile .
```

### Verificações de Saúde

```bash
# Verificar saúde do container
docker run --name symbi-test -d ghcr.io/thirdkeyai/symbi:latest up --http-bind 0.0.0.0:8080
docker exec symbi-test /usr/local/bin/symbi --version
docker rm -f symbi-test
```

## Otimização de Desempenho

### Limites de Recursos

```bash
# Definir limites de memória e CPU
docker run --memory=512m --cpus=1.0 \
  ghcr.io/thirdkeyai/symbi:latest mcp
```

### Otimização de Build

```bash
# Usar BuildKit para builds mais rápidos
DOCKER_BUILDKIT=1 docker build .

# Cache multi-estágio
docker build --target builder -t symbi-builder .
docker build --cache-from symbi-builder .
```

## Integração CI/CD

O GitHub Actions automaticamente constrói e publica containers em:
- Push para a branch `main`
- Novas tags de versão (`v*`)
- Pull requests (apenas build)

As imagens incluem metadados:
- SHA do commit Git
- Timestamp do build
- Resultados da varredura de vulnerabilidades
- SBOM (Software Bill of Materials)

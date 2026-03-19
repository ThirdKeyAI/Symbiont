# Guia de Containers Docker

## Outros idiomas

[English](docker.md) | [中文简体](docker.zh-cn.md) | [Español](docker.es.md) | O Symbi oferece um container Docker unificado com todas as funcionalidades incluídas, disponível através do GitHub Container Registry.

## Imagem Disponível

### Container Unificado Symbi
- **Imagem**: `ghcr.io/thirdkeyai/symbi:latest`
- **Propósito**: Container tudo-em-um com parsing DSL, runtime de agentes e servidor MCP
- **Tamanho**: ~80MB (inclui vector DB e suporte a API HTTP)
- **CLI**: Comando unificado `symbi` com subcomandos para diferentes operações

## Início Rápido

### Usando Imagem Pré-construída

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

# Executar com API HTTP
docker run --rm -p 8080:8080 \
  ghcr.io/thirdkeyai/symbi:latest \
  up --http-bind 0.0.0.0:8080
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
  -p 3000:3000 \
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
- `RUST_LOG` - Definir nível de log (debug, info, warn, error)
- `SYMBIONT_VECTOR_BACKEND` - Backend de vetores: `lancedb` (padrão) ou `qdrant`
- `QDRANT_URL` - URL do banco de dados vetorial Qdrant (apenas se usar backend Qdrant opcional)

### Montagem de Volumes

```bash
# Montar definições de agentes
-v $(pwd)/agents:/var/lib/symbi/agents

# Montar configuração
-v $(pwd)/config:/etc/symbi

# Montar diretório de dados
-v symbi-data:/var/lib/symbi/data
```

## Exemplo com Docker Compose

Por padrão, o Symbiont usa **LanceDB** como banco de dados vetorial embarcado -- sem necessidade de serviços externos. Se você precisar de um backend vetorial distribuído para implantações em escala, pode opcionalmente adicionar o Qdrant.

### Mínimo (LanceDB padrão -- sem necessidade de Qdrant)

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbi/agents
      - ./config:/etc/symbi
      - symbi-data:/var/lib/symbi/data
    environment:
      - RUST_LOG=info
    command: ["up", "--http-bind", "0.0.0.0:8080"]

volumes:
  symbi-data:
```

### Com Backend Qdrant Opcional

```yaml
version: '3.8'

services:
  symbi:
    image: ghcr.io/thirdkeyai/symbi:latest
    ports:
      - "8080:8080"
      - "3000:3000"
    volumes:
      - ./agents:/var/lib/symbi/agents
      - ./config:/etc/symbi
      - symbi-data:/var/lib/symbi/data
    environment:
      - RUST_LOG=info
      - SYMBIONT_VECTOR_BACKEND=qdrant
      - QDRANT_URL=http://qdrant:6334
    depends_on:
      - qdrant
    command: ["up", "--http-bind", "0.0.0.0:8080"]

  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - qdrant-data:/qdrant/storage

volumes:
  symbi-data:
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

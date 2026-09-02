# Helm chart — Symbiont runtime

Deploys the OSS `symbi` runtime. Minimal on purpose: the runtime, its Cedar
policies, and its secrets. No database, no console, no ingress assumptions.

```bash
helm install symbi ./symbi
```

## Supplying policies

The runtime is **fail-closed**: with no policy present, every tool call is
denied. That is the safe default, not a misconfiguration. Policies go in
`values.yaml` and are mounted at `/app/policies`:

```yaml
policies:
  default.cedar: |
    permit(principal, action == Action::"respond", resource);
    permit(principal, action == Action::"terminate", resource);
```

Changing a policy rolls the pods — the Deployment carries a checksum of the
policy set, so an edit is picked up rather than silently ignored.

## Model provider

Secrets become environment variables:

```yaml
env:
  OPENAI_API_KEY: sk-...
```

Or point at a model you run yourself, with no cloud key:

```yaml
env:
  OPENAI_API_KEY: ollama          # any non-empty value
config:
  OPENAI_BASE_URL: http://ollama.default.svc.cluster.local:11434/v1
  CHAT_MODEL: llama3.1
```

Bring your own Secret instead with `existingSecret: my-secret`.

## Probes

Liveness and readiness use `/api/v1/health/live` and `/api/v1/health/ready`,
both served by the runtime API on port 8080.

## Hardening

The pod runs as non-root with a read-only root filesystem and all capabilities
dropped. Writable `emptyDir` mounts are provided for `/app/.symbiont` (the
audit journal) and `/tmp`; add your own volume if you need the journal to
outlive the pod.

# Governed tool — argument contracts and the Cedar gate

Runs offline. No API key, no model, no network.

A `.clad.toml` declares what a tool accepts and how it runs. The runtime
validates arguments against that contract and names the Cedar action that
authorizes the call — all before a command line exists.

## Run it

```bash
cd examples/governed-tool
symbi tools validate
symbi tools test wordcount --arg mode=words --arg path=README.md
```

```
wordcount                                OK

  Command:   wc --words README.md
  Cedar:     Tool::Wordcount / execute_tool

  [dry run — command not executed]
```

## What the contract refuses

**A value outside the closed set.** `mode` is an `enum` with
`allowed = ["words", "lines", "chars"]`, so the model cannot invent a flag:

```bash
symbi tools test wordcount --arg mode=exec --arg path=README.md
```

```
  [dry run — validation failed, command not constructed]
```

**A value that fails its pattern.** `path` must match `^[A-Za-z0-9._-]+$`,
which admits a plain filename and nothing with a path separator:

```bash
symbi tools test wordcount --arg mode=words --arg path=../../etc/passwd
```

```
  [dry run — validation failed, command not constructed]
```

In both cases the command is *not constructed*. That ordering is the point:
there is no window in which a rejected argument reaches a shell.

## One thing worth knowing

`allowed` is only checked for `type = "enum"`. A `type = "string"` argument
with an `allowed` list is silently unconstrained — the manifest looks
restrictive and enforces nothing. Use `enum` when you mean a closed set.

## Making it executable

The dry run stops short of running the command. To let an agent actually call
this tool, permit its Cedar action in `policies/`:

```cedar
permit(principal, action == Action::"tool_call::wordcount", resource);
```

Without that the gate denies the call — see [`../policy-denial`](../policy-denial).

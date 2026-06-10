# Symbiont DSL Specification

> This specification reflects the actual grammar in
> [`crates/dsl/tree-sitter-symbiont/grammar.js`](https://github.com/thirdkeyai/symbiont/blob/main/crates/dsl/tree-sitter-symbiont/grammar.js), which is the source of truth. For a practical,
> example-driven walkthrough see the [DSL Guide](dsl-guide.md). Agent files use the
> `.symbi` extension (legacy `.dsl` is still recognized). Validate any file with
> `symbi dsl -f agents/<name>.symbi`.

## Overview

The Symbiont DSL is a declarative language for defining governed agents, their
policies, supporting types, and runtime integrations (schedules, channels,
memory, webhooks). A program is a sequence of top-level items.

## Top-level items

A program is any number of:

- `metadata { ... }` — document metadata
- `agent name(params) -> Type { ... }` — an agent definition
- `policy name { ... }` — a named policy
- `type Name = ...` — a type alias or struct
- `function name(params) -> Type { ... }` — a free function
- `schedule name { ... }` — a cron schedule
- `channel name { ... }` — an inter-agent channel
- `memory name { ... }` — a persistent memory store
- `webhook name { ... }` — webhook ingestion
- comments

## Metadata

```
metadata {
    version = "1.0.0"
    author = "ThirdKey"
    description = "..."
    tags = ["category", "use-case"]
}
```

Each pair is `identifier` `=` (or `:`) `value`, where a value is a string,
number, boolean, identifier, array, or record. Managed-CLI (Mode B) agents also
recognize `executor`, `model`, `allowed_tools`, and `system_prompt` — see the
[DSL Guide](dsl-guide.md).

## Agent definition

```
agent name(param: Type, ...) -> ReturnType {
    capabilities = ["read", "analyze"]

    policy policy_name {
        allow: read(input) if true
        deny: write(any)
    }

    with memory = "ephemeral", sandbox = "tier1" {
        result = process(input);
        return result;
    }
}
```

The parameter list and `-> ReturnType` are optional. The only items allowed
inside an agent body are: `capabilities`, `policy`, `function`, `with` blocks,
and comments.

### Capabilities

```
capabilities = ["read", "write", "analyze"]
```

An array of strings (the separator may be `=` or `:`).

## Policies

```
policy name {
    allow:   <expr> [if <expr>]
    deny:    <expr> [if <expr>]
    require: <expr> [if <expr>]
    audit:   <expr> [if <expr>]
}
```

Each rule is one of `allow` / `deny` / `require` / `audit`, then `:`, then an
expression, then an optional `if <expr>` guard. Policies may also appear at the
top level and inside `channel` blocks.

## `with` blocks

```
with memory = "persistent", sandbox = "tier1", timeout = "30m" {
    // statements
}
```

Zero or more `identifier = value` (or array) attributes, comma-separated,
followed by a block. Attributes the runtime understands include `sandbox`
(`docker`/`tier1`, `gvisor`/`tier2`, `firecracker`/`tier3`, `e2b`), `timeout`,
`memory`, `security`, and others; unrecognized attributes parse but are ignored.

## Functions

```
function name(param: Type) -> Type {
    // statements
}
```

## Types

Built-in types: `String`, `int`, `float`, `bool`. Generics use angle brackets
and may take multiple arguments: `Map<K, V>`, `Result<T, E>`. User-defined types:

```
type FileInfo = {
    path: String,
    size: int,
}

type AgentId = String   // alias
```

## Statements

Statements appear inside blocks. **Most statements must end with `;`**:

- `let x = <expr>;`
- assignment: `x = <expr>;` (also `+=`, `-=`, `*=`, `/=`, `%=`; the left side may be a member/index expression)
- `return <expr>;` (the expression is optional)
- expression statement: `<expr>;`
- `if <expr> { ... } else { ... }` — also `if let pattern = <expr> { ... }`; `else` may chain another `if` or a block
- `for x in <expr> { ... }`
- `match <expr> { pattern => result, _ => result }`
- `try { ... } catch (e) { ... }` (one or more `catch` clauses; a catch binds an identifier)

A block may end with a trailing expression (no `;`), which becomes the block's
value. There is no `while` loop.

## Expressions

Precedence, lowest to highest: `||`, `&&`, equality (`==`, `!=`, `in`),
comparison (`<`, `>`, `<=`, `>=`), additive (`+`, `-`), multiplicative
(`*`, `/`, `%`), unary (`!`, `not`, `-`), postfix, primary.

- **Boolean operators are `&&` and `||`** — `and`/`or` are not keywords.
  Negation is `!` or `not`.
- Postfix: member access `a.b`, function call `f(x, y)`, indexing `a[i]`.
- Calls accept positional and named arguments: `f(a, name = b)` (named-arg
  separator may be `=` or `:`).
- Record / struct literals: `TypeName { field: expr, ... }` or `{ field: expr }`.
- Lambda: `x => expr`.
- `if <expr> { ... } else { ... }` is also usable as an expression.
- Grouping `( <expr> )`, arrays `[a, b]`, and vault references `vault://path/to/secret`.

## Literals

- **String**: `"..."` with `\` escapes. (No string interpolation.)
- **Number**: `42`, `1_000`, `3.14`.
- **Boolean**: `true`, `false`.
- **Duration**: `30s`, `5m`, `2h`, `7d`, `1w`, `6months`, `1y`, and the
  `N.seconds` / `N.minutes` / `N.hours` forms.
- **Identifier**: `[a-zA-Z_][a-zA-Z0-9_]*`.

## Comments

Both `//` and `#` introduce line comments. (There is no block-comment syntax.)

## Notes

- The grammar file is the authoritative reference; this document summarizes it.
- For real, parse-tested examples see the agents under
  [`agents/`](https://github.com/thirdkeyai/symbiont/tree/main/agents) and the
  [DSL Guide](dsl-guide.md).

# Symbiont
by Thirdkey

Symbiont is a next-generation programming language and agent framework built for AI-native, privacy-first software.
It empowers developers to build autonomous, policy-aware agents that can safely collaborate with humans, other agents, and large language models — all while enforcing zero-trust security, data privacy, and provable behavior using homomorphic encryption and zero-knowledge proofs.

With Symbiont, you're not just writing code — you're deploying intelligent, verifiable systems that explain, justify, and protect every decision they make.

---

## 📐 Symbiont DSL: EBNF Grammar (v1 Draft)

```ebnf
Program         ::= { AgentDefinition }

AgentDefinition ::= "agent" Identifier AgentSignature? MetadataBlock CodeBlock

AgentSignature  ::= "(" [ ParameterList ] ")" "->" Type

ParameterList   ::= Parameter { "," Parameter }
Parameter       ::= Identifier ":" Type

MetadataBlock   ::= "with" MetadataEntry { "," MetadataEntry }

MetadataEntry   ::= Identifier "=" Literal

CodeBlock       ::= "{" StatementList "}"

StatementList   ::= { Statement }

Statement       ::= "return" Expression ";"
                 | Assignment
                 | FunctionCall ";"
                 | IfStatement
                 | Block

Assignment      ::= Identifier "=" Expression ";"

FunctionCall    ::= Identifier "(" [ ArgumentList ] ")"

ArgumentList    ::= Expression { "," Expression }

IfStatement     ::= "if" "(" Expression ")" CodeBlock [ "else" CodeBlock ]

Block           ::= "{" StatementList "}"

Expression      ::= Identifier
                 | Literal
                 | FunctionCall

Type            ::= Identifier
Literal         ::= String | Number | Boolean

Identifier      ::= /[a-zA-Z_][a-zA-Z0-9_]*/
String          ::= "\"" { any character except "\"" } "\""
Number          ::= /[0-9]+/
Boolean         ::= "true" | "false"
```

---

## 🔍 DSL Example (Using This Grammar)

```symbiont
agent analyze_health(input: HealthData) -> Result {
  with memory = "ephemeral", privacy = "medical", requires = "moderator_approval" {
    
    if (llm_check_safety(input)) {
        return analyze(input);
    } else {
        return reject();
    }
  }
}
```

---

## 🛠️ Parser Prototype in Python

We'll use **`lark`**, a modern Python parsing toolkit that supports EBNF and AST transformation.

---

### 📦 Step 1: Install Lark

```bash
pip install lark
```

---

### 🧪 Step 2: `symbiont_parser.py`

```python
from lark import Lark, Transformer, v_args

symbiont_grammar = r"""
    start: program
    program: agent_def+

    agent_def: "agent" CNAME agent_sig? metadata_block code_block

    agent_sig: "(" [param_list] ")" "->" type
    param_list: param ("," param)*
    param: CNAME ":" type

    metadata_block: "with" metadata_entry ("," metadata_entry)*
    metadata_entry: CNAME "=" literal

    code_block: "{" statement* "}"

    statement: "return" expr ";"              -> return_stmt
             | CNAME "=" expr ";"             -> assign
             | func_call ";"                  -> call
             | "if" "(" expr ")" code_block ["else" code_block] -> if_stmt

    func_call: CNAME "(" [expr_list] ")"
    expr_list: expr ("," expr)*

    expr: func_call
        | CNAME
        | literal

    type: CNAME

    literal: ESCAPED_STRING -> string
           | SIGNED_NUMBER  -> number
           | "true"         -> true
           | "false"        -> false

    %import common.CNAME
    %import common.ESCAPED_STRING
    %import common.SIGNED_NUMBER
    %import common.WS
    %ignore WS
"""

parser = Lark(symbiont_grammar, start="start", parser="lalr")

code = """
agent analyze_health(input: HealthData) -> Result {
  with memory = "ephemeral", privacy = "medical", requires = "moderator_approval" {
    if (llm_check_safety(input)) {
        return analyze(input);
    } else {
        return reject();
    }
  }
}
"""

tree = parser.parse(code)
print(tree.pretty())
```

---

### 🧠 Step 3: Optionally Add AST Transformer

If you want to compile to Python or Rust, define a `Transformer` to walk the parse tree and emit code.

---

## ✅ What You Now Have

* A **formal grammar** for Symbiont DSL
* A **working Python parser prototype** using Lark
* The ability to turn Symbiont DSL into ASTs — ready for transpilation, interpretation, or compilation

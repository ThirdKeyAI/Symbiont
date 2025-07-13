# How AST Works with LLMs

### 1. The "Model API" for Generating Code via AST

You wouldn't use a standard code-generation API like those from OpenAI or other providers in a raw way. Instead, Symbiont treats the LLM as a **reasoning engine within a larger, structured system.** The "API" is the Symbiont DSL itself. The process works as follows:

1.  **Goal to Plan:** An agent is given a high-level goal (e.g., "Add a data validation function"). The LLM, guided by the context from the RAG engine, creates a high-level plan.
2.  **Plan to DSL:** For each step in the plan, the LLM's task is to generate the specific **Symbiont DSL code** required to execute that step. It's not outputting raw Python or Rust; it's outputting instructions like `create_function`, `add_parameter`, or `insert_if_statement` using the syntax defined in your DSL.
3.  **DSL to AST:** The Symbiont Core Engine receives this DSL code, parses it with Tree-sitter, and validates it against the system's policies.
4.  **AST to Code:** The engine then executes these validated instructions by directly manipulating the AST of the target codebase, ensuring the change is precise and syntactically perfect.

So, the LLM's direct output is **Symbiont DSL**, which in turn commands the Core Engine to modify the AST. This provides a crucial layer of safety and abstraction.

### 2. How It Works on a New, Empty Project

This process is perfectly suited for starting from scratch. Here’s how an agent would build a new project in an empty repository:

1.  **Initial Goal:** The process starts with a high-level goal provided by the user, for example: "Create a simple web server in Python that responds with 'Hello, World!' on the root endpoint."

2.  **Bootstrapping Plan:** The Coder Agent, powered by the LLM, would first create a plan.
    * *Plan Step 1:* Create a new file named `main.py`.
    * *Plan Step 2:* Add the necessary imports (e.g., `Flask`).
    * *Plan Step 3:* Define the main application instance.
    * *Plan Step 4:* Create a function to handle requests to the `/` route.
    * *Plan Step 5:* Add the main execution block to run the server.

3.  **Incremental, AST-Based Creation:** The agent then executes this plan step-by-step, generating Symbiont DSL commands for each action:
    * It would first issue a DSL command to **create a new file**.
    * Next, it would generate DSL commands to **insert import nodes** into the file's AST.
    * Then, it would generate a command to **insert a function definition node** into the AST, complete with the correct name, parameters, and a body containing a return statement.

This process continues incrementally. The agent isn't generating a whole file at once; it is **building the code's structure piece by piece via the AST**, just as a human developer would. It would then likely delegate to a Tester Agent to run the code and verify the output, creating a continuous loop of building, testing, and refining until the initial goal is met.
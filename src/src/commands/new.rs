use clap::ArgMatches;
use std::fs;
use std::path::Path;

const TEMPLATES: &[(&str, &str)] = &[
    ("webhook-min", "Minimal webhook handler with bearer token auth + JSON echo"),
    ("webscraper-agent", "Web scraper tool + guard policy + sample prompt"),
    ("slm-first", "Router + SLM allow-list + confidence fallback"),
    ("rag-lite", "Qdrant + ingestion scripts + search agent"),
];

pub async fn run(matches: &ArgMatches) {
    if matches.get_flag("list") {
        list_templates();
        return;
    }

    let template = matches.get_one::<String>("template").unwrap();
    let project_name = matches
        .get_one::<String>("name")
        .map(|s| s.as_str())
        .unwrap_or(template);

    if !is_valid_template(template) {
        eprintln!("✗ Unknown template: {}", template);
        eprintln!("\nAvailable templates:");
        list_templates();
        std::process::exit(1);
    }

    println!("🔧 Creating project '{}' from template '{}'...", project_name, template);

    // Create project directory
    if Path::new(project_name).exists() {
        eprintln!("✗ Directory '{}' already exists", project_name);
        std::process::exit(1);
    }

    fs::create_dir(project_name).expect("Failed to create project directory");

    // Generate template based on type
    match template.as_str() {
        "webhook-min" => create_webhook_min_template(project_name),
        "webscraper-agent" => create_webscraper_agent_template(project_name),
        "slm-first" => create_slm_first_template(project_name),
        "rag-lite" => create_rag_lite_template(project_name),
        _ => unreachable!(),
    }

    println!("\n✅ Project created successfully!");
    println!("\n📝 Next steps:");
    println!("  cd {}", project_name);
    println!("  symbi up");
    println!("  # Follow instructions in README.md");
}

fn list_templates() {
    for (name, description) in TEMPLATES {
        println!("  {} - {}", name, description);
    }
}

fn is_valid_template(template: &str) -> bool {
    TEMPLATES.iter().any(|(name, _)| *name == template)
}

fn create_webhook_min_template(project_name: &str) {
    let base = Path::new(project_name);

    // Create directory structure
    fs::create_dir_all(base.join("agents")).unwrap();
    fs::create_dir_all(base.join("policies")).unwrap();
    fs::create_dir_all(base.join("tests")).unwrap();

    // Create agent
    fs::write(
        base.join("agents/webhook_handler.dsl"),
        r#"agent webhook_handler {
    name: "Webhook Handler"
    description: "Handles incoming webhook requests"

    on_http_post {
        validate: bearer_token
        parse: json

        response {
            status: 200
            body: {
                "received": true,
                "echo": request.body
            }
        }
    }
}
"#,
    ).unwrap();

    // Create policy
    fs::write(
        base.join("policies/webhook_policy.dsl"),
        r#"policy webhook_policy {
    name: "Webhook Security Policy"

    allow http_post {
        where: has_bearer_token()
        where: content_type == "application/json"
    }

    deny {
        where: request_size > 1MB
        message: "Request body too large"
    }

    rate_limit {
        requests: 100
        per: "1 minute"
    }
}
"#,
    ).unwrap();

    // Create test script
    fs::write(
        base.join("tests/webhook_test.sh"),
        r#"#!/bin/bash
# Test webhook handler

echo "Testing webhook..."
curl -X POST \
  -H "Authorization: Bearer dev" \
  -H "Content-Type: application/json" \
  -d '{"ping":"pong"}' \
  http://localhost:8081/webhook

echo -e "\n\nTest complete!"
"#,
    ).unwrap();

    // Make test script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(base.join("tests/webhook_test.sh")).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(base.join("tests/webhook_test.sh"), perms).unwrap();
    }

    // Create config
    fs::write(
        base.join("symbi.toml"),
        r#"[runtime]
mode = "dev"
hot_reload = true

[http]
enabled = true
port = 8081
dev_token = "dev"

[logging]
level = "info"
format = "pretty"
"#,
    ).unwrap();

    // Create README
    fs::write(
        base.join("README.md"),
        &format!(r#"# {}

Minimal webhook handler with bearer token authentication and JSON echo.

## Getting Started

1. Start the runtime:
   ```bash
   symbi up
   ```

2. Test the webhook:
   ```bash
   ./tests/webhook_test.sh
   ```

   Or manually:
   ```bash
   curl -X POST \
     -H "Authorization: Bearer dev" \
     -H "Content-Type: application/json" \
     -d '{{"test":"data"}}' \
     http://localhost:8081/webhook
   ```

## Project Structure

- `agents/webhook_handler.dsl` - Webhook agent definition
- `policies/webhook_policy.dsl` - Security policy
- `tests/webhook_test.sh` - Integration test
- `symbi.toml` - Runtime configuration

## Next Steps

- Modify the agent to add custom logic
- Adjust the policy for your security requirements
- Change the dev token for production use

## Documentation

See https://docs.symbi.sh for full documentation.
"#, project_name),
    ).unwrap();
}

fn create_webscraper_agent_template(project_name: &str) {
    let base = Path::new(project_name);

    fs::create_dir_all(base.join("agents")).unwrap();
    fs::create_dir_all(base.join("policies")).unwrap();
    fs::create_dir_all(base.join("tests")).unwrap();

    fs::write(
        base.join("agents/scraper.dsl"),
        r#"agent webscraper {
    name: "Web Scraper"
    description: "Scrapes and extracts content from URLs"

    tools: [http.fetch]

    on_http_post {
        validate: bearer_token
        parse: json

        action {
            url = request.body.url
            content = http.fetch(url)

            prompt: |
                Extract the main content from this webpage and summarize it in 3 bullet points:

                {{ content }}

            response {
                status: 200
                body: {
                    "url": url,
                    "summary": ai_response
                }
            }
        }
    }
}
"#,
    ).unwrap();

    fs::write(
        base.join("policies/scraper_policy.dsl"),
        r#"policy scraper_policy {
    name: "Web Scraper Security Policy"

    allow http.fetch {
        where: url.starts_with("https://")
        where: !url.contains("localhost")
        where: !is_private_ip(url)
    }

    rate_limit {
        requests: 20
        per: "1 minute"
    }
}
"#,
    ).unwrap();

    fs::write(
        base.join("README.md"),
        &format!(r#"# {}

Web scraper agent with URL validation and content extraction.

## Features

- HTTP fetch tool with rate limiting
- URL validation policy (HTTPS only, no private IPs)
- AI-powered content summarization

## Getting Started

```bash
symbi up

curl -X POST \
  -H "Authorization: Bearer dev" \
  -H "Content-Type: application/json" \
  -d '{{"url":"https://example.com"}}' \
  http://localhost:8081/webhook
```

## Documentation

See https://docs.symbi.sh for full documentation.
"#, project_name),
    ).unwrap();

    fs::write(base.join("symbi.toml"), r#"[runtime]
mode = "dev"
"#).unwrap();
}

fn create_slm_first_template(project_name: &str) {
    let base = Path::new(project_name);

    fs::create_dir_all(base.join("agents")).unwrap();
    fs::create_dir_all(base.join("routing")).unwrap();

    fs::write(
        base.join("agents/code_helper.dsl"),
        r#"agent code_helper {
    name: "Code Helper"
    description: "SLM-first coding assistant with LLM fallback"

    router {
        slm: "llama-3.2-3b"
        llm: "gpt-4"
        strategy: "confidence"
    }

    on_prompt {
        if confidence < 0.7 {
            use: llm
        } else {
            use: slm
        }

        response {
            body: {
                "model_used": model_name,
                "confidence": confidence_score,
                "response": ai_response
            }
        }
    }
}
"#,
    ).unwrap();

    fs::write(
        base.join("routing/config.toml"),
        r#"[router]
strategy = "confidence"

[slm]
model = "llama-3.2-3b"
confidence_threshold = 0.7
tasks = ["classify", "summarize", "extract", "simple_code"]

[llm]
model = "gpt-4o-mini"
fallback_on_low_confidence = true
tasks = ["complex_reasoning", "refactoring"]
"#,
    ).unwrap();

    fs::write(
        base.join("README.md"),
        &format!(r#"# {}

SLM-first coding assistant with intelligent routing and LLM fallback.

## Features

- Uses Llama 3.2 3B for simple tasks (fast, cost-effective)
- Falls back to GPT-4 for complex reasoning
- Confidence-based routing
- Cost tracking and metrics

## Getting Started

```bash
symbi up

curl -X POST \
  -H "Authorization: Bearer dev" \
  -H "Content-Type: application/json" \
  -d '{{"prompt":"Write a function to check if a number is prime"}}' \
  http://localhost:8081/webhook
```

## Documentation

See https://docs.symbi.sh for full documentation.
"#, project_name),
    ).unwrap();

    fs::write(base.join("symbi.toml"), r#"[runtime]
mode = "dev"
"#).unwrap();
}

fn create_rag_lite_template(project_name: &str) {
    let base = Path::new(project_name);

    fs::create_dir_all(base.join("agents")).unwrap();
    fs::create_dir_all(base.join("scripts")).unwrap();
    fs::create_dir_all(base.join("docs")).unwrap();

    fs::write(
        base.join("agents/doc_search.dsl"),
        r#"agent doc_search {
    name: "Document Search"
    description: "RAG agent for searching documentation"

    vector_db: "qdrant"
    collection: "docs"

    on_query {
        embed: query
        search: {
            collection: "docs",
            top_k: 5,
            score_threshold: 0.7
        }

        prompt: |
            Answer the question using these documents:

            {{ search_results }}

            Question: {{ query }}

        response {
            body: {
                "answer": ai_response,
                "sources": search_results.sources
            }
        }
    }
}
"#,
    ).unwrap();

    fs::write(
        base.join("scripts/ingest_docs.sh"),
        r#"#!/bin/bash
# Ingest documentation into Qdrant

echo "Ingesting documents..."

for file in docs/*.md; do
    echo "Processing $file..."
    # TODO: Implement ingestion logic
done

echo "Ingestion complete!"
"#,
    ).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(base.join("scripts/ingest_docs.sh")).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(base.join("scripts/ingest_docs.sh"), perms).unwrap();
    }

    fs::write(
        base.join("docs/sample.md"),
        r#"# Sample Documentation

This is a sample document for testing RAG functionality.

## Features

- Vector similarity search
- Hybrid search capabilities
- Automatic relevance scoring

## Usage

Query the documentation by sending POST requests to the webhook endpoint.
"#,
    ).unwrap();

    fs::write(
        base.join("README.md"),
        &format!(r#"# {}

RAG (Retrieval-Augmented Generation) agent for searching documentation.

## Features

- Qdrant vector database integration
- Document ingestion scripts
- Hybrid search with relevance scoring
- Sample documents included

## Getting Started

1. Start Qdrant:
   ```bash
   docker run -p 6333:6333 qdrant/qdrant
   ```

2. Ingest documents:
   ```bash
   ./scripts/ingest_docs.sh
   ```

3. Start Symbiont:
   ```bash
   symbi up
   ```

4. Query the documentation:
   ```bash
   curl -X POST \
     -H "Authorization: Bearer dev" \
     -H "Content-Type: application/json" \
     -d '{{"query":"What are the features?"}}' \
     http://localhost:8081/webhook
   ```

## Documentation

See https://docs.symbi.sh for full documentation.
"#, project_name),
    ).unwrap();

    fs::write(
        base.join("symbi.toml"),
        r#"[runtime]
mode = "dev"

[vector]
provider = "qdrant"
host = "localhost"
port = 6333
collection_name = "docs"
"#,
    ).unwrap();
}

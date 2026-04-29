metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Multi-purpose NLP text analysis with LLM integration"
    tags = ["nlp", "text-processing", "analysis", "sentiment", "entities"]
}

agent nlp_processor(text: String, tasks: Array<String>) -> NLPResults {
    capabilities = ["text_analysis", "sentiment_analysis", "entity_extraction", "summarization"]

    policy text_processing {
        allow: ["read_text", "analyze_text", "extract_entities", "generate_summary"]
            if text.length <= 50000
        allow: "use_llm"
            if tasks.contains("sentiment") || tasks.contains("summary")
        deny: ["store_text", "network_access", "file_access"]
            if text.contains_pii == true

        require: {
            pii_detection: true,
            input_validation: true,
            rate_limiting: "1000/hour",
            token_validation: true  // Validate LLM API tokens
        }

        audit: {
            log_level: "info",
            include_input: false,  // Protect PII
            include_output: false,  // Protect analyzed content
            include_metadata: true,
            include_task_list: true,
            content_filtering: true
        }
    }

    with
        memory = "ephemeral",
        privacy = "high",  // Elevated from medium
        security = "high",
        sandbox = "Tier1",
        timeout = 30000,
        max_memory_mb = 1024,
        max_cpu_cores = 2.0
    {
        try {
            // Validate input
            if text.is_empty() {
                return error("Input text cannot be empty");
            }

            // PII detection before processing
            if detect_pii(text) {
                log("WARNING", "PII detected in input text");
                // Optionally redact or reject
            }

            results = {};

            for task in tasks {
                match task {
                    "sentiment" => {
                        results.sentiment = analyze_sentiment(text);
                    },
                    "entities" => {
                        results.entities = extract_entities(text);
                    },
                    "summary" => {
                        // Use LLM with Vault-stored API key
                        let api_key = vault://llm/openai/api_key;
                        results.summary = summarize_with_llm(text, api_key);
                    },
                    "keywords" => {
                        results.keywords = extract_keywords(text);
                    },
                    "language" => {
                        results.language = detect_language(text);
                    },
                    _ => {
                        log("WARNING", "Unknown task type: " + task);
                    }
                }
            }

            log("INFO", "NLP analysis completed: " + tasks.length + " tasks");
            return results;

        } catch (error) {
            log("ERROR", "NLP processing failed: " + error.message);
            return error("Processing failed: " + error.message);
        }
    }
}

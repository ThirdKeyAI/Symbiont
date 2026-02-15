metadata {
    version = "1.4.0"
    author = "Symbiont Community"
    description = "Knowledge curator with persistent memory and hybrid search"
    tags = ["memory", "knowledge-base", "search", "rag"]
}

// Persistent markdown-backed memory with hybrid search
memory knowledge_store {
    store     markdown
    path      "data/knowledge"
    retention 365d
    search {
        vector_weight  0.6
        keyword_weight 0.4
    }
}

agent knowledge_curator(query: String, context: JSON) -> SearchResult {
    capabilities = ["memory_read", "memory_write", "text_analysis", "summarization"]

    policy knowledge_guard {
        allow: ["memory_read", "memory_write", "summarize", "search"]
            if context.user.role == "editor" || context.user.role == "admin"
        allow: ["memory_read", "search"]
            if context.user.role == "viewer"
        deny: ["memory_write"] if context.user.role == "viewer"
        deny: ["execute_code", "network_access", "file_access"]

        require: {
            input_validation: true,
            content_moderation: true,
            max_document_size: "5MB",
            rate_limiting: "200/hour"
        }

        audit: {
            log_level: "info",
            include_input: false,
            include_output: true,
            include_metadata: true,
            retention_days: 180
        }
    }

    with
        memory = "persistent",
        privacy = "high",
        security = "high",
        sandbox = "Tier1",
        timeout = 15000,
        max_memory_mb = 1024,
        max_cpu_cores = 1.0
    {
        // Determine intent: store, search, or summarize
        let intent = classify_intent(query);

        if intent == "store" {
            // Extract key facts and store in memory
            let facts = extract_facts(context.document);
            let tags = extract_tags(context.document);

            memory_write(knowledge_store, {
                "content": context.document,
                "facts": facts,
                "tags": tags,
                "source": context.source,
                "author": context.user.name,
                "timestamp": now()
            });

            return {
                "action": "stored",
                "facts_extracted": length(facts),
                "tags": tags
            };
        }

        if intent == "search" {
            // Hybrid search: vector similarity + keyword matching
            let results = memory_search(knowledge_store, query, limit: 10);

            // Re-rank by relevance
            let ranked = rerank(results, query);

            return {
                "action": "search",
                "results": ranked,
                "total_matches": length(results)
            };
        }

        if intent == "summarize" {
            // Retrieve relevant documents and synthesize
            let docs = memory_search(knowledge_store, query, limit: 20);
            let summary = synthesize(docs, query);

            return {
                "action": "summary",
                "summary": summary,
                "sources": length(docs)
            };
        }

        return { "action": "unknown", "error": "Unrecognized intent" };
    }
}

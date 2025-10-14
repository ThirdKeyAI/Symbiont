metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Multi-purpose NLP text analysis agent"
    tags = ["nlp", "text-processing", "analysis"]
}

agent nlp_processor(text: String, tasks: Array<String>) -> NLPResults {
    capabilities = ["text_analysis", "sentiment_analysis", "entity_extraction", "summarization"]
    
    policy text_processing {
        allow: read(text) if text.length <= 50000
        allow: use("llm") if tasks.contains("sentiment") || tasks.contains("summary")
        deny: store(text) if text.contains_pii == true
        audit: all_operations with content_filtering
    }
    
    with memory = "ephemeral", privacy = "medium" {
        results = {};
        
        for task in tasks {
            match task {
                "sentiment" => results.sentiment = analyze_sentiment(text),
                "entities" => results.entities = extract_entities(text),
                "summary" => results.summary = summarize_text(text),
                "keywords" => results.keywords = extract_keywords(text),
                "language" => results.language = detect_language(text)
            }
        }
        
        audit_log("nlp_analysis_completed", {
            "tasks": tasks,
            "text_length": text.length,
            "results_count": results.keys().length
        });
        
        return results;
    }
}
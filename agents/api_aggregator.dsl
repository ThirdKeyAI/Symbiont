agent api_aggregator(sources: Array<APISource>, query: String) -> AggregatedData {
    capabilities = ["api_access", "data_aggregation", "response_merging"]
    
    policy api_access {
        allow: call(external_api) if api.rate_limit_ok && api.authenticated
        require: valid_api_keys for all_sources
        deny: store(response) if response.contains_personal_data
        audit: api_calls with response_metadata
    }
    
    with timeout = 30.seconds, retry_policy = "exponential_backoff" {
        results = [];
        
        for source in sources {
            try {
                response = call_api(source.endpoint, {
                    "query": query,
                    "api_key": resolve_secret(source.auth_key),
                    "format": "json"
                });
                
                standardized_data = normalize_response(response, source.schema);
                results.append({
                    "source": source.name,
                    "data": standardized_data,
                    "timestamp": now()
                });
                
            } catch (APIError e) {
                results.append({
                    "source": source.name,
                    "error": e.message,
                    "timestamp": now()
                });
            }
        }
        
        return AggregatedData {
            query: query,
            sources_queried: sources.length,
            successful_responses: results.filter(r => r.data != null).length,
            aggregated_results: merge_results(results)
        };
    }
}
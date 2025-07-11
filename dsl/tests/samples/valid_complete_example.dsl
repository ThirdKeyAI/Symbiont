// Complete Symbiont DSL example
metadata {
    version: "1.0",
    author: "AI Assistant",
    description: "Complete DSL demonstration"
}

agent DataProcessor {
    capabilities: [read, write, transform]
    
    policy ProcessingPolicy {
        allow: read(data_source)
        require: validate(input)
        deny: delete(critical_data)
    }
    
    function process_data(input: String) -> Result<String> {
        let validated = validate(input);
        if validated {
            return transform(input);
        } else {
            return error("Invalid input");
        }
    }
}

type DataSource = {
    url: String,
    format: String,
    credentials: Option<String>
}

agent APIGateway {
    capabilities: [route, authenticate, log]
    
    policy SecurityPolicy {
        require: authenticate(request)
        allow: route(authenticated_request)
        audit: log(all_requests)
    }
}
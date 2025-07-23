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
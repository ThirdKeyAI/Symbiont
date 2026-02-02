agent code_runner(script: String) -> Output {
    with sandbox = "e2b", timeout = 60.seconds {
        return execute(script);
    }
}

agent data_processor(input: DataSource) -> ProcessedData {
    with sandbox = "docker" {
        let validated = validate(input);
        if validated {
            return transform(input);
        } else {
            return error("Invalid input");
        }
    }
}

agent secure_analyzer(data: String) -> AnalysisResult {
    with sandbox = "firecracker", timeout = 120.seconds {
        return analyze_securely(data);
    }
}
metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Multi-format file converter with security scanning"
    tags = ["conversion", "file-processing", "transformation", "security"]
}

agent format_converter(input_file: File, target_format: String) -> ConversionResult {
    capabilities = ["file_processing", "format_conversion", "data_transformation"]

    policy file_conversion {
        allow: ["read_file", "parse_format", "convert_format", "write_output"]
            if input_file.size <= 100_000_000  // 100MB
        allow: "write_output" if target_format in ["json", "csv", "xml", "yaml", "toml"]
        deny: ["execute_code", "network_access", "spawn_process"]
        deny: "process_file" if input_file.contains_executable_code

        require: {
            virus_scanning: true,
            file_type_validation: true,
            output_sanitization: true
        }

        audit: {
            log_level: "info",
            include_file_metadata: true,
            include_conversion_stats: true,
            include_input: false,  // Don't log file contents
            alert_on_suspicious_files: true
        }
    }

    with
        memory = "ephemeral",
        security = "high",
        sandbox = "Tier2",  // gVisor for untrusted file processing
        timeout = 60000,
        max_memory_mb = 2048,
        max_cpu_cores = 2.0
    {
        try {
            // Validate file safety
            if detect_malicious_content(input_file) {
                return ConversionResult {
                    success: false,
                    message: "File contains potentially malicious content",
                    error_details: "Security scan failed"
                };
            }

            source_format = detect_format(input_file);

            if source_format == target_format {
                return ConversionResult {
                    success: true,
                    message: "File already in target format",
                    output_file: input_file,
                    source_format: source_format,
                    target_format: target_format
                };
            }

            converted_data = convert_format(input_file, source_format, target_format);
            output_file = save_converted_file(converted_data, target_format);

            return ConversionResult {
                success: true,
                message: "Conversion completed successfully",
                output_file: output_file,
                source_format: source_format,
                target_format: target_format
            };

        } catch (ConversionError e) {
            log("ERROR", "Conversion failed: " + e.message);
            return ConversionResult {
                success: false,
                message: e.message,
                error_details: e.details
            };
        }
    }
}

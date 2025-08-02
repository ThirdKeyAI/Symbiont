agent format_converter(input_file: File, target_format: String) -> ConversionResult {
    capabilities = ["file_processing", "format_conversion", "data_transformation"]
    
    policy file_conversion {
        allow: read(input_file) if input_file.size <= 100MB
        allow: write(output) if target_format in ["json", "csv", "xml", "yaml"]
        deny: process(input_file) if input_file.contains_executable_code
        audit: file_operations with file_metadata
    }
    
    with memory = "ephemeral", security = "medium" {
        try {
            source_format = detect_format(input_file);
            
            if source_format == target_format {
                return ConversionResult {
                    success: true,
                    message: "File already in target format",
                    output_file: input_file
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
            audit_log("conversion_failed", e.details);
            return ConversionResult {
                success: false,
                message: e.message,
                error_details: e.details
            };
        }
    }
}
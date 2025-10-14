agent data_validator(data: DataSet, schema: ValidationSchema) -> ValidationReport {
    capabilities = ["data_validation", "schema_checking", "quality_assessment"]
    
    policy data_quality {
        allow: read(data) if data.source.trusted == true
        require: schema_validation for all_operations
        audit: validation_results with statistics
    }
    
    with memory = "ephemeral", privacy = "low" {
        report = ValidationReport {
            valid_records: 0,
            invalid_records: 0,
            errors: [],
            warnings: [],
            quality_score: 0.0
        };
        
        for record in data.records {
            validation_result = validate_against_schema(record, schema);
            
            if validation_result.valid {
                report.valid_records += 1;
            } else {
                report.invalid_records += 1;
                report.errors.append(validation_result.errors);
            }
        }
        
        report.quality_score = report.valid_records / (report.valid_records + report.invalid_records);
        
        return report;
    }
}
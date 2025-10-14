agent security_scanner(target: ScanTarget, scan_type: String) -> SecurityReport {
    capabilities = ["security_scanning", "vulnerability_detection", "compliance_checking"]
    
    policy security_scanning {
        allow: scan(target) if user.role == "security_analyst"
        require: [
            authorized_scan_request,
            target_ownership_verified,
            scan_scope_approved
        ]
        deny: scan(target) if target.classification == "production" && scan_type == "intrusive"
        audit: all_scan_activities with detailed_logs
    }
    
    with memory = "encrypted", privacy = "high", requires = "security_clearance" {
        report = SecurityReport {
            target: target.identifier,
            scan_type: scan_type,
            start_time: now(),
            vulnerabilities: [],
            compliance_status: {},
            risk_score: 0.0
        };
        
        match scan_type {
            "vulnerability" => {
                vulns = scan_vulnerabilities(target);
                report.vulnerabilities = vulns;
                report.risk_score = calculate_risk_score(vulns);
            },
            "compliance" => {
                report.compliance_status = check_compliance(target, ["SOC2", "HIPAA", "GDPR"]);
            },
            "comprehensive" => {
                report.vulnerabilities = scan_vulnerabilities(target);
                report.compliance_status = check_compliance(target, ["SOC2", "HIPAA", "GDPR"]);
                report.risk_score = calculate_comprehensive_risk(report);
            }
        }
        
        // Generate remediation recommendations
        report.recommendations = generate_remediation_plan(report);
        report.end_time = now();
        
        security_event("scan_completed", {
            "target": target.identifier,
            "scan_type": scan_type,
            "vulnerabilities_found": report.vulnerabilities.length,
            "risk_score": report.risk_score
        });
        
        return report;
    }
}
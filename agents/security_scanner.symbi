metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Security vulnerability scanner with compliance checking and CVSS scoring"
    tags = ["security", "scanning", "vulnerability", "compliance", "CVSS", "CWE"]
}

agent security_scanner(target: ScanTarget, scan_type: String) -> SecurityReport {
    capabilities = ["security_scanning", "vulnerability_detection", "compliance_checking", "cvss_scoring"]

    policy security_scanning {
        allow: ["scan_target", "read_files", "analyze_dependencies", "check_vulnerabilities"]
            if user.role == "security_analyst" && target.authorized == true
        deny: ["write_files", "modify_permissions", "execute_code", "network_access"]
        deny: "scan_target"
            if target.classification == "production" && scan_type == "intrusive"

        require: {
            rbac_authorization: true,
            scan_approval_required: true,
            target_ownership_verified: true,
            sandbox_tier: "Tier2",  // gVisor isolation for safety
            max_scan_depth: 10,
            cvss_scoring: true,
            cwe_classification: true
        }

        audit: {
            log_level: "warning",
            include_findings: true,
            include_cvss_scores: true,
            include_target_metadata: true,
            include_scan_parameters: true,
            alert_on_critical_findings: true,
            compliance_tags: ["OWASP", "CWE", "HIPAA", "SOC2"]
        }
    }

    with
        memory = "encrypted",
        privacy = "high",
        security = "high",
        sandbox = "Tier2",  // gVisor for untrusted target scanning
        timeout = 300000,  // 5 minutes
        max_memory_mb = 2048,
        max_cpu_cores = 2.0,
        requires = ["security_clearance", "scan_authorization"]
    {
        try {
            // Verify authorization
            if !verify_scan_authorization(user, target) {
                return error("Unauthorized scan attempt");
            }

            report = SecurityReport {
                target: target.identifier,
                scan_type: scan_type,
                start_time: now(),
                vulnerabilities: [],
                compliance_status: {},
                risk_score: 0.0,
                scanner_version: "1.0.0"
            };

            match scan_type {
                "vulnerability" => {
                    // Use CVE database from Vault
                    let cve_db_url = vault://security/cve_database_url;
                    vulns = scan_vulnerabilities(target, cve_db_url);
                    report.vulnerabilities = vulns;
                    report.risk_score = calculate_cvss_risk_score(vulns);
                },
                "compliance" => {
                    report.compliance_status = check_compliance(
                        target,
                        ["SOC2", "HIPAA", "GDPR", "PCI-DSS"]
                    );
                },
                "comprehensive" => {
                    let cve_db_url = vault://security/cve_database_url;
                    report.vulnerabilities = scan_vulnerabilities(target, cve_db_url);
                    report.compliance_status = check_compliance(
                        target,
                        ["SOC2", "HIPAA", "GDPR", "PCI-DSS"]
                    );
                    report.risk_score = calculate_comprehensive_risk(report);
                },
                _ => {
                    return error("Unknown scan type: " + scan_type);
                }
            }

            // Generate remediation recommendations
            report.recommendations = generate_remediation_plan(report);
            report.end_time = now();

            // Trigger security alerts for critical findings
            let critical_count = count_by_severity(report.vulnerabilities, "CRITICAL");
            if critical_count > 0 {
                log("CRITICAL", "Found " + critical_count + " critical vulnerabilities");
                trigger_security_alert(target, critical_count);
            }

            return report;

        } catch (error) {
            log("ERROR", "Security scan failed: " + error.message);
            return error("Scan failed: " + error.message);
        }
    }
}

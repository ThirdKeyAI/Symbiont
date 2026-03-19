metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Research agent that only executes SchemaPin-verified tools, demonstrating supply-chain security for MCP integrations"
    tags = ["research", "schemapin", "supply-chain", "verification", "trust"]
}

# This agent demonstrates SchemaPin's Trust-On-First-Use (TOFU) model.
# Every tool invocation and web request must have a cryptographically
# valid signature before execution. Unsigned or tampered tools are rejected.

agent schemapin_researcher(query: String, sources: List) -> ResearchReport {
    capabilities = ["read", "web_search", "summarize", "cite"]

    policy verified_tools_only {
        // Only allow tool invocations with valid SchemaPin signatures
        allow: invoke_tool(tool) if tool.schema_verified == true
        deny: invoke_tool(tool) if tool.schema_verified == false

        // Only allow web requests to domains with SchemaPin discovery
        allow: web_request(url) if url.domain.has_schemapin_discovery == true
        deny: web_request(url) if url.domain.has_schemapin_discovery == false

        // Read local files freely (no external trust needed)
        allow: read(file) if file.local == true

        // No writes — research is read-only
        deny: write(any)
        deny: execute(any)

        require: {
            schemapin_mode: "strict",       // reject unverified tools
            trust_bundle: "default",         // use pre-configured trust anchors
            pin_on_first_use: false,         // strict mode — no TOFU
            verify_tool_schemas: true,
            log_verification_failures: true
        }

        audit: {
            log_level: "info",
            include_verification_status: true,
            include_tool_signatures: true,
            include_trust_chain: true,
            alert_on_verification_failure: true
        }
    }

    with
        sandbox = "Tier1",
        memory = "session",
        timeout = 120000,
        max_memory_mb = 1024
    {
        report = ResearchReport {
            query: query,
            findings: [],
            sources_checked: 0,
            sources_verified: 0,
            sources_rejected: 0,
            verification_log: []
        };

        for source in sources {
            report.sources_checked += 1;

            try {
                // Verify the source has valid SchemaPin signatures
                let verification = verify_schemapin(source.domain);
                report.verification_log.push({
                    domain: source.domain,
                    status: "verified",
                    public_key: verification.public_key_fingerprint,
                    algorithm: verification.algorithm
                });
                report.sources_verified += 1;

                // Fetch and analyze the verified source
                let data = web_request(source.url);
                let analysis = summarize(data, context = query);

                report.findings.push({
                    source: source.url,
                    summary: analysis.summary,
                    relevance: analysis.relevance_score,
                    verified: true,
                    signature: verification.signature
                });

            } catch (verification_error) {
                // Source failed verification — log but don't use
                report.verification_log.push({
                    domain: source.domain,
                    status: "rejected",
                    reason: verification_error.message
                });
                report.sources_rejected += 1;

                log("WARNING", "Rejected unverified source: " + source.domain);
            }
        }

        // Synthesize findings from verified sources only
        if report.findings.len() > 0 {
            report.synthesis = synthesize(
                report.findings,
                style = "academic",
                require_citations = true
            );
        } else {
            report.synthesis = "No verified sources available for this query.";
        }

        report.trust_summary = format(
            "{}/{} sources verified, {} rejected",
            report.sources_verified,
            report.sources_checked,
            report.sources_rejected
        );

        return report;
    }
}

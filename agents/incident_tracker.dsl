metadata {
    version = "1.4.0"
    author = "Symbiont Community"
    description = "Incident tracker combining webhook ingestion with persistent memory for history"
    tags = ["webhook", "memory", "incident-response", "devops", "alerting"]
}

// Persistent memory for incident history and post-mortems
memory incident_history {
    store     markdown
    path      "data/incidents"
    retention 730d
    search {
        vector_weight  0.5
        keyword_weight 0.5
    }
}

// GitHub webhook: security advisories and failed deployments
webhook github_incidents {
    path     "/hooks/github"
    provider github
    secret   "vault://webhooks/github/secret"
    agent    incident_tracker
    filter {
        json_path "$.action"
        equals    "created"
    }
}

// Stripe webhook: payment failures
webhook stripe_failures {
    path     "/hooks/stripe"
    provider stripe
    secret   "vault://webhooks/stripe/secret"
    agent    incident_tracker
    filter {
        json_path "$.type"
        contains  "failed"
    }
}

agent incident_tracker(event: JSON) -> IncidentReport {
    capabilities = ["event_processing", "memory_read", "memory_write", "alerting", "signature_verification"]

    policy incident_guard {
        allow: ["parse_event", "classify_severity", "memory_write", "memory_read", "publish_alert"]
            if event.verified == true
        allow: ["memory_read", "search_history"]
        deny: ["execute_code", "file_access", "network_access"]

        require: {
            signature_verification: true,
            rate_limiting: "500/minute",
            input_validation: true,
            deduplication: true
        }

        audit: {
            log_level: "warning",
            include_input: false,
            include_output: true,
            include_metadata: true,
            retention_days: 365,
            alert_on_critical: true,
            compliance_tags: ["SOC2", "incident-response"]
        }
    }

    with
        memory = "persistent",
        privacy = "high",
        security = "high",
        sandbox = "Tier1",
        timeout = 10000,
        max_memory_mb = 512,
        max_cpu_cores = 1.0
    {
        // Classify incident severity from event payload
        let severity = classify_severity(event);
        let source = event.source;
        let dedup_key = generate_dedup_key(event);

        // Check for duplicate events
        let existing = memory_search(incident_history, dedup_key, limit: 1);
        if length(existing) > 0 {
            // Update existing incident count
            memory_write(incident_history, {
                "dedup_key": dedup_key,
                "occurrences": existing[0].occurrences + 1,
                "last_seen": now()
            });
            return { "status": "deduplicated", "incident_id": existing[0].id };
        }

        // Search for similar past incidents
        let similar = memory_search(incident_history, event.summary, limit: 5);
        let resolution_hints = extract_resolutions(similar);

        // Store new incident
        let incident = {
            "id": generate_id(),
            "dedup_key": dedup_key,
            "source": source,
            "severity": severity,
            "summary": event.summary,
            "payload": event,
            "similar_incidents": length(similar),
            "resolution_hints": resolution_hints,
            "occurrences": 1,
            "created_at": now(),
            "last_seen": now(),
            "status": "open"
        };

        memory_write(incident_history, incident);

        // Route alerts by severity
        if severity == "critical" {
            publish("topic://pages", incident);
            publish("topic://alerts/critical", incident);
        }
        if severity == "high" {
            publish("topic://alerts/high", incident);
        }
        if severity == "warning" {
            publish("topic://alerts/warning", incident);
        }

        return {
            "status": "created",
            "incident_id": incident.id,
            "severity": severity,
            "similar_past_incidents": length(similar),
            "resolution_hints": resolution_hints
        };
    }
}

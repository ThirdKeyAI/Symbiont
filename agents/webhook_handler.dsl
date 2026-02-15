metadata {
    version = "1.4.0"
    author = "Symbiont Community"
    description = "Webhook handler with signature verification and alert routing"
    tags = ["webhook", "event-processing", "alerting", "security"]
}

// v1.4.0: First-class webhook block with provider preset and event filter
webhook slack_alerts {
    path     "/hooks/slack"
    provider slack
    secret   "vault://webhooks/slack/secret"
    agent    webhook_handler
    filter {
        json_path "$.type"
        equals    "security_alert"
    }
}

agent webhook_handler(body: JSON) -> Maybe<Alert> {
    capabilities = ["http_input", "event_processing", "alerting", "signature_verification"]

    policy webhook_guard {
        allow: ["parse_json", "validate_signature", "publish_alert"]
            if body.source == "slack" || body.user.ends_with("@company.com")
        allow: "publish_alert" if body.type == "security_alert"
        deny: ["execute_code", "file_access", "network_access"]

        require: {
            signature_verification: true,
            rate_limiting: "100/minute",
            input_validation: true
        }

        audit: {
            log_level: "info",
            include_input: false,
            include_output: true,
            include_metadata: true,
            retention_days: 90
        }
    }

    with
        memory = "ephemeral",
        privacy = "strict",
        security = "high",
        sandbox = "Tier1",
        timeout = 5000,
        max_memory_mb = 256,
        max_cpu_cores = 0.5
    {
        try {
            // Verify webhook signature
            let signature = body.headers["X-Webhook-Signature"];
            let secret = vault://webhooks/slack/secret;

            if !verify_hmac_sha256(body.raw, secret, signature) {
                log("WARNING", "Invalid webhook signature");
                return None;
            }

            // Process security alerts
            if body.type == "security_alert" {
                alert = {
                    "summary": body.message,
                    "source": body.source,
                    "level": body.severity,
                    "user": body.user,
                    "timestamp": now()
                };

                publish("topic://alerts", alert);
                return Some(alert);
            }

            return None;

        } catch (error) {
            log("ERROR", "Webhook processing failed: " + error.message);
            return None;
        }
    }
}

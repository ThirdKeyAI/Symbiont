metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Multi-channel notification router with consent management and rate limiting"
    tags = ["notifications", "routing", "email", "slack", "sms", "webhook"]
}

agent notification_router(event: Event, routing_rules: NotificationRules) -> NotificationResult {
    capabilities = ["event_processing", "notification_delivery", "multi_channel_messaging"]

    policy notification_delivery {
        allow: ["send_email", "send_slack", "send_webhook", "send_sms"]
            if recipient.consent_given == true && check_rate_limit(recipient.id)
        deny: "send_notification"
            if event.severity == "low" && current_time.is_outside_business_hours()
        deny: ["file_access", "execute_code"]

        require: {
            consent_verification: true,
            rate_limiting: "100/hour/recipient",
            retry_attempts: 3,
            timeout_per_channel: "5000ms"
        }

        audit: {
            log_level: "info",
            include_recipient_tracking: true,
            include_delivery_status: true,
            include_input: false,  // Protect notification content
            alert_on_delivery_failure: true,
            compliance_tags: ["GDPR", "consent-management"]
        }
    }

    with
        memory = "ephemeral",
        privacy = "high",
        security = "high",
        sandbox = "Tier1",
        timeout = 30000,
        max_memory_mb = 512,
        max_cpu_cores = 1.0
    {
        try {
            notifications = [];

            // Determine notification channels based on event severity and rules
            channels = determine_channels(event, routing_rules);

            for channel in channels {
                try {
                    notification = format_notification(event, channel);

                    match channel.type {
                        "email" => {
                            let smtp_config = vault://notifications/smtp/config;
                            result = send_email_with_retry(notification, channel.recipients, smtp_config, max_retries = 3);
                            notifications.append({"channel": "email", "result": result, "timestamp": now()});
                        },
                        "slack" => {
                            let webhook_url = vault://notifications/slack/webhook;
                            result = send_slack_with_retry(notification, webhook_url, max_retries = 3);
                            notifications.append({"channel": "slack", "result": result, "timestamp": now()});
                        },
                        "sms" => {
                            let sms_api_key = vault://notifications/twilio/api_key;
                            result = send_sms_with_retry(notification, channel.phone_numbers, sms_api_key, max_retries = 3);
                            notifications.append({"channel": "sms", "result": result, "timestamp": now()});
                        },
                        "webhook" => {
                            result = call_webhook_with_retry(notification, channel.endpoint, max_retries = 3);
                            notifications.append({"channel": "webhook", "result": result, "timestamp": now()});
                        },
                        _ => {
                            log("WARNING", "Unknown channel type: " + channel.type);
                        }
                    }
                } catch (ChannelError e) {
                    log("ERROR", "Channel delivery failed: " + channel.type + " - " + e.message);
                    notifications.append({
                        "channel": channel.type,
                        "result": "failed",
                        "error": e.message,
                        "timestamp": now()
                    });
                }
            }

            return NotificationResult {
                event_id: event.id,
                notifications_sent: notifications.filter(n => n.result == "success").length,
                delivery_results: notifications,
                timestamp: now()
            };

        } catch (error) {
            log("ERROR", "Notification routing failed: " + error.message);
            return NotificationResult {
                event_id: event.id,
                notifications_sent: 0,
                delivery_results: [],
                error: error.message,
                timestamp: now()
            };
        }
    }
}

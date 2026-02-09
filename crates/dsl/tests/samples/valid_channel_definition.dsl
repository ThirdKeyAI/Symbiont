channel slack_ops {
    platform: "slack"
    workspace: "acme-corp"
    channels: ["#ops-agents", "#compliance"]
    default_agent: "compliance_check"
    dlp_profile: "hipaa"
    audit_level: "full"
    default_deny: true

    policy channel_guard {
        allow: invoke("compliance_check")
        deny: invoke("deploy_prod")
        audit: all_interactions
    }

    data_classification {
        pii: redact
        phi: block
        api_key: redact
        public: allow
    }
}

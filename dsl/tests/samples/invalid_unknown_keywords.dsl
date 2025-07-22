unknown_block {
    invalid_field: "value"
}

agent TestAgent {
    unknown_capability: [invalid, keywords]
    
    invalid_policy UnknownPolicy {
        unknown_rule: action(parameter)
    }
}
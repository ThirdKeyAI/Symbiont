//! ToolClad argument validation
//!
//! Validates tool arguments against their declared types.
//! All types reject shell metacharacters by default.

use super::manifest::ArgDef;

/// Shell metacharacters that are always rejected.
const INJECTION_CHARS: &[char] = &[
    ';', '|', '&', '$', '`', '(', ')', '{', '}', '[', ']', '<', '>', '!',
];

/// Validate an argument value against its definition.
pub fn validate_arg(def: &ArgDef, value: &str) -> Result<String, String> {
    let value = value.trim();

    match def.type_name.as_str() {
        "string" => validate_string(def, value),
        "integer" => validate_integer(def, value),
        "port" => validate_port(value),
        "boolean" => validate_boolean(value),
        "enum" => validate_enum(def, value),
        "scope_target" => validate_scope_target(value),
        "url" => validate_url(def, value),
        "path" => validate_path(value),
        "ip_address" => validate_ip_address(value),
        "cidr" => validate_cidr(value),
        other => Err(format!("Unknown type: {}", other)),
    }
}

fn check_injection(value: &str) -> Result<(), String> {
    for c in INJECTION_CHARS {
        if value.contains(*c) {
            return Err(format!(
                "Injection detected: value contains forbidden character '{}'",
                c
            ));
        }
    }
    Ok(())
}

fn validate_string(def: &ArgDef, value: &str) -> Result<String, String> {
    check_injection(value)?;
    if value.is_empty() {
        return Err("String argument cannot be empty".to_string());
    }
    if let Some(pattern) = &def.pattern {
        let re = regex::Regex::new(pattern)
            .map_err(|e| format!("Invalid pattern '{}': {}", pattern, e))?;
        if !re.is_match(value) {
            return Err(format!(
                "Value '{}' does not match pattern '{}'",
                value, pattern
            ));
        }
    }
    Ok(value.to_string())
}

fn validate_integer(def: &ArgDef, value: &str) -> Result<String, String> {
    let mut n: i64 = value
        .parse()
        .map_err(|_| format!("'{}' is not a valid integer", value))?;

    if let Some(min) = def.min {
        if n < min {
            if def.clamp {
                n = min;
            } else {
                return Err(format!("Value {} is below minimum {}", n, min));
            }
        }
    }
    if let Some(max) = def.max {
        if n > max {
            if def.clamp {
                n = max;
            } else {
                return Err(format!("Value {} is above maximum {}", n, max));
            }
        }
    }
    Ok(n.to_string())
}

fn validate_port(value: &str) -> Result<String, String> {
    let n: u16 = value
        .parse()
        .map_err(|_| format!("'{}' is not a valid port number", value))?;
    if n == 0 {
        return Err("Port must be 1-65535".to_string());
    }
    Ok(n.to_string())
}

fn validate_boolean(value: &str) -> Result<String, String> {
    match value {
        "true" | "false" => Ok(value.to_string()),
        _ => Err(format!(
            "'{}' is not a valid boolean (use 'true' or 'false')",
            value
        )),
    }
}

fn validate_enum(def: &ArgDef, value: &str) -> Result<String, String> {
    check_injection(value)?;
    if let Some(allowed) = &def.allowed {
        if allowed.contains(&value.to_string()) {
            Ok(value.to_string())
        } else {
            Err(format!(
                "'{}' is not in allowed values: {}",
                value,
                allowed.join(", ")
            ))
        }
    } else {
        Err("Enum type requires 'allowed' list".to_string())
    }
}

fn validate_scope_target(value: &str) -> Result<String, String> {
    check_injection(value)?;
    if value.is_empty() {
        return Err("Scope target cannot be empty".to_string());
    }
    if value.contains('*') {
        return Err("Wildcards are not allowed in scope targets".to_string());
    }
    // Basic format check: IP, CIDR, or hostname
    if value.contains('/') {
        // CIDR
        validate_cidr(value)?;
    } else if value.parse::<std::net::IpAddr>().is_ok() {
        // Valid IP
    } else {
        // Hostname — alphanumeric + dots + hyphens
        if !value
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        {
            return Err(format!("'{}' is not a valid hostname", value));
        }
    }
    Ok(value.to_string())
}

fn validate_url(def: &ArgDef, value: &str) -> Result<String, String> {
    check_injection(value)?;
    if !value.contains("://") {
        return Err(format!("'{}' is not a valid URL", value));
    }
    if let Some(schemes) = &def.schemes {
        let scheme = value.split("://").next().unwrap_or("");
        if !schemes.contains(&scheme.to_string()) {
            return Err(format!(
                "URL scheme '{}' not allowed (allowed: {})",
                scheme,
                schemes.join(", ")
            ));
        }
    }
    Ok(value.to_string())
}

fn validate_path(value: &str) -> Result<String, String> {
    check_injection(value)?;
    if value.contains("..") {
        return Err("Path traversal (..) is not allowed".to_string());
    }
    Ok(value.to_string())
}

fn validate_ip_address(value: &str) -> Result<String, String> {
    value
        .parse::<std::net::IpAddr>()
        .map_err(|_| format!("'{}' is not a valid IP address", value))?;
    Ok(value.to_string())
}

fn validate_cidr(value: &str) -> Result<String, String> {
    check_injection(value)?;
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("'{}' is not valid CIDR notation", value));
    }
    parts[0]
        .parse::<std::net::IpAddr>()
        .map_err(|_| format!("'{}' has an invalid IP in CIDR", value))?;
    let prefix: u8 = parts[1]
        .parse()
        .map_err(|_| format!("'{}' has an invalid prefix length", value))?;
    if prefix > 128 {
        return Err(format!("CIDR prefix {} is too large", prefix));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_arg(type_name: &str) -> ArgDef {
        ArgDef {
            position: 1,
            required: true,
            type_name: type_name.to_string(),
            description: String::new(),
            allowed: None,
            default: None,
            pattern: None,
            sanitize: None,
            min: None,
            max: None,
            clamp: false,
            schemes: None,
            scope_check: false,
        }
    }

    #[test]
    fn test_string_valid() {
        let def = make_arg("string");
        assert!(validate_arg(&def, "hello").is_ok());
    }

    #[test]
    fn test_string_injection() {
        let def = make_arg("string");
        assert!(validate_arg(&def, "hello; rm -rf /").is_err());
        assert!(validate_arg(&def, "test | cat").is_err());
        assert!(validate_arg(&def, "$(whoami)").is_err());
    }

    #[test]
    fn test_integer_range() {
        let mut def = make_arg("integer");
        def.min = Some(1);
        def.max = Some(10);
        assert!(validate_arg(&def, "5").is_ok());
        assert!(validate_arg(&def, "0").is_err());
        assert!(validate_arg(&def, "11").is_err());
    }

    #[test]
    fn test_integer_clamp() {
        let mut def = make_arg("integer");
        def.min = Some(1);
        def.max = Some(10);
        def.clamp = true;
        assert_eq!(validate_arg(&def, "0").unwrap(), "1");
        assert_eq!(validate_arg(&def, "100").unwrap(), "10");
    }

    #[test]
    fn test_port() {
        let def = make_arg("port");
        assert!(validate_arg(&def, "80").is_ok());
        assert!(validate_arg(&def, "0").is_err());
        assert!(validate_arg(&def, "70000").is_err());
    }

    #[test]
    fn test_enum() {
        let mut def = make_arg("enum");
        def.allowed = Some(vec!["ping".into(), "service".into()]);
        assert!(validate_arg(&def, "ping").is_ok());
        assert!(validate_arg(&def, "exploit").is_err());
    }

    #[test]
    fn test_scope_target() {
        let def = make_arg("scope_target");
        assert!(validate_arg(&def, "10.0.1.5").is_ok());
        assert!(validate_arg(&def, "10.0.1.0/24").is_ok());
        assert!(validate_arg(&def, "example.com").is_ok());
        assert!(validate_arg(&def, "*.example.com").is_err());
        assert!(validate_arg(&def, "10.0.1.5; rm -rf /").is_err());
    }

    #[test]
    fn test_ip_address() {
        let def = make_arg("ip_address");
        assert!(validate_arg(&def, "192.168.1.1").is_ok());
        assert!(validate_arg(&def, "::1").is_ok());
        assert!(validate_arg(&def, "not-an-ip").is_err());
    }

    #[test]
    fn test_cidr() {
        let def = make_arg("cidr");
        assert!(validate_arg(&def, "10.0.0.0/8").is_ok());
        assert!(validate_arg(&def, "10.0.0.0").is_err()); // no prefix
    }
}

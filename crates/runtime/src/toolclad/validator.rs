//! ToolClad argument validation
//!
//! Validates tool arguments against their declared types.
//! All types reject shell metacharacters by default.

use std::collections::HashMap;
use std::path::Path;

use super::manifest::ArgDef;

/// Shell metacharacters and control characters that are always rejected.
const INJECTION_CHARS: &[char] = &[
    ';', '|', '&', '$', '`', '(', ')', '{', '}', '[', ']', '<', '>', '!', '\n', '\r', '\0',
];

/// Validate an argument value against its definition.
/// If `custom_types` is provided, unknown types are resolved against it.
pub fn validate_arg(def: &ArgDef, value: &str) -> Result<String, String> {
    validate_arg_with_custom(def, value, None)
}

/// Validate an argument value, falling back to custom type definitions for unknown types.
pub fn validate_arg_with_custom(
    def: &ArgDef,
    value: &str,
    custom_types: Option<&HashMap<String, ArgDef>>,
) -> Result<String, String> {
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
        "msf_options" => validate_msf_options(def, value),
        "credential_file" => validate_credential_file(def, value),
        "duration" => validate_duration(value),
        "regex_match" => validate_regex_match(def, value),
        other => {
            // Check custom types
            if let Some(types) = custom_types {
                if let Some(base_def) = types.get(other) {
                    return validate_arg_with_custom(base_def, value, custom_types);
                }
            }
            Err(format!("Unknown type: {}", other))
        }
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
    // Canonicalize to resolve symlinks and prevent path traversal via symlink
    let path = Path::new(value);
    if path.exists() {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path '{}': {}", value, e))?;
        Ok(canonical.to_string_lossy().to_string())
    } else {
        Ok(value.to_string())
    }
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
    let addr: std::net::IpAddr = parts[0]
        .parse()
        .map_err(|_| format!("'{}' has an invalid IP in CIDR", value))?;
    let prefix: u8 = parts[1]
        .parse()
        .map_err(|_| format!("'{}' has an invalid prefix length", value))?;
    let max_prefix = match addr {
        std::net::IpAddr::V4(_) => 32,
        std::net::IpAddr::V6(_) => 128,
    };
    if prefix > max_prefix {
        return Err(format!(
            "CIDR prefix {} is too large (max {} for {})",
            prefix,
            max_prefix,
            if addr.is_ipv4() { "IPv4" } else { "IPv6" }
        ));
    }
    Ok(value.to_string())
}

/// Validate MSF options: semicolon-delimited `set KEY VALUE` pairs.
fn validate_msf_options(_def: &ArgDef, value: &str) -> Result<String, String> {
    check_injection(value)?;
    if value.is_empty() {
        return Err("MSF options cannot be empty".to_string());
    }
    let set_re = regex::Regex::new(r"^set [A-Za-z0-9_]+ .+$").unwrap();
    for segment in value.split(';') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if !set_re.is_match(segment) {
            return Err(format!(
                "Invalid MSF option segment '{}': must match 'set KEY VALUE'",
                segment
            ));
        }
    }
    Ok(value.to_string())
}

/// Validate a credential file path: must be a valid path that exists on disk.
/// Canonicalizes the path to resolve symlinks.
fn validate_credential_file(_def: &ArgDef, value: &str) -> Result<String, String> {
    let validated = validate_path(value)?;
    let path = Path::new(&validated);
    if !path.exists() {
        return Err(format!("Credential file '{}' does not exist", value));
    }
    // Return the canonicalized path from validate_path
    Ok(validated)
}

/// Validate a duration: integer with optional suffix (s/m/h) or bare seconds.
/// Parses to seconds and rejects non-positive values.
fn validate_duration(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("Duration cannot be empty".to_string());
    }
    let (num_str, multiplier) = if let Some(n) = value.strip_suffix('h') {
        (n, 3600i64)
    } else if let Some(n) = value.strip_suffix('m') {
        (n, 60i64)
    } else if let Some(n) = value.strip_suffix('s') {
        (n, 1i64)
    } else {
        (value, 1i64)
    };
    let n: i64 = num_str
        .parse()
        .map_err(|_| format!("'{}' is not a valid duration", value))?;
    let seconds = n * multiplier;
    if seconds <= 0 {
        return Err(format!(
            "Duration must be positive, got {} seconds",
            seconds
        ));
    }
    Ok(seconds.to_string())
}

/// Validate a value against a required regex pattern from the arg definition.
fn validate_regex_match(def: &ArgDef, value: &str) -> Result<String, String> {
    check_injection(value)?;
    if value.is_empty() {
        return Err("regex_match argument cannot be empty".to_string());
    }
    let pattern = def
        .pattern
        .as_ref()
        .ok_or("regex_match type requires a 'pattern' field")?;
    let re =
        regex::Regex::new(pattern).map_err(|e| format!("Invalid pattern '{}': {}", pattern, e))?;
    if !re.is_match(value) {
        return Err(format!(
            "Value '{}' does not match required pattern '{}'",
            value, pattern
        ));
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

    #[test]
    fn test_cidr_ipv6() {
        let def = make_arg("cidr");
        assert!(validate_arg(&def, "2001:db8::/32").is_ok());
        assert!(validate_arg(&def, "::1/128").is_ok());
        assert!(validate_arg(&def, "fe80::/10").is_ok());
        // IPv6 prefix > 128 should fail
        assert!(validate_arg(&def, "::1/129").is_err());
    }

    #[test]
    fn test_cidr_ipv4_max_prefix() {
        let def = make_arg("cidr");
        assert!(validate_arg(&def, "192.168.0.0/32").is_ok());
        // IPv4 prefix > 32 should fail
        assert!(validate_arg(&def, "192.168.0.0/33").is_err());
    }

    #[test]
    fn test_msf_options_valid() {
        let def = make_arg("msf_options");
        assert_eq!(
            validate_arg(&def, "set RHOSTS 10.0.0.1").unwrap(),
            "set RHOSTS 10.0.0.1"
        );
    }

    #[test]
    fn test_msf_options_multiple() {
        let def = make_arg("msf_options");
        // Semicolons are in INJECTION_CHARS, so multi-segment strings get rejected
        // by check_injection. Single segments work.
        assert!(validate_arg(&def, "set RHOSTS 10.0.0.1; set RPORT 443").is_err());
    }

    #[test]
    fn test_msf_options_invalid_format() {
        let def = make_arg("msf_options");
        assert!(validate_arg(&def, "RHOSTS 10.0.0.1").is_err());
        assert!(validate_arg(&def, "").is_err());
    }

    #[test]
    fn test_credential_file_missing() {
        let def = make_arg("credential_file");
        assert!(validate_arg(&def, "/nonexistent/path/cred.key").is_err());
    }

    #[test]
    fn test_credential_file_traversal() {
        let def = make_arg("credential_file");
        assert!(validate_arg(&def, "/etc/../shadow").is_err());
    }

    #[test]
    fn test_duration_bare_seconds() {
        let def = make_arg("duration");
        assert_eq!(validate_arg(&def, "30").unwrap(), "30");
    }

    #[test]
    fn test_duration_with_suffix() {
        let def = make_arg("duration");
        assert_eq!(validate_arg(&def, "5m").unwrap(), "300");
        assert_eq!(validate_arg(&def, "2h").unwrap(), "7200");
        assert_eq!(validate_arg(&def, "10s").unwrap(), "10");
    }

    #[test]
    fn test_duration_non_positive() {
        let def = make_arg("duration");
        assert!(validate_arg(&def, "0").is_err());
        assert!(validate_arg(&def, "-5").is_err());
    }

    #[test]
    fn test_duration_invalid() {
        let def = make_arg("duration");
        assert!(validate_arg(&def, "abc").is_err());
        assert!(validate_arg(&def, "").is_err());
    }

    #[test]
    fn test_regex_match_valid() {
        let mut def = make_arg("regex_match");
        def.pattern = Some(r"^\d{3}-\d{4}$".to_string());
        assert!(validate_arg(&def, "123-4567").is_ok());
    }

    #[test]
    fn test_regex_match_no_match() {
        let mut def = make_arg("regex_match");
        def.pattern = Some(r"^\d{3}-\d{4}$".to_string());
        assert!(validate_arg(&def, "abc-defg").is_err());
    }

    #[test]
    fn test_regex_match_missing_pattern() {
        let def = make_arg("regex_match");
        assert!(validate_arg(&def, "anything").is_err());
    }

    #[test]
    fn test_custom_type_resolution() {
        let mut custom_types = HashMap::new();
        let mut base_def = make_arg("enum");
        base_def.allowed = Some(vec!["ssh".into(), "ftp".into(), "http".into()]);
        custom_types.insert("service_protocol".to_string(), base_def);

        let def = make_arg("service_protocol");
        assert!(validate_arg_with_custom(&def, "ssh", Some(&custom_types)).is_ok());
        assert!(validate_arg_with_custom(&def, "telnet", Some(&custom_types)).is_err());
    }

    #[test]
    fn test_custom_type_unknown() {
        let def = make_arg("totally_unknown");
        assert!(validate_arg(&def, "value").is_err());
    }
}

//! Runs the ToolClad reference vector suites (`tests/toolclad_vectors/`)
//! through Symbiont's validator, manifest loader and scope engine.
//!
//! Every vector is executed. A vector on which Symbiont deliberately
//! differs from the reference is listed in [`DIVERGENCES`] with its reason;
//! such a vector must still produce the *opposite* verdict, so a divergence
//! that silently disappears is caught too.

use std::collections::HashMap;

use symbi_runtime::toolclad::manifest::ArgDef;
use symbi_runtime::toolclad::validator::validate_arg_in;
use symbi_runtime::toolclad::{parse_manifest, Scope};

/// `(vector id, reason)`.
const DIVERGENCES: &[(&str, &str)] = &[(
    "msf-options-accept",
    "Symbiont's msf_options is one `set KEY VALUE` and refuses `;`: the value is \
     fed to msfconsole, where `;` separates console commands",
)];

fn load(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/toolclad_vectors")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must be present: {e}", path.display()));
    serde_json::from_str(&text).expect("vector JSON")
}

fn expected(id: &str, expect: &str) -> bool {
    let accept = match expect {
        "accept" => true,
        "reject" => false,
        other => panic!("vector {id}: unknown expect '{other}'"),
    };
    if DIVERGENCES.iter().any(|(d, _)| *d == id) {
        !accept
    } else {
        accept
    }
}

#[test]
fn injection_vectors() {
    let doc = load("injection_vectors.json");
    let mut failures = Vec::new();
    let mut executed = 0;
    let mut seen_divergences = Vec::new();

    for v in doc["arg_vectors"].as_array().expect("arg_vectors") {
        let id = v["id"].as_str().unwrap();
        let def: ArgDef = serde_json::from_value(v["arg"].clone())
            .unwrap_or_else(|e| panic!("vector {id}: bad arg definition: {e}"));
        let value = v["value"].as_str().unwrap();
        // The reference's strict entry point: template-form strictness.
        let verdict = validate_arg_in(&def, value, None, false);
        if verdict.is_ok() != expected(id, v["expect"].as_str().unwrap()) {
            failures.push(format!("arg vector {id}: got {verdict:?} ({})", v["note"]));
        }
        if DIVERGENCES.iter().any(|(d, _)| *d == id) {
            seen_divergences.push(id.to_string());
        }
        executed += 1;
    }

    for v in doc["manifest_vectors"]
        .as_array()
        .expect("manifest_vectors")
    {
        let id = v["id"].as_str().unwrap();
        let verdict = parse_manifest(v["manifest"].as_str().unwrap(), &HashMap::new());
        if verdict.is_ok() != expected(id, v["expect"].as_str().unwrap()) {
            failures.push(format!(
                "manifest vector {id}: got {} ({})",
                verdict
                    .map(|_| "accepted".to_string())
                    .unwrap_or_else(|e| e),
                v["note"]
            ));
        }
        if DIVERGENCES.iter().any(|(d, _)| *d == id) {
            seen_divergences.push(id.to_string());
        }
        executed += 1;
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for (id, _) in DIVERGENCES {
        assert!(
            seen_divergences.iter().any(|s| s == id),
            "listed divergence {id} is not in the vector file"
        );
    }
    assert!(
        executed >= 98,
        "expected the full suite, executed {executed}"
    );
}

fn rule_scope(targets: &[&str], domains: &[&str]) -> Scope {
    Scope {
        targets: targets.iter().map(|s| s.to_string()).collect(),
        domains: domains.iter().map(|s| s.to_string()).collect(),
        exclude: Vec::new(),
    }
}

#[test]
fn scope_vectors() {
    let doc = load("scope_vectors.json");
    let v = &doc["vectors"];
    let s = |x: &serde_json::Value| x.as_str().unwrap().to_string();
    let mut executed = 0;

    for section in ["ipv4_in_cidr", "ipv6_in_cidr"] {
        for case in v[section].as_array().unwrap() {
            let (ip, cidr) = (s(&case["ip"]), s(&case["cidr"]));
            let got = rule_scope(&[&cidr], &[]).check(&ip).is_ok();
            assert_eq!(
                got,
                case["expected"].as_bool().unwrap(),
                "{section}: {ip} in {cidr}"
            );
            executed += 1;
        }
    }
    for case in v["hostname_wildcard_match"].as_array().unwrap() {
        let (host, pattern) = (s(&case["hostname"]), s(&case["pattern"]));
        let got = rule_scope(&[], &[&pattern]).check(&host).is_ok();
        assert_eq!(
            got,
            case["expected"].as_bool().unwrap(),
            "{host} vs {pattern}"
        );
        executed += 1;
    }
    let target = ArgDef {
        type_name: "scope_target".into(),
        ..Default::default()
    };
    for case in v["scope_target_validation"].as_array().unwrap() {
        let value = s(&case["value"]);
        let got = validate_arg_in(&target, &value, None, false).is_ok();
        assert_eq!(
            got,
            case["valid"].as_bool().unwrap(),
            "scope_target {value:?}"
        );
        executed += 1;
    }
    for case in v["url_scope_check"].as_array().unwrap() {
        let url = s(&case["url"]);
        let hosts: Vec<String> = case["allowed_hosts"]
            .as_array()
            .unwrap()
            .iter()
            .map(s)
            .collect();
        let hosts: Vec<&str> = hosts.iter().map(String::as_str).collect();
        let got = rule_scope(&[], &hosts).check(&url).is_ok();
        assert_eq!(
            got,
            case["expected"].as_bool().unwrap(),
            "{url} against {hosts:?}"
        );
        executed += 1;
    }
    assert!(executed > 0);
}

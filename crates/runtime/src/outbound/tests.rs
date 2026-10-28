use super::*;
use reqwest::header::{HeaderMap, HeaderValue};
use std::time::{Duration, Instant};

fn capability() -> Capability {
    toml::from_str(
        r#"
tool = "case_read"
origin = "https://cases.example.test"
method = "GET"
path_prefix = "/records/"
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn outbound_authority_is_scoped_revocable_and_cannot_be_resurrected() {
    let registry = Registry::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    let first = registry
        .acquire("run-1", "scope-policy-1", deadline, None)
        .unwrap();
    let same = registry
        .acquire("run-1", "scope-policy-1", deadline, None)
        .unwrap();
    assert_eq!(first.epoch, same.epoch);
    let second = registry
        .acquire("run-2", "scope-policy-1", deadline, None)
        .unwrap();
    assert_ne!(first.epoch, second.epoch);
    first.admit_send("case_read", 1).unwrap();
    assert!(same.admit_send("case_read", 1).is_err());
    assert!(registry
        .acquire("run-1", "different-scope", deadline, None)
        .is_err());
    assert!(first.check_live().is_err());
    registry.cancel("run-1", deadline);
    assert!(first.admit_send("case_write", 1).is_err());
    assert!(registry
        .acquire("run-1", "scope-policy-1", deadline, None)
        .is_err());
    second.admit_send("case_read", 1).unwrap();
    registry.cancel("not-yet-started", deadline);
    assert!(registry
        .acquire("not-yet-started", "scope-policy-1", deadline, None)
        .is_err());
}

#[tokio::test]
async fn outbound_cancellation_interrupts_pending_resolution_and_expiry_denies_send() {
    let lease = Lease::new("scope".into(), Instant::now() + Duration::from_secs(10));
    let waiter = lease.clone();
    let task = tokio::spawn(async move {
        waiter
            .during(std::future::pending::<Result<(), String>>())
            .await
    });
    lease.revoke();
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    let expired = Lease::new("scope".into(), Instant::now());
    assert!(expired.admit_send("case_read", 1).is_err());
}

#[test]
fn outbound_capabilities_reject_origin_method_path_and_transport_confusion() {
    let cap = capability();
    for (method, url) in [
        ("POST", "https://cases.example.test/records/1"),
        ("GET", "https://other.example.test/records/1"),
        ("GET", "https://cases.example.test:444/records/1"),
        ("GET", "https://cases.example.test/recordsevil/1"),
        ("GET", "https://cases.example.test/records/a%2fb"),
        ("GET", "https://cases.example.test/records/%252e%252e"),
        ("GET", "https://cases.example.test/records/1#other"),
    ] {
        let request = reqwest::Request::new(method.parse().unwrap(), url.parse().unwrap());
        assert!(cap.check_request(&request).is_err(), "{method} {url}");
    }
    let mut request = reqwest::Request::new(
        reqwest::Method::GET,
        "https://cases.example.test/records/1".parse().unwrap(),
    );
    cap.check_request(&request).unwrap();
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("other.example.test"));
    assert!(cap.check_request(&request).is_err());
}

#[test]
fn outbound_private_exceptions_are_exact_ip_origins_and_never_metadata() {
    let mut cap = capability();
    cap.private_origin = true;
    for origin in [
        "http://localhost:8080",
        "http://169.254.169.254",
        "http://[fd00:ec2::254]",
        "http://127.0.0.1:8080/path",
        "http://127.0.0.1:0",
        "http://[::ffff:a9fe:a9fe]",
        "http://[fe80::1]",
        "http://100.100.100.200",
    ] {
        cap.origin = origin.into();
        assert!(cap.validate().is_err(), "{origin}");
    }
    for origin in [
        "http://[::ffff:7f00:1]:8080",
        "http://[fd12:3456::1]:8080",
        "http://10.0.0.5:8080",
        "http://127.0.0.1:8080",
    ] {
        cap.origin = origin.into();
        cap.validate().unwrap_or_else(|e| panic!("{origin}: {e}"));
    }
    cap.private_origin = false;
    assert!(cap.validate().is_err());
}

#[test]
fn outbound_credentials_are_sensitive_and_reflection_is_rejected_before_delivery() {
    let secret = Credential::new("synthetic-outbound-fixture-key".into()).unwrap();
    let mut headers = HeaderMap::new();
    secret
        .inject(&mut headers, "authorization", "Bearer ")
        .unwrap();
    assert!(headers["authorization"].is_sensitive());
    assert!(secret.check_response(&headers, b"ok").is_err());
    headers.clear();
    assert!(secret
        .check_response(&headers, b"prefix synthetic-outbound-fixture-key suffix")
        .is_err());
    secret.check_response(&headers, b"ordinary result").unwrap();
    let mixed = Credential::new("Synthetic-Mixed-Value".into()).unwrap();
    headers.insert(
        reqwest::header::HeaderName::from_bytes(b"Synthetic-Mixed-Value").unwrap(),
        reqwest::header::HeaderValue::from_static("reflected"),
    );
    assert!(mixed.check_response(&headers, b"ok").is_err());
    headers.clear();
    // Header-name normalization does not broaden body matching.
    mixed
        .check_response(&headers, b"synthetic-mixed-value")
        .unwrap();
}

#[tokio::test]
async fn outbound_store_failures_never_select_an_environment_fallback() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("store.toml");
    std::fs::write(
        &config,
        format!(
            "type = 'file'\npath = '{}'\nformat = 'json'\n",
            root.path().join("missing.json").display()
        ),
    )
    .unwrap();
    let source = CredentialSource::SecretStore {
        config_file: config,
        key: "missing".into(),
        version: "v1".into(),
    };
    assert!(source.resolve("fixture-agent").await.is_err());
    let invalid: Result<CredentialSource, _> = toml::from_str(
        "source = 'environment'\nvariable = 'FIXTURE_KEY'\nversion = 'v1'\nfallback = 'OTHER_KEY'",
    );
    assert!(invalid.is_err());
}

#[test]
fn outbound_profiles_fail_closed_on_unknown_fields_versions_and_sources() {
    let root = tempfile::tempdir().unwrap();
    assert!(Profile::load(root.path()).unwrap().is_none());
    for text in ["[outbound]\nschema_version = 99", "[outbound]\nmode = 'unrestricted'", "[outbound]\nschema_version = 1\ntenant = 't'\nworkspace = 'w'\ncapabilities = {}\ncredentials = {}"] {
        std::fs::write(root.path().join("symbiont.toml"), text).unwrap();
        assert!(Profile::load(root.path()).is_err());
    }
}

#[test]
fn outbound_closing_runs_that_never_acquired_does_not_exhaust_capacity() {
    let registry = Registry::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    for run in 0..(super::authority::MAX_RUNS * 2) {
        registry.cancel(&format!("idle-{run}"), deadline);
    }
    let (live, _) = registry.counts();
    assert_eq!(live, 0);
    let lease = registry.acquire("fresh", "scope", deadline, None).unwrap();
    lease.admit_send("case_read", 1).unwrap();
    // A run closed before it ever acquired still cannot acquire later.
    assert!(registry.acquire("idle-7", "scope", deadline, None).is_err());
}

#[test]
fn outbound_capacity_exhaustion_refuses_only_new_runs_and_recovers_after_expiry() {
    let registry = Registry::default();
    let long = Instant::now() + Duration::from_secs(10);
    // The margin must outlast filling the registry on a loaded host;
    // otherwise early leases expire mid-setup and are pruned.
    let short = Instant::now() + Duration::from_secs(3);
    let kept = registry.acquire("kept", "scope", long, None).unwrap();
    for run in 1..super::authority::MAX_RUNS {
        registry
            .acquire(&format!("busy-{run}"), "scope", short, None)
            .unwrap();
    }
    assert!(registry.acquire("overflow", "scope", long, None).is_err());
    // Cancelling an unknown run at capacity must not disable the registry or
    // revoke unrelated leases.
    registry.cancel("unknown", long);
    kept.admit_send("case_read", 2).unwrap();
    assert_eq!(
        registry.acquire("kept", "scope", long, None).unwrap().epoch,
        kept.epoch
    );
    std::thread::sleep(short.saturating_duration_since(Instant::now()) + Duration::from_millis(50));
    registry.acquire("overflow", "scope", long, None).unwrap();
    kept.admit_send("case_read", 2).unwrap();
}

#[test]
fn outbound_closed_run_set_overflow_refuses_new_runs_until_expiry() {
    let registry = Registry::default();
    let long = Instant::now() + Duration::from_secs(10);
    let kept = registry.acquire("kept", "scope", long, None).unwrap();
    // Fill the closed-run set with guards that expire soon.
    // Generous margin: inserting MAX_CLOSED guards can be slow on a loaded
    // host, and guards that expire mid-setup are pruned.
    let soon = Instant::now() - super::authority::CLOSED_GRACE + Duration::from_secs(5);
    let expires = soon + super::authority::CLOSED_GRACE;
    for run in 0..super::authority::MAX_CLOSED {
        registry.cancel(&format!("closed-{run}"), soon);
    }
    registry.cancel("overflowing", soon);
    assert!(registry
        .acquire("overflowing", "scope", long, None)
        .is_err());
    assert!(registry.acquire("new", "scope", long, None).is_err());
    kept.admit_send("case_read", 1).unwrap();
    std::thread::sleep(
        expires.saturating_duration_since(Instant::now()) + Duration::from_millis(50),
    );
    registry.acquire("new", "scope", long, None).unwrap();
}

#[test]
fn outbound_closed_run_guard_covers_the_grant_run_deadline() {
    let registry = Registry::default();
    let supplied = Instant::now() + Duration::from_millis(50);
    let run_deadline = Instant::now() + super::authority::CLOSED_GRACE + Duration::from_secs(30);
    let lease = registry
        .acquire("run", "scope", run_deadline, None)
        .unwrap();
    registry.cancel("run", supplied);
    assert!(lease.admit_send("case_read", 1).is_err());
    let (_, closed) = registry.counts();
    assert_eq!(closed, 1);
    assert!(
        registry.closed_until("run").unwrap() >= run_deadline,
        "guard must last until the lease's run deadline"
    );
    // A run that never acquired: a refused late grant extends the guard to
    // that grant's own run deadline.
    registry.cancel("idle", supplied);
    assert!(registry
        .acquire("idle", "scope", run_deadline, None)
        .is_err());
    assert!(registry.closed_until("idle").unwrap() >= run_deadline);
}

#[tokio::test]
async fn outbound_revoking_a_live_lease_cancels_in_flight_work() {
    let registry = Registry::default();
    let lease = registry
        .acquire(
            "run",
            "scope",
            Instant::now() + Duration::from_secs(10),
            None,
        )
        .unwrap();
    let waiter = lease.clone();
    let task = tokio::spawn(async move {
        waiter
            .during(std::future::pending::<Result<(), String>>())
            .await
    });
    tokio::task::yield_now().await;
    registry.cancel("run", Instant::now() + Duration::from_secs(10));
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(registry
        .acquire(
            "run",
            "scope",
            Instant::now() + Duration::from_secs(10),
            None
        )
        .is_err());
}

fn get(cap: &Capability, url: &str) -> Result<(), String> {
    cap.check_request(&reqwest::Request::new(
        reqwest::Method::GET,
        url.parse().unwrap(),
    ))
}

#[test]
fn outbound_path_parameters_and_dot_segments_are_rejected_unless_opted_in() {
    let mut cap = capability();
    for url in [
        "https://cases.example.test/records/..;/admin",
        "https://cases.example.test/records/..%3B/admin",
        "https://cases.example.test/records/..%3b/admin",
        "https://cases.example.test/records/1%3bjsessionid=x",
        "https://cases.example.test/records/1;v=2",
    ] {
        assert!(get(&cap, url).is_err(), "{url}");
    }
    cap.allow_path_parameters = true;
    cap.validate().unwrap();
    get(&cap, "https://cases.example.test/records/1;v=2").unwrap();
    get(&cap, "https://cases.example.test/records/1%3Bv=2").unwrap();
    // Opting in never admits a parameterized dot segment.
    for url in [
        "https://cases.example.test/records/..;/admin",
        "https://cases.example.test/records/.;x/admin",
        "https://cases.example.test/records/..%3B/admin",
        "https://cases.example.test/records/..%3bx/admin",
        "https://cases.example.test/records/.%3Bx/admin",
    ] {
        assert!(get(&cap, url).is_err(), "{url}");
    }
    // The opt-in is explicit configuration, and unknown fields stay rejected.
    let parsed: Capability = toml::from_str(
        "tool='t'\norigin='https://cases.example.test'\nmethod='GET'\npath_prefix='/'\nallow_path_parameters=true",
    )
    .unwrap();
    assert!(parsed.allow_path_parameters);
    assert!(toml::from_str::<Capability>(
        "tool='t'\norigin='https://cases.example.test'\nmethod='GET'\npath_prefix='/'\nallow_matrix=true",
    )
    .is_err());
}

#[test]
fn outbound_dot_segments_are_checked_after_percent_decoding() {
    assert_eq!(super::config::percent_decode("%2E%2e"), b"..");
    assert_eq!(super::config::percent_decode("%zz."), b"%zz.");
    assert_eq!(super::config::percent_decode("a%2"), b"a%2");
}

#[test]
fn outbound_store_identity_excludes_authentication_values() {
    let root = tempfile::tempdir().unwrap();
    let config_file = root.path().join("store.toml");
    let source = CredentialSource::SecretStore {
        config_file: config_file.clone(),
        key: "cases".into(),
        version: "v1".into(),
    };
    for method in ["token", "approle"] {
        let descriptor = |secret: &str, mount: &str| {
            let auth = if method == "token" {
                format!("method='token'\ntoken='{secret}'")
            } else {
                format!("method='approle'\nrole_id='fixture-role'\nsecret_id='{secret}'")
            };
            std::fs::write(
                &config_file,
                format!("type='vault'\nurl='https://vault.example.test'\nmount_path='{mount}'\n[auth]\n{auth}\n"),
            )
            .unwrap();
            source.freeze().unwrap().descriptor
        };
        let first = descriptor("synthetic-first", "secret");
        assert_eq!(first, descriptor("synthetic-second", "secret"), "{method}");
        assert_ne!(first, descriptor("synthetic-first", "another-mount"));
    }
}

#[test]
fn outbound_query_keys_are_constrained_only_when_configured() {
    let mut cap = capability();
    get(
        &cap,
        "https://cases.example.test/records/1?anything=1&other=2",
    )
    .unwrap();
    cap.allowed_query_keys = Some(vec!["page".into(), "limit".into()]);
    cap.validate().unwrap();
    get(&cap, "https://cases.example.test/records/1?page=2&limit=10").unwrap();
    get(&cap, "https://cases.example.test/records/1").unwrap();
    for url in [
        "https://cases.example.test/records/1?page=2&admin=1",
        "https://cases.example.test/records/1?page=2&page=3",
        "https://cases.example.test/records/1?pa%67e=2&page=3",
        "https://cases.example.test/records/1?debug",
    ] {
        assert!(get(&cap, url).is_err(), "{url}");
    }
    cap.allowed_query_keys = Some(vec![]);
    cap.validate().unwrap();
    assert!(get(&cap, "https://cases.example.test/records/1?page=1").is_err());
    for keys in [vec!["page".to_string(), "page".into()], vec![String::new()]] {
        cap.allowed_query_keys = Some(keys);
        assert!(cap.validate().is_err());
    }
}

#[tokio::test]
async fn outbound_credentials_resolve_once_per_lease_and_again_for_a_new_run() {
    use std::sync::atomic::Ordering;
    let variable = "SYMBI_OUTBOUND_LEASE_CACHE_FIXTURE";
    std::env::set_var(variable, "synthetic-lease-cache-value");
    let frozen = CredentialSource::Environment {
        variable: variable.into(),
        version: "v1".into(),
    }
    .freeze()
    .unwrap();
    let registry = Registry::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    let lease = registry.acquire("run-1", "scope", deadline, None).unwrap();
    let first = lease.credential("cases", &frozen, "agent").await.unwrap();
    let second = lease.credential("cases", &frozen, "agent").await.unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(frozen.resolutions.load(Ordering::SeqCst), 1);

    let next = registry.acquire("run-2", "scope", deadline, None).unwrap();
    next.credential("cases", &frozen, "agent").await.unwrap();
    assert_eq!(frozen.resolutions.load(Ordering::SeqCst), 2);

    // Revocation drops the cached value and refuses further resolution.
    let weak = std::sync::Arc::downgrade(&first);
    registry.cancel("run-1", deadline);
    drop((first, second));
    assert!(
        weak.upgrade().is_none(),
        "revocation must drop the cached credential"
    );
    assert!(lease.credential("cases", &frozen, "agent").await.is_err());
    assert_eq!(frozen.resolutions.load(Ordering::SeqCst), 2);
    std::env::remove_var(variable);
}

#[test]
fn outbound_registry_leases_adopt_and_enforce_the_session_assignment_epoch() {
    let registry = Registry::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    let session = uuid::Uuid::new_v4();
    let lease = registry
        .acquire("run", "scope", deadline, Some(session))
        .unwrap();
    assert_eq!(lease.epoch, session);
    assert_eq!(
        registry
            .acquire("run", "scope", deadline, Some(session))
            .unwrap()
            .epoch,
        session
    );
    // A dispatch bound to another assignment revokes the run's authority.
    assert!(registry
        .acquire("run", "scope", deadline, Some(uuid::Uuid::new_v4()))
        .is_err());
    assert!(lease.check_live().is_err());
}

#[test]
fn outbound_capability_validation_explains_origin_and_method_mistakes() {
    let mut cap = capability();
    cap.method = "OPTIONS".into();
    assert!(cap.validate().unwrap_err().contains("OPTIONS"));
    cap.method = "GET".into();
    cap.origin = "https://Cases.Example.Test".into();
    assert!(cap.validate().unwrap_err().contains("lowercase"));
    cap.origin = "https://cases.example.test:443".into();
    assert!(cap.validate().unwrap_err().contains("default port"));
    cap.origin = "https://cases.example.test:8443".into();
    cap.validate().unwrap();
}

#[tokio::test]
async fn outbound_concurrent_store_lookups_share_one_resolution() {
    let root = tempfile::tempdir().unwrap();
    let config_file = root.path().join("store.toml");
    let data = root.path().join("store.json");
    std::fs::write(&data, r#"{"cases":"synthetic-concurrent-value"}"#).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::fs::write(
        &config_file,
        format!("type='file'\nformat='json'\npath='{}'\n", data.display()),
    )
    .unwrap();
    let source = CredentialSource::SecretStore {
        config_file,
        key: "cases".into(),
        version: "v1".into(),
    }
    .freeze()
    .unwrap();
    let lease = Lease::new("fixture".into(), Instant::now() + Duration::from_secs(30));
    let lookups = (0..16).map(|_| lease.credential("cases", &source, "fixture"));
    let results = futures::future::join_all(lookups).await;
    let first = results[0].as_ref().unwrap();
    assert!(results
        .iter()
        .all(|result| std::sync::Arc::ptr_eq(first, result.as_ref().unwrap())));
    assert_eq!(
        source.resolutions.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    lease.revoke();
    assert!(lease.credential("cases", &source, "fixture").await.is_err());
}

#[test]
fn outbound_reflection_search_is_exact_and_linear() {
    for (pattern, haystack, expected) in [
        ("abc", "xxabcxx", true),
        ("aab", "aaab", true),
        ("abab", "abaabab", true),
        ("abc", "ababab", false),
        ("abc", "ab", false),
    ] {
        assert_eq!(
            super::Needle::new(pattern.as_bytes()).found_in(haystack.as_bytes()),
            expected,
            "{pattern} in {haystack}"
        );
    }
    // Adversarial input that is quadratic for a naive window scan.
    let secret = format!("{}b", "a".repeat(4095));
    let credential = Credential::new(secret).unwrap();
    let body = vec![b'a'; 4 * 1024 * 1024];
    let started = Instant::now();
    credential.check_response(&HeaderMap::new(), &body).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
}

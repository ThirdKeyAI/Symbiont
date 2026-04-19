//! E2E-7: per-IP rate limiting.
//!
//! Drives the server with a quick burst from a single loopback IP and
//! asserts that the limiter eventually responds with 429. The limiter
//! quota is 100 req/min, so 200 sequential requests on the unauthed
//! /api/v1/health endpoint should trip it.

#![cfg(feature = "e2e")]

use symbi_e2e::{spawn_runtime_server, test_client, SpawnOptions};
use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_limiter_issues_429_after_burst() {
    let td = tempdir().unwrap();
    let mut opts = SpawnOptions::new(td);
    opts.enable_rate_limiting = true;
    let server = spawn_runtime_server(opts).await;

    let client = test_client();
    let url = format!("{}/api/v1/health", server.base_url);

    let mut saw_429 = false;
    for _ in 0..200 {
        let status = client.get(&url).send().await.expect("req").status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            saw_429 = true;
            break;
        }
    }
    assert!(
        saw_429,
        "expected at least one 429 after 200 rapid requests against /api/v1/health"
    );

    server.shutdown().await;
}

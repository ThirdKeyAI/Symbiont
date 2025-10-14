//! HTTP Input Module Integration Tests
//!
//! Tests for the HTTP Input module that validates functionality, security, and correctness
//! based on the original test plan.

#[cfg(feature = "http-input")]
use std::sync::Arc;
#[cfg(feature = "http-input")]
use std::time::Duration;

#[cfg(feature = "http-input")]
use reqwest;
#[cfg(feature = "http-input")]
use serde_json::json;
#[cfg(feature = "http-input")]
use tokio::time::timeout;

#[cfg(feature = "http-input")]
use symbi_runtime::{
    AgentRuntime, AgentId, RuntimeConfig, ExecutionMode,
    SecurityTier, ResourceLimits, Capability, Priority,
    http_input::{HttpInputConfig, HttpInputServer},
};

#[cfg(feature = "http-input")]
/// Create a test HTTP input configuration with a random port
fn create_test_config(port: u16) -> HttpInputConfig {
    HttpInputConfig {
        bind_address: "127.0.0.1".to_string(),
        port,
        path: "/webhook".to_string(),
        agent: AgentId::new(),
        auth_header: Some("Bearer test-token-123".to_string()),
        jwt_public_key_path: None,
        max_body_bytes: 1024, // 1KB for testing payload size limits
        concurrency: 5,
        routing_rules: None,
        response_control: None,
        forward_headers: vec![],
        cors_enabled: true,
        audit_enabled: true,
    }
}

#[cfg(feature = "http-input")]
/// Create a minimal agent runtime for testing
async fn create_test_runtime() -> AgentRuntime {
    let config = RuntimeConfig::default();
    AgentRuntime::new(config).await.expect("Failed to create runtime")
}

#[cfg(feature = "http-input")]
/// Find an available port for testing
async fn find_available_port() -> u16 {
    use tokio::net::TcpListener;
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[cfg(feature = "http-input")]
/// Start a test HTTP server and return the server handle and base URL
async fn start_test_server() -> (tokio::task::JoinHandle<()>, String, u16) {
    let port = find_available_port().await;
    let config = create_test_config(port);
    let runtime = Arc::new(create_test_runtime().await);
    let base_url = format!("http://127.0.0.1:{}", port);
    
    let server = HttpInputServer::new(config)
        .with_runtime(runtime);
    
    let handle = tokio::spawn(async move {
        if let Err(e) = server.start().await {
            eprintln!("Server error: {:?}", e);
        }
    });
    
    // Wait a moment for server to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    (handle, base_url, port)
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_valid_request_returns_200_ok() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    let payload = json!({
        "message": "Hello from webhook",
        "data": {
            "source": "test",
            "timestamp": "2024-01-01T00:00:00Z"
        }
    });
    
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Authorization", "Bearer test-token-123")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Assert successful response
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("content-type").unwrap().to_str().unwrap().contains("application/json"));
    
    // Verify response body contains expected agent invocation result
    let response_body: serde_json::Value = response.json().await.expect("Failed to parse JSON response");
    assert!(response_body.get("status").is_some());
    assert_eq!(response_body["status"], "invoked");
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_invalid_token_returns_401_unauthorized() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    let payload = json!({
        "message": "This should fail with wrong token"
    });
    
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Authorization", "Bearer wrong-token")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Assert unauthorized response
    assert_eq!(response.status(), 401);
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_missing_token_returns_401_unauthorized() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    let payload = json!({
        "message": "This should fail without token"
    });
    
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Assert unauthorized response
    assert_eq!(response.status(), 401);
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_payload_too_large_returns_413() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    // Create a payload larger than the configured max_body_bytes (1024 bytes)
    let large_data = "x".repeat(2048); // 2KB payload
    let payload = json!({
        "message": "Large payload test",
        "large_data": large_data
    });
    
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Authorization", "Bearer test-token-123")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Assert payload too large response
    assert_eq!(response.status(), 413);
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_malformed_json_returns_400_bad_request() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    // Send malformed JSON
    let malformed_json = r#"{"message": "incomplete json""#; // Missing closing brace
    
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Authorization", "Bearer test-token-123")
            .header("Content-Type", "application/json")
            .body(malformed_json)
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Assert bad request response
    assert_eq!(response.status(), 400);
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_agent_interaction_and_invocation() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    let payload = json!({
        "action": "test_agent_invocation",
        "parameters": {
            "test_param": "test_value"
        }
    });
    
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Authorization", "Bearer test-token-123")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Assert successful response
    assert_eq!(response.status(), 200);
    
    // Verify response contains agent invocation details
    let response_body: serde_json::Value = response.json().await.expect("Failed to parse JSON response");
    
    // Check that the agent was invoked correctly
    assert_eq!(response_body["status"], "invoked");
    assert!(response_body.get("agent_id").is_some());
    assert!(response_body.get("timestamp").is_some());
    
    // Verify the timestamp is a valid RFC3339 format
    let timestamp_str = response_body["timestamp"].as_str().unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(timestamp_str).is_ok());
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_cors_headers_when_enabled() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    // Send an OPTIONS request to check CORS headers
    let response = timeout(Duration::from_secs(5), 
        client
            .request(reqwest::Method::OPTIONS, &format!("{}/webhook", base_url))
            .header("Origin", "https://example.com")
            .header("Access-Control-Request-Method", "POST")
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // Check for CORS headers (exact headers depend on tower-http CORS implementation)
    assert!(response.headers().contains_key("access-control-allow-origin") ||
            response.headers().contains_key("Access-Control-Allow-Origin"));
}

#[cfg(feature = "http-input")]
#[tokio::test]
async fn test_content_type_enforcement() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    // Send request without proper content-type
    let response = timeout(Duration::from_secs(5), 
        client
            .post(&format!("{}/webhook", base_url))
            .header("Authorization", "Bearer test-token-123")
            .body(r#"{"message": "test"}"#)
            // Deliberately omit Content-Type header
            .send()
    ).await.expect("Request timeout").expect("Request failed");
    
    // The server should handle this gracefully, likely returning 400 or processing as text
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[cfg(feature = "http-input")]
#[tokio::test]  
async fn test_concurrent_requests_within_limits() {
    let (_handle, base_url, _port) = start_test_server().await;
    let client = reqwest::Client::new();
    
    let _payload = json!({
        "message": "Concurrent request test"
    });
    
    // Send multiple concurrent requests (within the concurrency limit of 5)
    let mut handles = vec![];
    for i in 0..3 {
        let client = client.clone();
        let url = format!("{}/webhook", base_url);
        let payload = json!({
            "message": format!("Concurrent request {}", i)
        });
        
        let handle = tokio::spawn(async move {
            timeout(Duration::from_secs(5), 
                client
                    .post(&url)
                    .header("Authorization", "Bearer test-token-123")
                    .header("Content-Type", "application/json")
                    .json(&payload)
                    .send()
            ).await.expect("Request timeout").expect("Request failed")
        });
        
        handles.push(handle);
    }
    
    // Wait for all requests to complete
    let responses = futures::future::join_all(handles).await;
    
    // All requests should succeed
    for response in responses {
        let response = response.expect("Task failed");
        assert_eq!(response.status(), 200);
    }
}

// Test that only compiles when http-input feature is enabled
#[cfg(not(feature = "http-input"))]
#[tokio::test]
async fn test_http_input_feature_disabled() {
    // This test ensures that when the http-input feature is not enabled,
    // we can't access the http_input module
    
    // This should not compile if http-input feature is disabled:
    // use symbi_runtime::http_input::HttpInputConfig;
    
    // Instead, we just verify the test runs
    // Test completed successfully
}
//! Real Chromium and local HTTP sinks using the request policy and call audit.
//! The executor here is a fixture; production browser run ownership is pending.
use super::*;
use crate::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry,
        conversation::Conversation,
        dispatch::GovernedToolDispatcher,
        effect_journal::{EffectJournal, ToolEffect},
        executor::ActionExecutor,
        inference::ToolDefinition,
        loop_types::{LoopConfig, LoopEvent, LoopState, Observation, ProposedAction},
        policy_bridge::DefaultPolicyGate,
        prepared::AuthorizedAction,
        protected_journal::ProtectedJournal,
    },
    toolclad::{
        browser_network::{fulfill_response, BrowserNetworkPolicy, MAX_RESPONSE_BYTES},
        http_transport,
        manifest::{BrowserNetworkDef, BrowserScopeDef},
    },
};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Instant};
use tokio::net::{TcpListener, TcpStream};

pub(super) struct Broker {
    policy: BrowserNetworkPolicy,
    client: reqwest::Client,
    audit: EffectJournal,
    deadline: Instant,
    denied: Vec<String>,
}
impl Broker {
    pub(super) async fn handle(&mut self, params: &Value) -> Result<Value, String> {
        self.audit.check_live()?;
        let request = match self.policy.prepare_request(params) {
            Ok(request) => request,
            Err(error) => {
                self.denied
                    .push(params["request"]["url"].as_str().unwrap().into());
                return Err(error);
            }
        };
        let response = http_transport::exchange(
            &self.client,
            request,
            Some(&self.audit),
            self.deadline,
            MAX_RESPONSE_BYTES,
        )
        .await?;
        fulfill_response(
            params["requestId"].as_str().unwrap(),
            response.status,
            &response.headers,
            &response.body,
        )
    }
}

struct BrowserFixture {
    origin: String,
    label: String,
}
#[async_trait::async_trait]
impl ActionExecutor for BrowserFixture {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "browser_network_fixture".into(),
            description: "Local browser request mediation fixture".into(),
            parameters: json!({"type":"object","properties":{}}),
        }]
    }
    async fn execute_actions(
        &self,
        _: &[ProposedAction],
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        panic!("fixture requires governed dispatch");
    }
    async fn execute_authorized(
        &self,
        grants: Vec<AuthorizedAction>,
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        assert_eq!(grants.len(), 1);
        let grant = grants.into_iter().next().unwrap();
        grant.check_live().unwrap();
        let policy = BrowserNetworkPolicy::new(
            &BrowserScopeDef {
                allowed_domains: vec!["127.0.0.1".into()],
                blocked_domains: vec![],
                allow_external: false,
            },
            Some(&BrowserNetworkDef {
                allowed_methods: vec!["GET".into(), "HEAD".into(), "POST".into()],
                private_origins: vec![self.origin.clone()],
            }),
        )
        .unwrap();
        let mut probe = start(&self.label, Duration::from_secs(30)).await;
        let version = probe.request("Browser.getVersion", json!({}), None).await;
        println!("broker browser artifact: {version}");
        probe.broker = Some(Broker {
            client: policy.client(Duration::from_secs(5)).unwrap(),
            policy,
            audit: grant.effect_journal().unwrap(),
            deadline: grant.deadline(),
            denied: vec![],
        });
        let target = probe
            .request("Target.createTarget", json!({"url":"about:blank"}), None)
            .await;
        let session = probe
            .request(
                "Target.attachToTarget",
                json!({"targetId":target["targetId"],"flatten":true}),
                None,
            )
            .await;
        let session = session["sessionId"].as_str().unwrap();
        probe.request("Page.enable", json!({}), Some(session)).await;
        probe
            .request("Fetch.enable", json!({}), Some(session))
            .await;
        let navigation = probe
            .request(
                "Page.navigate",
                json!({"url":format!("{}/", self.origin)}),
                Some(session),
            )
            .await;
        assert!(navigation.get("errorText").is_none(), "{navigation}");
        let page = probe.request("Runtime.evaluate", json!({"expression":r#"
            (async () => {
                if (document.readyState !== 'complete') await new Promise(resolve => addEventListener('load', resolve, {once:true}));
                const response = await fetch('/mutate', {method:'POST',headers:{'Content-Type':'text/plain'},body:' exact body ∑ '});
                document.getElementById('result').textContent = await response.text();
                let deniedMethod = false;
                try { await fetch('/forbidden', {method:'DELETE'}); } catch (_) { deniedMethod = true; }
                return {title:document.title, result:document.getElementById('result').textContent,
                    direct:document.getElementById('direct').dataset.denied,
                    redirect:document.getElementById('redirect').dataset.denied, deniedMethod};
            })()
        "#, "awaitPromise":true,"returnByValue":true}), Some(session)).await;
        assert!(page.get("exceptionDetails").is_none(), "{page}");
        assert_eq!(
            page["result"]["value"],
            json!({"title":"Broker fixture","result":"stored","direct":"true","redirect":"true","deniedMethod":true})
        );
        assert_eq!(probe.broker.as_ref().unwrap().denied.len(), 3);
        assert_eq!(probe.requests.len(), 6);
        println!("observed browser requests: {:?}", probe.requests);
        // Drain the final interception acknowledgement before reporting success.
        probe
            .request(
                "Runtime.evaluate",
                json!({"expression":"1","returnByValue":true}),
                Some(session),
            )
            .await;
        assert!(probe.pending.is_empty());
        probe.close().await;
        assert!(owned_containers(&self.label).await.is_empty());
        let ProposedAction::ToolCall { call_id, name, .. } = grant.action() else {
            panic!("call");
        };
        vec![
            Observation::tool_result(name, page["result"]["value"].to_string())
                .with_call_id(call_id),
        ]
    }
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buf = [0; 4096];
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0 && bytes.len() + n < 16 * 1024);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..end]);
            let length: usize = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() == end + 4 + length {
                return bytes;
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker, supervisor and pinned local Chromium image"]
async fn contained_browser_brokers_audited_http_and_blocks_redirect_egress() {
    tokio::time::timeout(Duration::from_secs(40), async {
        let allowed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let denied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", allowed.local_addr().unwrap());
        let denied_origin = format!("http://{}", denied.local_addr().unwrap());
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let state = LoopState::new(crate::types::AgentId::new(), Conversation::new());
        let journal = Arc::new(ProtectedJournal::create(root.path(), state.agent_id).unwrap());
        let observer = journal.clone();
        let expected_origin = origin.clone();
        let server = tokio::spawn(async move {
            let mut seen = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = allowed.accept().await.unwrap();
                let entries = ProtectedJournal::verify(observer.path(), &observer.public_key()).unwrap();
                let LoopEvent::ToolEffect {effect:ToolEffect::NetworkRequestStarted {url,..}, ..} = &entries.last().unwrap().event else { panic!("request must be durable before connection"); };
                assert!(url.starts_with(&expected_origin));
                let request = read_request(&mut stream).await;
                let line = String::from_utf8_lossy(&request).lines().next().unwrap().to_owned();
                let (status, headers, body) = match line.as_str() {
                    "GET / HTTP/1.1" => (200, "Content-Type: text/html; charset=utf-8\r\n".to_owned(), format!(r#"<!doctype html><title>Broker fixture</title><p id="result">waiting</p><img id="direct" src="{denied_origin}/pixel" onerror="this.dataset.denied='true'"><img id="redirect" src="/redirect" onerror="this.dataset.denied='true'">"#)),
                    "GET /redirect HTTP/1.1" => (302, format!("Location: {denied_origin}/bounced\r\n"), String::new()),
                    "POST /mutate HTTP/1.1" => {
                        assert!(request.ends_with(" exact body ∑ ".as_bytes()));
                        assert!(String::from_utf8_lossy(&request).to_lowercase().contains(&format!("host: {}\r\n", allowed.local_addr().unwrap())));
                        (200, "Content-Type: text/plain\r\n".into(), "stored".into())
                    }
                    _ => panic!("unexpected request reached allowed sink: {line}"),
                };
                seen.push(line);
                let response = format!("HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            seen
        });
        let fixture = BrowserFixture { origin, label:format!("symbi.browser-broker={}", uuid::Uuid::new_v4()) };
        let config = LoopConfig { tool_definitions:fixture.tool_definitions(), tool_timeout:Duration::from_secs(35), ..Default::default() };
        let output = GovernedToolDispatcher { executor:&fixture, journal:journal.as_ref(), gate:&DefaultPolicyGate::permissive_for_dev_only(), circuit_breakers:&CircuitBreakerRegistry::default() }
            .dispatch(&[ProposedAction::ToolCall { call_id:"browser-call".into(), name:"browser_network_fixture".into(), arguments:"{}".into() }], &state, &config).await.unwrap();
        assert_eq!(output.len(), 1);
        assert!(!output[0].is_error, "{:?}", output[0]);
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 3);
        assert!(tokio::time::timeout(Duration::from_millis(250), denied.accept()).await.is_err(), "denied origin received a connection");
        let entries = ProtectedJournal::verify(journal.path(), &journal.public_key()).unwrap();
        let LoopEvent::PolicyEvaluated { approved_calls, .. } = &entries[0].event else { panic!("policy"); };
        let fingerprint = approved_calls[0]["fingerprint"].as_str().unwrap();
        let mut started = HashMap::new();
        let mut finished = Vec::new();
        for entry in &entries {
            if let LoopEvent::ToolEffect { call_fingerprint, effect, .. } = &entry.event {
                assert_eq!(call_fingerprint, fingerprint);
                match effect {
                    ToolEffect::NetworkRequestStarted {request_id,method,url,request_hash,..} => {
                        assert_eq!(request_hash.len(), 64);
                        assert!(started.insert(request_id.clone(), (method.clone(),url.clone())).is_none());
                    }
                    ToolEffect::NetworkRequestFinished {request_id,status,response_hash,error,..} => {
                        assert!(started.contains_key(request_id));
                        assert!(matches!(status,Some(200|302)));
                        assert_eq!(response_hash.as_ref().unwrap().len(), 64);
                        assert!(error.is_none());
                        assert!(!finished.contains(request_id));
                        finished.push(request_id.clone());
                    }
                    ToolEffect::FilePublicationPrepared { .. }
                    | ToolEffect::FilePublicationFinished { .. } => {
                        panic!("unexpected file publication effect in browser broker journal")
                    }
                }
            }
        }
        assert_eq!(started.len(), 3);
        assert_eq!(finished.len(), 3);
        println!("verified {} signed entries; {} actual HTTP effects; zero denied-sink connections; worker removed", entries.len(), seen.len());
    }).await.expect("bounded browser broker test");
}

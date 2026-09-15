use super::*;

fn scope() -> BrowserScopeDef {
    BrowserScopeDef {
        allowed_domains: vec![],
        blocked_domains: vec![],
        allow_external: true,
    }
}

fn request(url: &str, method: &str) -> Value {
    json!({"requestId":"request-1", "request":{"url":url,"method":method,"headers":{}}})
}

#[test]
fn private_capabilities_are_exact_and_do_not_replace_scope() {
    let default = BrowserNetworkPolicy::new(&scope(), None).unwrap();
    for url in [
        "http://127.0.0.1:8080/",
        "http://10.0.0.1/",
        "http://localhost/",
        "http://[::1]/",
        "http://169.254.169.254/",
    ] {
        assert!(default.check_destination(url, "GET").is_err(), "{url}");
    }
    assert!(default
        .check_destination("https://example.com/", "GET")
        .is_ok());
    let network = BrowserNetworkDef {
        private_origins: vec![
            "http://127.0.0.1:8080".into(),
            "https://[fd12::1]:8443".into(),
        ],
        ..Default::default()
    };
    let policy = BrowserNetworkPolicy::new(&scope(), Some(&network)).unwrap();
    for url in [
        "http://127.1:8080/path",
        "http://0x7f000001:8080/",
        "https://[fd12::1]:8443/",
    ] {
        assert!(policy.check_destination(url, "GET").is_ok(), "{url}");
    }
    for url in [
        "http://127.0.0.1:8081/",
        "https://127.0.0.1:8080/",
        "http://localhost:8080/",
        "http://127.0.0.2:8080/",
        "http://[::ffff:127.0.0.1]:8080/",
    ] {
        assert!(policy.check_destination(url, "GET").is_err(), "{url}");
    }
    let blocked = BrowserScopeDef {
        blocked_domains: vec!["127.0.0.1".into()],
        ..scope()
    };
    assert!(BrowserNetworkPolicy::new(&blocked, Some(&network))
        .unwrap()
        .check_destination("http://127.1:8080/", "GET")
        .is_err());
}

#[test]
fn malformed_or_overbroad_capabilities_fail_closed() {
    for origin in [
        "http://localhost:80",
        "http://169.254.169.254",
        "http://[fd00:ec2::254]",
        "http://0.0.0.0",
        "http://224.0.0.1",
        "http://[fe80::1]",
        "http://[::ffff:10.0.0.1]",
        "https://example.com",
        "http://127.0.0.1/path",
        "http://127.0.0.1?query",
        "http://127.0.0.1#hash",
        "http://user@127.0.0.1",
        "http://127.0.0.1:0",
    ] {
        let network = BrowserNetworkDef {
            private_origins: vec![origin.into()],
            ..Default::default()
        };
        assert!(
            BrowserNetworkPolicy::new(&scope(), Some(&network)).is_err(),
            "{origin}"
        );
    }
    for methods in [
        vec![],
        vec!["CONNECT"],
        vec!["TRACE"],
        vec!["get"],
        vec!["GET", "GET"],
    ] {
        let network = BrowserNetworkDef {
            allowed_methods: methods.into_iter().map(String::from).collect(),
            ..Default::default()
        };
        assert!(BrowserNetworkPolicy::new(&scope(), Some(&network)).is_err());
    }
    let policy = BrowserNetworkPolicy::new(&scope(), None).unwrap();
    assert!(policy
        .prepare_request(&request("https://example.com/", "POST"))
        .is_err());
    assert!(toml::from_str::<BrowserNetworkDef>("allow_private=true").is_err());
}

#[test]
fn request_translation_binds_authority_and_preserves_complete_body_bytes() {
    let policy = BrowserNetworkPolicy::new(
        &scope(),
        Some(&BrowserNetworkDef {
            allowed_methods: vec!["GET".into(), "POST".into()],
            ..Default::default()
        }),
    )
    .unwrap();
    let mut input = request("https://ExAMPLe.com:443/a/../effect#fragment", "POST");
    input["request"]["headers"] = json!({"Host":"metadata.invalid", "Connection":"X-Hop", "X-Hop":"removed", "Proxy-Authorization":"removed", "Content-Length":"999", "Transfer-Encoding":"chunked", "Expect":"100-continue", "X-Exact":"preserved", "Content-Type":"application/octet-stream"});
    input["request"]["hasPostData"] = json!(true);
    input["request"]["postDataEntries"] =
        json!([{"bytes":STANDARD.encode([0, 255])},{"bytes":STANDARD.encode(b" exact ")}]);
    let output = policy.prepare_request(&input).unwrap();
    assert_eq!(output.url().as_str(), "https://example.com/effect");
    assert_eq!(output.body().unwrap().as_bytes().unwrap(), b"\0\xff exact ");
    assert_eq!(output.headers().len(), 2);
    assert_eq!(output.headers()["x-exact"], "preserved");
    for change in [
        json!({"responseStatusCode":200}),
        json!({"responseErrorReason":"Failed"}),
    ] {
        let mut changed = input.clone();
        changed
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(policy.prepare_request(&changed).is_err());
    }
}

#[test]
fn ambiguous_incomplete_and_oversized_requests_are_refused() {
    let policy = BrowserNetworkPolicy::new(
        &scope(),
        Some(&BrowserNetworkDef {
            allowed_methods: vec!["POST".into()],
            ..Default::default()
        }),
    )
    .unwrap();
    let cases = [
        json!({"hasPostData":true}),
        json!({"hasPostData":false,"postData":"present"}),
        json!({"hasPostData":"true"}),
        json!({"postDataEntries":[{}]}),
        json!({"hasPostData":true,"postDataEntries":[]}),
        json!({"postDataEntries":[{"bytes":"YQ==","file":"omitted"}]}),
        json!({"postDataEntries":[{"bytes":"bad base64"}]}),
        json!({"postDataEntries":[{"bytes":"YQ=="}],"postData":"different"}),
        json!({"postData": "x".repeat(MAX_REQUEST_BYTES + 1)}),
        json!({"headers":{"Content-Type":"multipart/form-data; boundary=test"}}),
        json!({"headers":{"Content-Type":" \tMuLtIpArT/form-data; boundary=test"}}),
        json!({"headers":{"X-Duplicate":"one","x-duplicate":"two"}}),
        json!({"headers":{"X-Invalid":"value\r\nInjected: yes"}}),
        json!({"headers":{":authority":"metadata.invalid"}}),
        json!({"headers":{"X-Large":"x".repeat(MAX_HEADER_BYTES + 1)}}),
    ];
    for change in cases {
        let mut input = request("https://example.com/", "POST");
        input["request"]
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(policy.prepare_request(&input).is_err());
    }
}

#[test]
fn response_translation_preserves_redirects_cookies_and_binary_values() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "location",
        HeaderValue::from_static("http://127.0.0.1/blocked"),
    );
    headers.insert("connection", HeaderValue::from_static("x-hop"));
    headers.insert("x-hop", HeaderValue::from_static("removed"));
    headers.insert("content-length", HeaderValue::from_static("999"));
    headers.append("set-cookie", HeaderValue::from_static("first=one"));
    headers.append("set-cookie", HeaderValue::from_static("second=two"));
    headers.insert("x-binary", HeaderValue::from_bytes(&[255]).unwrap());
    let output = fulfill_response("paused-1", 302, &headers, b"encoded body").unwrap();
    assert_eq!(output["responseCode"], 302);
    assert_eq!(
        STANDARD.decode(output["body"].as_str().unwrap()).unwrap(),
        b"encoded body"
    );
    let bytes = STANDARD
        .decode(output["binaryResponseHeaders"].as_str().unwrap())
        .unwrap();
    let fields: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    assert_eq!(fields.len(), 4);
    assert!(fields.contains(&b"set-cookie: first=one".as_slice()));
    assert!(fields.contains(&b"set-cookie: second=two".as_slice()));
    assert!(fields.contains(&b"x-binary: \xff".as_slice()));
    assert!(fulfill_response("id", 101, &headers, &[]).is_err());
    assert!(fulfill_response("id", 200, &headers, &vec![0; MAX_RESPONSE_BYTES + 1]).is_err());
}

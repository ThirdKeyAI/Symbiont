//! Browser request construction for a trusted CDP mediator.
//!
//! These helpers validate untrusted request events; they do not issue execution
//! authority. A driver must retain the isolated worker, intercept every target,
//! and dispatch the unchanged request through the call-bound effect journal.
//! Never continue a paused request onto the browser's own network stack.
use super::{
    browser_state::{parse_browser_url, BrowserScopeChecker},
    manifest::{BrowserNetworkDef, BrowserScopeDef},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Method, Request,
};
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

pub const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEADERS: usize = 128;
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Immutable destination/method configuration copied from a trusted manifest.
pub struct BrowserNetworkPolicy {
    scope: BrowserScopeChecker,
    methods: HashSet<String>,
    private_origins: HashSet<String>,
}

impl BrowserNetworkPolicy {
    pub fn new(
        scope: &BrowserScopeDef,
        network: Option<&BrowserNetworkDef>,
    ) -> Result<Self, String> {
        let scope = BrowserScopeChecker::new(scope);
        scope.validate()?;
        let network = network.cloned().unwrap_or_default();
        if network.allowed_methods.is_empty()
            || network.allowed_methods.len() > 7
            || network.private_origins.len() > 32
        {
            return Err("browser network capability count is invalid".into());
        }
        let mut methods = HashSet::new();
        for method in network.allowed_methods {
            if !matches!(
                method.as_str(),
                "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
            ) || !methods.insert(method)
            {
                return Err(
                    "browser methods must be unique supported uppercase HTTP methods".into(),
                );
            }
        }
        let mut private_origins = HashSet::new();
        for value in network.private_origins {
            let url = parse_browser_url(&value)?;
            let private = match url.host() {
                Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private(),
                Some(url::Host::Ipv6(ip)) => {
                    (ip.is_loopback() || ip.is_unique_local())
                        && ip != "fd00:ec2::254".parse::<std::net::Ipv6Addr>().unwrap()
                }
                _ => false,
            };
            // Only literal loopback/RFC1918/ULA origins are eligible. In
            // particular, DNS, metadata, link-local, mapped IPv6 and public
            // addresses cannot use this exception. Port zero is never useful.
            if !private
                || url.port_or_known_default() == Some(0)
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || !private_origins.insert(url.origin().ascii_serialization())
            {
                return Err("browser private origins must be unique literal loopback/private HTTP(S) origins without a path, query or fragment".into());
            }
        }
        Ok(Self {
            scope,
            methods,
            private_origins,
        })
    }

    /// Both domain scope and address class apply. The private exception binds
    /// scheme, literal IP and effective port; it never relaxes DNS filtering.
    pub fn check_destination(&self, value: &str, method: &str) -> Result<url::Url, String> {
        if !self.methods.contains(method) {
            return Err("browser HTTP method is outside the manifest capability".into());
        }
        let url = parse_browser_url(value)?;
        self.scope.check_url(url.as_str())?;
        if !self
            .private_origins
            .contains(&url.origin().ascii_serialization())
        {
            crate::net_guard::reject_ssrf_url(url.as_str())?;
        }
        Ok(url)
    }

    /// Build the broker client. Responses remain byte-for-byte encoded as
    /// received; Chromium performs content decoding after CDP fulfillment.
    /// There is no cookie jar, ambient proxy, automatic redirect or TLS bypass.
    pub fn client(&self, timeout: Duration) -> Result<reqwest::Client, String> {
        if timeout.is_zero() || timeout > Duration::from_secs(300) {
            return Err("browser request timeout must be within 1 ns..300 seconds".into());
        }
        crate::net_guard::customise_ssrf_safe_client(timeout, |builder| {
            builder.no_gzip().no_brotli().no_deflate().no_zstd()
        })
        .map_err(|e| format!("browser broker client: {e}"))
    }

    /// Translate a `Fetch.requestPaused` params object at the request stage.
    /// The returned request must remain unchanged through audited dispatch.
    /// Incomplete, multipart/file and ambiguous bodies are refused.
    pub fn prepare_request(&self, params: &Value) -> Result<Request, String> {
        if !params.is_object()
            || params.get("responseStatusCode").is_some()
            || params.get("responseErrorReason").is_some()
        {
            return Err("browser broker requires a paused request before network execution".into());
        }
        let request = params
            .get("request")
            .and_then(Value::as_object)
            .ok_or("missing browser request")?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or("missing browser method")?;
        let mut url = self.check_destination(
            request
                .get("url")
                .and_then(Value::as_str)
                .ok_or("missing browser URL")?,
            method,
        )?;
        // Fragments are page state, never HTTP request-target bytes.
        url.set_fragment(None);
        let headers = request
            .get("headers")
            .and_then(Value::as_object)
            .ok_or("missing browser request headers")?;
        if headers.len() > MAX_HEADERS {
            return Err("browser request header count exceeds limit".into());
        }
        let mut parsed = HeaderMap::new();
        let mut bytes = method.len() + url.as_str().len();
        let mut header_bytes = 0usize;
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "invalid browser header name")?;
            let value = value
                .as_str()
                .ok_or("browser header values must be strings")?;
            header_bytes = header_bytes
                .saturating_add(name.as_str().len())
                .saturating_add(value.len());
            if header_bytes > MAX_HEADER_BYTES {
                return Err("browser request headers exceed byte limit".into());
            }
            let value = HeaderValue::from_str(value).map_err(|_| "invalid browser header value")?;
            if parsed.insert(name, value).is_some() {
                return Err("case-duplicate browser request headers are ambiguous".into());
            }
        }
        bytes += header_bytes;
        if parsed.get("content-type").is_some_and(|value| {
            value
                .to_str()
                .unwrap_or("")
                .trim_start()
                .as_bytes()
                .get(..10)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"multipart/"))
        }) {
            return Err(
                "browser multipart/file uploads require a separate complete-body capability".into(),
            );
        }
        let body = request_body(request)?;
        bytes = bytes.saturating_add(body.len());
        if bytes > MAX_REQUEST_BYTES {
            return Err("browser request exceeds application-byte limit".into());
        }
        if matches!(method, "GET" | "HEAD") && !body.is_empty() {
            return Err("browser GET/HEAD requests cannot contain a body".into());
        }
        let connection = connection_headers(&parsed)?;
        let names: Vec<_> = parsed
            .keys()
            .filter(|name| {
                hop_header(name.as_str())
                    || connection.contains(name.as_str())
                    || matches!(name.as_str(), "host" | "content-length" | "expect")
            })
            .cloned()
            .collect();
        for name in names {
            parsed.remove(name);
        }
        let mut result = Request::new(
            Method::from_bytes(method.as_bytes()).map_err(|_| "invalid HTTP method")?,
            url,
        );
        *result.headers_mut() = parsed;
        if !body.is_empty() {
            *result.body_mut() = Some(body.into());
        }
        Ok(result)
    }
}

fn request_body(request: &serde_json::Map<String, Value>) -> Result<Vec<u8>, String> {
    let has_body = match request.get("hasPostData") {
        Some(value) => Some(value.as_bool().ok_or("invalid browser body flag")?),
        None => None,
    };
    let text = match request.get("postData") {
        Some(value) => Some(value.as_str().ok_or("invalid browser text body")?),
        None => None,
    };
    let mut body = Vec::new();
    if let Some(entries) = request.get("postDataEntries") {
        let entries = entries.as_array().ok_or("invalid browser body entries")?;
        if entries.len() > 128 {
            return Err("browser body has too many entries".into());
        }
        if entries.is_empty() && has_body == Some(true) {
            return Err("browser body entry list is incomplete".into());
        }
        for entry in entries {
            if entry.as_object().is_none_or(|entry| entry.len() != 1) {
                return Err("browser body entry contains unsupported data".into());
            }
            let encoded = entry
                .get("bytes")
                .and_then(Value::as_str)
                .ok_or("browser body entry is incomplete")?;
            if encoded.len() > MAX_REQUEST_BYTES * 4 / 3 + 4 {
                return Err("browser body entry exceeds limit".into());
            }
            let part = STANDARD
                .decode(encoded)
                .map_err(|_| "invalid browser body encoding")?;
            if part.len() > MAX_REQUEST_BYTES.saturating_sub(body.len()) {
                return Err("browser body exceeds limit".into());
            }
            body.extend_from_slice(&part);
        }
        if text.is_some_and(|text| text.as_bytes() != body) {
            return Err("browser body representations disagree".into());
        }
    } else if let Some(text) = text {
        if text.len() > MAX_REQUEST_BYTES {
            return Err("browser body exceeds limit".into());
        }
        body.extend_from_slice(text.as_bytes());
    } else if has_body == Some(true) {
        return Err("browser request body was omitted by the transport".into());
    }
    if has_body == Some(false) && !body.is_empty() {
        return Err("browser body conflicts with its absence flag".into());
    }
    Ok(body)
}

fn hop_header(name: &str) -> bool {
    name.starts_with("proxy-")
        || matches!(
            name,
            "connection" | "keep-alive" | "te" | "trailer" | "transfer-encoding" | "upgrade"
        )
}

fn connection_headers(headers: &HeaderMap) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for value in headers.get_all("connection") {
        for name in value
            .to_str()
            .map_err(|_| "invalid connection header")?
            .split(',')
        {
            let name = HeaderName::from_bytes(name.trim().as_bytes())
                .map_err(|_| "invalid connection header token")?;
            names.insert(name.as_str().to_owned());
        }
    }
    Ok(names)
}

/// Build `Fetch.fulfillRequest` params from a complete bounded broker response.
/// Binary header encoding preserves non-UTF8 values and duplicate Set-Cookie.
/// Redirects are returned to Chromium for a new independently checked request.
pub fn fulfill_response(
    request_id: &str,
    status: u16,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Value, String> {
    if request_id.is_empty()
        || request_id.len() > 256
        || request_id.chars().any(char::is_control)
        || !(200..=599).contains(&status)
        || body.len() > MAX_RESPONSE_BYTES
        || headers.len() > MAX_HEADERS
    {
        return Err("browser response exceeds supported bounds".into());
    }
    let connection = connection_headers(headers)?;
    let mut binary = Vec::new();
    for (name, value) in headers {
        if hop_header(name.as_str())
            || name == "content-length"
            || connection.contains(name.as_str())
        {
            continue;
        }
        if value
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
        {
            return Err("invalid browser response header bytes".into());
        }
        if !binary.is_empty() {
            binary.push(0);
        }
        binary.extend_from_slice(name.as_str().as_bytes());
        binary.extend_from_slice(b": ");
        binary.extend_from_slice(value.as_bytes());
        if binary.len() > MAX_HEADER_BYTES {
            return Err("browser response headers exceed byte limit".into());
        }
    }
    Ok(json!({"requestId": request_id, "responseCode": status,
        "binaryResponseHeaders": STANDARD.encode(binary), "body": STANDARD.encode(body)}))
}

#[cfg(test)]
#[path = "browser/network_tests.rs"]
mod tests;

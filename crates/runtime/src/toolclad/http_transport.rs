//! Bounded asynchronous HTTP exchange with optional call-bound effect records.
//! Destination policy belongs to the caller; redirects must remain disabled.
use crate::reasoning::effect_journal::{EffectJournal, ToolEffect};
use sha2::{Digest, Sha256};
use std::time::Instant;

#[cfg(all(test, unix))]
#[path = "http_transport_tests.rs"]
mod tests;

pub(super) struct Response {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

/// Hash the built request, including resolved secret values, without storing
/// those values in the journal. Streaming bodies are not accepted.
#[cfg(test)]
fn request_identity(request: &reqwest::Request) -> Result<(String, u64), String> {
    request_identity_with(request, None)
}

/// Like [`request_identity`], but the value of `redact.0` is replaced by the
/// non-secret reference `redact.1`. The journal then binds the exact request
/// and credential identity without becoming an offline guessing oracle for the
/// injected credential.
fn request_identity_with(
    request: &reqwest::Request,
    redact: Option<(&reqwest::header::HeaderName, &str)>,
) -> Result<(String, u64), String> {
    let body = match request.body() {
        Some(body) => body
            .as_bytes()
            .ok_or("HTTP streaming request bodies are unsupported")?,
        None => &[],
    };
    let mut headers: Vec<_> = request.headers().iter().collect();
    headers.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    let mut bytes = request.method().as_str().len() + request.url().as_str().len() + body.len();
    let mut hash = Sha256::new();
    hash_field(&mut hash, b"symbi-http-request-v1");
    hash_field(&mut hash, request.method().as_str().as_bytes());
    hash_field(&mut hash, request.url().as_str().as_bytes());
    for (name, value) in headers {
        let value = match redact {
            Some((secret, reference)) if secret == name => reference.as_bytes(),
            _ => value.as_bytes(),
        };
        bytes = bytes.saturating_add(name.as_str().len() + value.len());
        hash_field(&mut hash, name.as_str().as_bytes());
        hash_field(&mut hash, value);
    }
    hash_field(&mut hash, body);
    if bytes > 2 * 1024 * 1024 {
        return Err("HTTP request exceeds 2 MiB application-byte limit".into());
    }
    Ok((hex::encode(hash.finalize()), bytes as u64))
}

/// The journaled identity of a request. A capability credential is replaced
/// by its non-secret reference before hashing.
fn journal_identity(
    request: &reqwest::Request,
    guard: Option<&crate::outbound::DispatchGuard>,
) -> Result<(String, u64), String> {
    let reference = guard
        .map(|guard| guard.credential_reference())
        .transpose()?
        .flatten();
    request_identity_with(
        request,
        reference
            .as_ref()
            .map(|(name, reference)| (name, reference.as_str())),
    )
}

#[cfg(test)]
pub(super) async fn exchange(
    client: &reqwest::Client,
    request: reqwest::Request,
    journal: Option<&EffectJournal>,
    deadline: Instant,
    limit: usize,
) -> Result<Response, String> {
    exchange_guarded(client, request, journal, deadline, limit, None).await
}

pub(super) async fn exchange_guarded(
    client: &reqwest::Client,
    request: reqwest::Request,
    journal: Option<&EffectJournal>,
    deadline: Instant,
    limit: usize,
    guard: Option<&crate::outbound::DispatchGuard>,
) -> Result<Response, String> {
    if Instant::now() >= deadline || limit == 0 || limit > 10 * 1024 * 1024 {
        return Err("HTTP exchange has an expired deadline or invalid response limit".into());
    }
    let (request_hash, request_bytes) = journal_identity(&request, guard)?;
    let request_id = uuid::Uuid::new_v4().to_string();
    if let Some(journal) = journal {
        journal
            .append(ToolEffect::NetworkRequestStarted {
                outbound: guard.map(|guard| guard.descriptor.clone()),
                request_id: request_id.clone(),
                method: request.method().to_string(),
                url: request.url().to_string(),
                request_hash,
                request_bytes,
            })
            .await?;
    }
    let mut observed_status = None;
    let mut observed_headers = None;
    let mut received_bytes = 0u64;
    let mut received_hash = None;
    let exchange = async {
        if let Some(journal) = journal {
            journal.check_live()?;
        }
        if let Some(guard) = guard {
            guard.admit(&request)?;
        }
        let mut response = client
            .execute(request)
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        observed_status = Some(status);
        // Only validated headers may contribute to evidence, including when a
        // later size check, read failure or timeout prevents body validation.
        if let Some(guard) = guard {
            guard.response(&headers, &[])?;
        }
        observed_headers = Some(headers.clone());
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err("HTTP response exceeds output limit".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("HTTP response read failed: {e}"))?
        {
            received_bytes = received_bytes.saturating_add(chunk.len() as u64);
            if chunk.len() > limit.saturating_sub(body.len()) {
                return Err("HTTP response exceeds output limit".into());
            }
            body.extend_from_slice(&chunk);
        }
        received_hash = Some(hex::encode(Sha256::digest(&body)));
        if let Some(guard) = guard {
            guard.response(&headers, &body)?;
            if !guard.delivers_body(status) {
                body = br#"{"error":"configured outbound service rejected the request"}"#.to_vec();
            }
        }
        Ok(Response {
            status,
            headers,
            body,
        })
    };
    let result = tokio::time::timeout_at(deadline.into(), async {
        match guard {
            Some(guard) => guard.lease.during(exchange).await,
            None => exchange.await,
        }
    })
    .await
    .unwrap_or_else(|_| Err("HTTP request deadline expired".into()));
    if let Some(journal) = journal {
        let (status, response_hash, response_bytes, error) = match &result {
            Ok(response) => (
                Some(response.status),
                received_hash.clone(),
                received_bytes,
                None,
            ),
            // Transport errors can contain credential-bearing URL data. Keep
            // detailed diagnostics in the existing tool result, not this event.
            Err(_) => (
                observed_status,
                None,
                received_bytes,
                Some("HTTP exchange failed or was incomplete".into()),
            ),
        };
        let response_headers_hash = result
            .as_ref()
            .ok()
            .map(|response| &response.headers)
            .or(observed_headers.as_ref())
            .map(|headers| {
                let mut hash = Sha256::new();
                for (name, value) in headers {
                    hash_field(&mut hash, name.as_str().as_bytes());
                    hash_field(&mut hash, value.as_bytes());
                }
                hex::encode(hash.finalize())
            });
        journal
            .append(ToolEffect::NetworkRequestFinished {
                request_id,
                status,
                response_hash,
                response_headers_hash,
                response_bytes,
                error,
            })
            .await
            .map_err(|e| format!("HTTP outcome was not durably acknowledged: {e}"))?;
    }
    result
}

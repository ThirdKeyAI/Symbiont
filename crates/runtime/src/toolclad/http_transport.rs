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
fn request_identity(request: &reqwest::Request) -> Result<(String, u64), String> {
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
        bytes = bytes.saturating_add(name.as_str().len() + value.as_bytes().len());
        hash_field(&mut hash, name.as_str().as_bytes());
        hash_field(&mut hash, value.as_bytes());
    }
    hash_field(&mut hash, body);
    if bytes > 2 * 1024 * 1024 {
        return Err("HTTP request exceeds 2 MiB application-byte limit".into());
    }
    Ok((hex::encode(hash.finalize()), bytes as u64))
}

pub(super) async fn exchange(
    client: &reqwest::Client,
    request: reqwest::Request,
    journal: Option<&EffectJournal>,
    deadline: Instant,
    limit: usize,
) -> Result<Response, String> {
    if Instant::now() >= deadline || limit == 0 || limit > 10 * 1024 * 1024 {
        return Err("HTTP exchange has an expired deadline or invalid response limit".into());
    }
    let (request_hash, request_bytes) = request_identity(&request)?;
    let request_id = uuid::Uuid::new_v4().to_string();
    if let Some(journal) = journal {
        journal
            .append(ToolEffect::NetworkRequestStarted {
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
    let result = tokio::time::timeout_at(deadline.into(), async {
        if let Some(journal) = journal {
            journal.check_live()?;
        }
        let mut response = client
            .execute(request)
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        observed_status = Some(status);
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
        Ok(Response {
            status,
            headers,
            body,
        })
    })
    .await
    .unwrap_or_else(|_| Err("HTTP request deadline expired".into()));
    if let Some(journal) = journal {
        let (status, response_hash, response_bytes, error) = match &result {
            Ok(response) => (
                Some(response.status),
                Some(hex::encode(Sha256::digest(&response.body))),
                response.body.len() as u64,
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

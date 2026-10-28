//! Optional outbound capabilities. Credentials and execution authority stay
//! in the runtime; this module never grants a worker an independent network.
mod authority;
pub(crate) mod config;

pub(crate) use authority::{Lease, Registry};
pub(crate) use config::FrozenCredential;
pub use config::{Capability, CredentialSource, Profile};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use zeroize::Zeroizing;

pub(crate) struct DispatchGuard {
    pub lease: std::sync::Arc<Lease>,
    pub capability: Capability,
    pub name: String,
    pub credential: Option<std::sync::Arc<Credential>>,
    pub descriptor: serde_json::Value,
    /// The manifest's declared success statuses, in addition to 2xx.
    pub success_status: Vec<u16>,
}

impl DispatchGuard {
    pub fn admit(&self, request: &reqwest::Request) -> Result<(), String> {
        self.capability.check_request(request)?;
        self.lease
            .admit_send(&self.name, self.capability.max_requests)
    }
    /// The credential header and a stable non-secret reference to the
    /// configured credential identity (alias, source and declared version).
    pub fn credential_reference(&self) -> Result<Option<(HeaderName, String)>, String> {
        if self.credential.is_none() {
            return Ok(None);
        }
        let header = HeaderName::from_bytes(self.capability.header.as_bytes())
            .map_err(|_| "invalid credential header")?;
        let alias = self.capability.credential.as_deref().unwrap_or_default();
        let identity = self
            .descriptor
            .get("credential")
            .cloned()
            .unwrap_or_default();
        let digest = crate::reasoning::prepared::digest_json(&identity)?;
        Ok(Some((
            header,
            format!("symbi-outbound-credential-ref-v1:{alias}:{digest}"),
        )))
    }

    /// Bodies of other statuses are replaced with a generic error before
    /// delivery to the worker.
    pub fn delivers_body(&self, status: u16) -> bool {
        (200..300).contains(&status) || self.success_status.contains(&status)
    }

    pub fn response(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), String> {
        if let Some(credential) = &self.credential {
            credential.check_response(headers, body)?;
        }
        self.lease.check_live()
    }
}

/// Deliberately neither serializable nor printable. A resolved credential is
/// cached only within one lease and is never shared across executions. HTTP libraries may copy header buffers internally.
pub struct Credential(Zeroizing<String>);

impl Credential {
    pub fn new(value: String) -> Result<Self, String> {
        let value = Zeroizing::new(value);
        if value.is_empty() || value.len() > 4096 || value.bytes().any(|b| b.is_ascii_control()) {
            return Err("outbound credential is empty, oversized or contains controls".into());
        }
        Ok(Self(value))
    }

    pub(crate) fn inject(
        &self,
        headers: &mut HeaderMap,
        name: &str,
        prefix: &str,
    ) -> Result<(), String> {
        let name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| "invalid credential header")?;
        let value = Zeroizing::new(format!("{prefix}{}", self.0.as_str()));
        let mut value =
            HeaderValue::from_str(&value).map_err(|_| "invalid credential header value")?;
        value.set_sensitive(true);
        headers.insert(name, value);
        Ok(())
    }

    /// Exact reflection protection, not general data-loss prevention. Filtering
    /// precedes delivery to the worker, including fully buffered event streams.
    pub(crate) fn check_response(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), String> {
        let needle = Needle::new(self.0.as_bytes());
        // HTTP normalizes header names to lowercase before this boundary.
        // Compare that representation too; bodies retain exact matching.
        let header_secret = Zeroizing::new(self.0.to_ascii_lowercase());
        let header_needle = Needle::new(header_secret.as_bytes());
        // HTTP strips optional whitespace around field values on the wire.
        let header_value_needle = Needle::new(self.0.trim_matches(' ').as_bytes());
        if needle.found_in(body)
            || headers.iter().any(|(name, value)| {
                header_needle.found_in(name.as_str().as_bytes())
                    || header_value_needle.found_in(value.as_bytes())
            })
        {
            return Err("outbound response contains a protected credential".into());
        }
        Ok(())
    }
}

/// Linear-time exact substring search (Knuth-Morris-Pratt), so scanning a
/// multi-megabyte response for a credential cannot degrade quadratically.
struct Needle<'a> {
    pattern: &'a [u8],
    failure: Vec<usize>,
}

impl<'a> Needle<'a> {
    fn new(pattern: &'a [u8]) -> Self {
        let mut failure = vec![0; pattern.len()];
        let mut k = 0;
        for i in 1..pattern.len() {
            while k > 0 && pattern[i] != pattern[k] {
                k = failure[k - 1];
            }
            if pattern[i] == pattern[k] {
                k += 1;
            }
            failure[i] = k;
        }
        Self { pattern, failure }
    }

    fn found_in(&self, haystack: &[u8]) -> bool {
        if self.pattern.is_empty() {
            return true;
        }
        let mut k = 0;
        for &byte in haystack {
            while k > 0 && byte != self.pattern[k] {
                k = self.failure[k - 1];
            }
            if byte == self.pattern[k] {
                k += 1;
                if k == self.pattern.len() {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod dispatch_tests;

//! Bounded platform HTTP transport. Destinations come from operator configuration,
//! fixed platform endpoints, or an authenticated Bot Framework service URL.
use crate::error::ChannelAdapterError;
use reqwest::{Client, Response, Url};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::time::Duration;

pub(crate) const RECEIPT_LIMIT: usize = 64 * 1024;
#[cfg(feature = "teams")]
pub(crate) const METADATA_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct PreparedPost {
    pub method: &'static str,
    pub url: String,
    pub body: Value,
}

impl PreparedPost {
    pub fn new(url: Url, body: Value) -> Self {
        Self {
            method: "POST",
            url: url.into(),
            body,
        }
    }
}

pub(crate) fn client() -> Result<Client, ChannelAdapterError> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ChannelAdapterError::Internal(format!("HTTP client init: {e}")))
}

/// Reject ambiguous URL syntax before parsing can discard or normalize it.
/// Local HTTP is supported only for operator-configured Mattermost servers.
pub(crate) fn base_url(raw: &str, allow_http: bool) -> Result<Url, ChannelAdapterError> {
    let invalid = || ChannelAdapterError::Config("invalid platform base URL".into());
    if raw.is_empty()
        || raw.len() > 8192
        || raw
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(invalid());
    }
    let url = Url::parse(raw).map_err(|_| invalid())?;
    if !(url.scheme() == "https" || (allow_http && url.scheme() == "http"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

#[cfg(any(feature = "teams", feature = "mattermost", test))]
pub(crate) fn append_segments(mut url: Url, segments: &[&str]) -> Result<Url, ChannelAdapterError> {
    if segments.iter().any(|s| {
        s.is_empty()
            || matches!(*s, "." | "..")
            || s.len() > 8192
            || s.chars().any(char::is_control)
    }) {
        return Err(ChannelAdapterError::Config(
            "invalid platform path segment".into(),
        ));
    }
    url.path_segments_mut()
        .map_err(|_| ChannelAdapterError::Config("platform URL has no path".into()))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

/// Check HTTP status and bound decoded bytes, including chunked responses.
/// Error bodies are deliberately excluded from diagnostics: auth endpoints may
/// echo credentials and platform endpoints may return arbitrary content.
pub(crate) async fn read_json<T: DeserializeOwned>(
    mut response: Response,
    limit: usize,
) -> Result<T, ChannelAdapterError> {
    if !response.status().is_success() {
        return Err(ChannelAdapterError::Connection(format!(
            "platform HTTP status {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(ChannelAdapterError::ParseError(
            "platform response exceeds byte limit".into(),
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ChannelAdapterError::Connection(format!("platform response read: {e}")))?
    {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(ChannelAdapterError::ParseError(
                "platform response exceeds byte limit".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|e| ChannelAdapterError::ParseError(format!("platform JSON response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_and_unsecured_destinations() {
        for raw in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/?q=1",
            "https://example.com/#x",
            " https://example.com",
            "https://example.com\\other",
            "https://example.com/\npath",
        ] {
            assert!(base_url(raw, false).is_err(), "{raw:?}");
        }
        assert!(base_url("http://127.0.0.1:8065/team", true).is_ok());
    }

    #[test]
    fn reply_identifiers_cannot_replace_path_or_query() {
        let url = append_segments(
            base_url("https://example.com/teams/", false).unwrap(),
            &["v3", "conversations", "a/b?#%", "activities", "id"],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/teams/v3/conversations/a%2Fb%3F%23%25/activities/id"
        );
        for id in ["", ".", "..", "x\ny"] {
            assert!(append_segments(url.clone(), &[id]).is_err());
        }
    }
}

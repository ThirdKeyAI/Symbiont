use super::Credential;
use crate::{
    reasoning::prepared::digest_json,
    secrets::{new_secret_store, SecretsConfig},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn read_bounded(path: &Path, limit: usize) -> Result<String, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "cannot open outbound configuration")?;
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect outbound configuration")?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("outbound configuration is not a bounded regular file".into());
    }
    let mut text = String::new();
    file.take(limit as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|_| "cannot read outbound configuration")?;
    if text.len() > limit {
        return Err("outbound configuration exceeds its byte limit".into());
    }
    Ok(text)
}

/// Explicit source selection. A store error never falls back to the environment.
/// The version is operator-declared and must change when its intended credential
/// identity changes. Secret values are not part of policy or approval displays.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialSource {
    Environment {
        variable: String,
        version: String,
    },
    SecretStore {
        config_file: PathBuf,
        key: String,
        version: String,
    },
}

pub(crate) enum FrozenSource {
    Environment(String),
    Store {
        config: Box<SecretsConfig>,
        key: String,
    },
}

pub(crate) struct FrozenCredential {
    source: FrozenSource,
    pub descriptor: serde_json::Value,
    #[cfg(test)]
    pub resolutions: std::sync::atomic::AtomicUsize,
}

impl CredentialSource {
    pub fn validate(&self) -> Result<(), String> {
        let version = match self {
            Self::Environment { variable, version } => {
                if variable.is_empty()
                    || variable.len() > 128
                    || !variable
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                {
                    return Err("credential variable must be an explicit uppercase name".into());
                }
                version
            }
            Self::SecretStore {
                config_file,
                key,
                version,
            } => {
                if !config_file.is_absolute() || !identifier(key) {
                    return Err(
                        "credential store requires an absolute configuration path and a simple key"
                            .into(),
                    );
                }
                version
            }
        };
        if !identifier(version) {
            return Err("credential version is invalid".into());
        }
        Ok(())
    }

    pub(crate) fn freeze(&self) -> Result<FrozenCredential, String> {
        self.validate()?;
        let mut descriptor = serde_json::to_value(self).map_err(|_| "invalid credential source")?;
        let source = match self {
            Self::Environment { variable, .. } => FrozenSource::Environment(variable.clone()),
            Self::SecretStore {
                config_file, key, ..
            } => {
                let text = zeroize::Zeroizing::new(read_bounded(config_file, 64 * 1024)?);
                let mut config: SecretsConfig =
                    toml::from_str(&text).map_err(|_| "invalid credential store configuration")?;
                // Each resolution constructs a fresh store. Retained credential
                // reuse belongs only to the lease, never across assignments.
                // Relative storage paths depend on ambient cwd.
                if let crate::secrets::SecretsBackend::File(file) = &config.backend {
                    if !file.path.is_absolute() {
                        return Err("credential store data path must be absolute".into());
                    }
                }
                config.common.enable_cache = false;
                descriptor["store_configuration_digest"] =
                    serde_json::json!(store_configuration_digest(&config)?);
                FrozenSource::Store {
                    config: Box::new(config),
                    key: key.clone(),
                }
            }
        };
        Ok(FrozenCredential {
            source,
            descriptor,
            #[cfg(test)]
            resolutions: Default::default(),
        })
    }

    pub async fn resolve(&self, agent_id: &str) -> Result<Credential, String> {
        self.freeze()?.resolve(agent_id).await
    }
}

/// Bind the store's configuration and key references, never its authentication
/// values. The declared credential version identifies intentional rotation.
fn store_configuration_digest(config: &SecretsConfig) -> Result<String, String> {
    use crate::secrets::{SecretsBackend, VaultAuthConfig};
    let mut identity = config.clone();
    if let SecretsBackend::Vault(vault) = &mut identity.backend {
        match &mut vault.auth {
            VaultAuthConfig::Token { token } => token.clear(),
            VaultAuthConfig::AppRole { secret_id, .. } => secret_id.clear(),
            VaultAuthConfig::Kubernetes { .. } | VaultAuthConfig::Aws { .. } => {}
        }
    }
    digest_json(&serde_json::to_value(identity).map_err(|_| "invalid store configuration")?)
}

impl FrozenCredential {
    pub async fn resolve(&self, agent_id: &str) -> Result<Credential, String> {
        #[cfg(test)]
        self.resolutions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let value = match &self.source {
            FrozenSource::Environment(variable) => std::env::var(variable)
                .map_err(|_| "configured outbound credential is unavailable")?,
            FrozenSource::Store { config, key } => {
                let store = new_secret_store(config, agent_id)
                    .await
                    .map_err(|_| "outbound secret store is unavailable")?;
                store
                    .get_secret(key)
                    .await
                    .map_err(|_| "outbound secret store refused the credential")?
                    .value()
                    .to_owned()
            }
        };
        Credential::new(value)
    }
}

fn default_header() -> String {
    "authorization".into()
}
fn default_prefix() -> String {
    "Bearer ".into()
}
fn default_calls() -> u32 {
    128
}
fn default_request_bytes() -> usize {
    2 * 1024 * 1024
}
fn default_response_bytes() -> usize {
    4 * 1024 * 1024
}
fn default_timeout() -> u64 {
    60
}

/// A named operation ceiling, intersected with the manifest's validated inputs,
/// exact prepared request and per-call policy/approval. Prefixes are path-segment
/// boundaries, never hostname wildcards or unrestricted proxy destinations.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub tool: String,
    pub origin: String,
    pub method: String,
    pub path_prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default = "default_header")]
    pub header: String,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub private_origin: bool,
    #[serde(default = "default_calls")]
    pub max_requests: u32,
    #[serde(default = "default_request_bytes")]
    pub max_request_bytes: usize,
    #[serde(default = "default_response_bytes")]
    pub max_response_bytes: usize,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Permit `;` path parameters. Off by default: servers disagree about
    /// whether `..;` is a traversal segment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_path_parameters: bool,
    /// When set, every query key must be listed and may appear once. When
    /// unset, the capability does not constrain the query string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_query_keys: Option<Vec<String>>,
}

/// Decode `%XX` escapes; invalid escapes are kept literally.
pub(super) fn percent_decode(segment: &str) -> Vec<u8> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

impl Capability {
    pub fn validate(&self) -> Result<(), String> {
        let origin = url::Url::parse(&self.origin).map_err(|_| "invalid outbound origin")?;
        let canonical = origin.origin().ascii_serialization();
        if canonical != self.origin {
            if canonical.eq_ignore_ascii_case(&self.origin) {
                return Err(format!(
                    "outbound origin must be lowercase; use \"{canonical}\""
                ));
            }
            if let Some(port) = origin.port_or_known_default() {
                if self
                    .origin
                    .trim_end_matches('/')
                    .ends_with(&format!(":{port}"))
                    && origin.port().is_none()
                {
                    return Err(format!(
                        "outbound origin must omit the scheme's default port; use \"{canonical}\""
                    ));
                }
            }
        }
        if self.method == "OPTIONS" {
            return Err(
                "outbound capabilities cannot use OPTIONS; ToolClad cannot dispatch it".into(),
            );
        }
        if !identifier(&self.tool)
            || origin.origin().ascii_serialization() != self.origin
            || !matches!(origin.scheme(), "http" | "https")
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.port_or_known_default() == Some(0)
            || !matches!(
                self.method.as_str(),
                "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
            )
            || !self.path_prefix.starts_with('/')
            || self.path_prefix.len() > 2048
            || self.path_prefix.contains(['%', '?', '#', '\\'])
            || self.path_prefix.split('/').any(|s| matches!(s, "." | ".."))
            || self.path_prefix.bytes().any(|b| b.is_ascii_control())
            || self.max_requests == 0
            || self.max_requests > 1000
            || self.max_request_bytes == 0
            || self.max_request_bytes > 2 * 1024 * 1024
            || self.max_response_bytes == 0
            || self.max_response_bytes > 4 * 1024 * 1024
            || self.timeout_seconds == 0
            || self.timeout_seconds > 120
            || self.prefix.len() > 64
            || self.prefix.bytes().any(|b| b.is_ascii_control())
        {
            return Err("invalid outbound capability".into());
        }
        if let Some(keys) = &self.allowed_query_keys {
            let unique: std::collections::BTreeSet<_> = keys.iter().collect();
            if keys.len() > 64
                || unique.len() != keys.len()
                || keys.iter().any(|key| {
                    key.is_empty()
                        || key.len() > 128
                        || key
                            .bytes()
                            .any(|b| b.is_ascii_control() || matches!(b, b'&' | b'=' | b'#' | b'%'))
                })
            {
                return Err("invalid outbound allowed_query_keys".into());
            }
        }
        let header = reqwest::header::HeaderName::from_bytes(self.header.as_bytes())
            .map_err(|_| "invalid outbound credential header")?;
        if matches!(
            header.as_str(),
            "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "upgrade"
                | "proxy-authorization"
                | "cookie"
                | "content-type"
        ) {
            return Err("outbound credentials cannot control transport headers".into());
        }
        if self.private_origin {
            let permitted = match origin.host() {
                Some(url::Host::Ipv4(ip)) => crate::net_guard::is_private_origin_ip(ip.into()),
                Some(url::Host::Ipv6(ip)) => crate::net_guard::is_private_origin_ip(ip.into()),
                _ => false,
            };
            if !permitted {
                return Err(
                    "private outbound origin must be an exact loopback or private IP origin".into(),
                );
            }
        } else {
            if origin.scheme() != "https" {
                return Err("public outbound capabilities require HTTPS".into());
            }
            crate::net_guard::reject_ssrf_url(origin.as_str())?;
        }
        Ok(())
    }

    pub(crate) fn check_request(&self, request: &reqwest::Request) -> Result<(), String> {
        self.validate()?;
        let url = request.url();
        let path = url.path();
        let lower = path.to_ascii_lowercase();
        let prefix = self.path_prefix.trim_end_matches('/');
        if url.origin().ascii_serialization() != self.origin
            || request.method().as_str() != self.method
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !(path == prefix || path.starts_with(&format!("{prefix}/")))
            || path.contains("//")
            || ["%2f", "%5c", "%2e", "%25", "%00"]
                .iter()
                .any(|p| lower.contains(p))
        {
            return Err("request is outside the outbound capability".into());
        }
        if !self.allow_path_parameters && (path.contains(';') || lower.contains("%3b")) {
            return Err("outbound capability does not permit path parameters".into());
        }
        for segment in path.split('/') {
            // Servers may decode before stripping parameters: `..%3Bx` must
            // be rejected just like `..;x`, even with parameters opted in.
            let decoded = percent_decode(segment);
            let bare = decoded
                .split(|byte| *byte == b';')
                .next()
                .unwrap_or_default();
            if matches!(bare, b"." | b"..") {
                return Err("request is outside the outbound capability".into());
            }
        }
        if let Some(allowed) = &self.allowed_query_keys {
            let mut seen = std::collections::BTreeSet::new();
            for (key, _) in url.query_pairs() {
                if !allowed.iter().any(|name| *name == key) {
                    return Err("outbound request uses a query key outside the capability".into());
                }
                if !seen.insert(key.into_owned()) {
                    return Err("outbound request repeats a query key".into());
                }
            }
        }
        let body = request
            .body()
            .map(|b| {
                b.as_bytes()
                    .ok_or("outbound streaming request body is unsupported")
            })
            .transpose()?
            .unwrap_or_default();
        let header_bytes: usize = request
            .headers()
            .iter()
            .map(|(n, v)| n.as_str().len() + v.as_bytes().len())
            .sum();
        if body
            .len()
            .saturating_add(header_bytes)
            .saturating_add(url.as_str().len())
            > self.max_request_bytes
        {
            return Err("outbound request exceeds its capability byte limit".into());
        }
        for name in [
            "host",
            "content-length",
            "connection",
            "upgrade",
            "transfer-encoding",
            "proxy-authorization",
            "proxy-connection",
        ] {
            if request.headers().contains_key(name) {
                return Err("outbound request contains a forbidden transport header".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub tenant: String,
    pub workspace: String,
    #[serde(default)]
    pub credentials: BTreeMap<String, CredentialSource>,
    pub capabilities: BTreeMap<String, Capability>,
    #[serde(skip)]
    frozen: BTreeMap<String, Arc<FrozenCredential>>,
}

impl Profile {
    pub fn load(project: &Path) -> Result<Option<Self>, String> {
        let path = project.join("symbiont.toml");
        if !path
            .try_exists()
            .map_err(|_| "cannot inspect project configuration")?
        {
            return Ok(None);
        }
        let text = read_bounded(&path, 1024 * 1024)?;
        let root: toml::Value =
            toml::from_str(&text).map_err(|_| "invalid project configuration")?;
        let Some(value) = root.get("outbound") else {
            return Ok(None);
        };
        let mut profile: Self = value
            .clone()
            .try_into()
            .map_err(|_| "invalid outbound profile; check schema and supported fields")?;
        if profile.schema_version != 1
            || !identifier(&profile.tenant)
            || !identifier(&profile.workspace)
            || profile.credentials.len() > 128
            || profile.capabilities.is_empty()
            || profile.capabilities.len() > 128
        {
            return Err("invalid outbound profile version, scope or capacity".into());
        }
        for (name, source) in &profile.credentials {
            if !identifier(name) {
                return Err("invalid outbound credential alias".into());
            }
            profile
                .frozen
                .insert(name.clone(), Arc::new(source.freeze()?));
        }
        for (name, capability) in &profile.capabilities {
            if !identifier(name) {
                return Err("invalid outbound capability name".into());
            }
            capability.validate()?;
            if capability
                .credential
                .as_ref()
                .is_some_and(|name| !profile.credentials.contains_key(name))
            {
                return Err("outbound capability names an unknown credential".into());
            }
        }
        Ok(Some(profile))
    }

    pub(crate) fn credential(
        &self,
        capability: &Capability,
    ) -> Result<Option<Arc<FrozenCredential>>, String> {
        capability
            .credential
            .as_ref()
            .map(|name| {
                self.frozen
                    .get(name)
                    .cloned()
                    .ok_or_else(|| "outbound credential was not frozen".into())
            })
            .transpose()
    }

    pub(crate) fn descriptor(&self) -> serde_json::Value {
        let credentials: BTreeMap<_, _> = self
            .frozen
            .iter()
            .map(|(name, source)| (name, &source.descriptor))
            .collect();
        serde_json::json!({"schema_version": self.schema_version, "tenant": self.tenant, "workspace": self.workspace, "capabilities": self.capabilities, "credentials": credentials})
    }
}

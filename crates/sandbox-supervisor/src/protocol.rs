use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub const VERSION: u32 = 4;
pub const IMPLEMENTATION: &str = env!("SYMBI_SUPERVISOR_IMPLEMENTATION");
pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_ENVIRONMENT: usize = 64 * 1024;
pub const SOCKET: &str = "supervisor.sock";
pub const LABEL: &str = "ai.symbiont.lease";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub version: u32,
    pub implementation: String,
    pub lease: uuid::Uuid,
    pub name: String,
    pub docker_binary: PathBuf,
    pub docker_environment: HashMap<String, String>,
    /// Complete literal argv beginning with `create`. The supervisor inserts
    /// only its reserved ownership label and private environment-file argument.
    pub arguments: Vec<String>,
    pub environment: HashMap<String, String>,
    pub lifetime_ms: u64,
    pub startup_ms: u64,
    pub resources: crate::admission::WorkerResources,
    pub staging: Vec<uuid::Uuid>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Ping {
        version: u32,
        implementation: String,
    },
    Inspect {
        version: u32,
        implementation: String,
        lease: Option<uuid::Uuid>,
    },
    Create(Box<Create>),
    CreateVm(Box<CreateVm>),
    Close {},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Capacity {
        snapshot: crate::inspection::CapacitySnapshot,
    },
    Measurement {
        measurement: crate::inspection::WorkerMeasurement,
    },
    Ready {
        version: u32,
        implementation: String,
        jailed_vms: bool,
    },
    Registered {
        lease: uuid::Uuid,
    },
    Created {
        id: String,
    },
    VmCreated {
        pid: u32,
        vsock_path: PathBuf,
    },
    Closed {},
    /// Before Registered, this explicitly proves this request started no
    /// worker. After Registered, only Closed can confirm cleanup.
    Failed {
        message: String,
    },
}

/// Only VM configuration crosses the supervisor control channel. Workload
/// argv, input and environment travel on the separate private guest channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateVm {
    pub version: u32,
    pub implementation: String,
    pub lease: uuid::Uuid,
    pub binary: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub boot_args: String,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub lifetime_ms: u64,
    pub startup_ms: u64,
}
impl CreateVm {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.version != VERSION || self.implementation != IMPLEMENTATION {
            anyhow::bail!("unsupported VM supervisor protocol or implementation");
        }
        for path in [&self.binary, &self.kernel, &self.rootfs] {
            if !path.is_absolute()
                || path.to_str().is_none()
                || path.as_os_str().as_encoded_bytes().contains(&0)
                || path
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir))
            {
                anyhow::bail!("VM artifact paths must be resolved absolute paths");
            }
        }
        if self.boot_args.len() > 4096
            || self.boot_args.contains('\0')
            || !(1..=32).contains(&self.vcpus)
            || !(64..=16384).contains(&self.memory_mib)
            || self.lifetime_ms == 0
            || self.lifetime_ms > 86_400_000
            || self.startup_ms == 0
            || self.startup_ms > self.lifetime_ms
        {
            anyhow::bail!("invalid VM configuration or lifetime");
        }
        Ok(())
    }
    pub fn vm_config(&self, vsock: &Path) -> serde_json::Value {
        serde_json::json!({
            "boot-source":{"kernel_image_path":self.kernel,"boot_args":self.boot_args},
            "drives":[{"drive_id":"rootfs","path_on_host":self.rootfs,"is_root_device":true,"is_read_only":true}],
            "machine-config":{"vcpu_count":self.vcpus,"mem_size_mib":self.memory_mib,"smt":false},
            "vsock":{"guest_cid":3,"uds_path":vsock},
        })
    }
}

pub fn validate_environment(environment: &HashMap<String, String>) -> anyhow::Result<()> {
    let mut size: usize = 0;
    for (key, value) in environment {
        size = size
            .saturating_add(key.len())
            .saturating_add(value.len())
            .saturating_add(2);
        if key.is_empty()
            || key.starts_with('#')
            || key.contains(['=', '\0', '\r', '\n'])
            || key.chars().any(char::is_whitespace)
            || value.contains(['\0', '\r', '\n'])
        {
            anyhow::bail!("invalid container environment framing");
        }
    }
    if size > MAX_ENVIRONMENT {
        anyhow::bail!("container environment exceeds 64 KiB");
    }
    Ok(())
}

pub fn validate_daemon_environment(environment: &HashMap<String, String>) -> anyhow::Result<()> {
    validate_environment(environment)?;
    if environment.keys().any(|key| {
        !matches!(
            key.as_str(),
            "PATH"
                | "HOME"
                | "LANG"
                | "DOCKER_HOST"
                | "DOCKER_CONTEXT"
                | "DOCKER_CONFIG"
                | "DOCKER_CERT_PATH"
                | "DOCKER_TLS_VERIFY"
                | "DOCKER_API_VERSION"
        )
    }) {
        anyhow::bail!("unapproved Docker client environment variable");
    }
    Ok(())
}

impl Create {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.resources.validate()?;
        validate_staging_ids(&self.staging)?;
        if self.version != VERSION || self.implementation != IMPLEMENTATION {
            anyhow::bail!("unsupported supervisor protocol or implementation");
        }
        if self.name != format!("symbi-{}", self.lease) {
            anyhow::bail!("container name does not match lease identity");
        }
        if !self.docker_binary.is_absolute() || self.docker_binary.to_str().is_none() {
            anyhow::bail!("Docker client must have a resolved absolute path");
        }
        if self.lifetime_ms == 0
            || self.lifetime_ms > 86_400_000
            || self.startup_ms == 0
            || self.startup_ms > self.lifetime_ms
        {
            anyhow::bail!("invalid container lifetime or startup budget");
        }
        if self.arguments.first().map(String::as_str) != Some("create")
            || self.arguments.get(1).map(String::as_str) != Some("--interactive")
            || self.arguments.get(2).map(String::as_str) != Some("--name")
            || self.arguments.get(3) != Some(&self.name)
            || self.arguments.iter().any(|a| a.contains('\0'))
        {
            anyhow::bail!("invalid stopped-container creation argv");
        }
        validate_environment(&self.environment)?;
        validate_daemon_environment(&self.docker_environment)?;
        Ok(())
    }
}

pub fn socket_path(root: &Path) -> anyhow::Result<PathBuf> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        anyhow::bail!("supervisor state directory must be absolute without traversal");
    }
    let path = root.join(SOCKET);
    if path.as_os_str().as_encoded_bytes().len() >= 104 {
        anyhow::bail!("supervisor socket path exceeds the supported Unix path length");
    }
    Ok(path)
}

/// Bounded newline frames. Retain this future for the whole frame, or close the
/// connection on timeout; restarting a partially consumed frame is forbidden.
pub async fn read_frame<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncBufRead + Unpin),
) -> anyhow::Result<Option<T>> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            anyhow::bail!("truncated supervisor frame");
        }
        let used = buffer
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buffer.len(), |p| p + 1);
        let complete = buffer[used - 1] == b'\n';
        if bytes.len().saturating_add(used) > MAX_FRAME {
            anyhow::bail!("supervisor frame exceeds 1 MiB");
        }
        bytes.extend_from_slice(&buffer[..used]);
        reader.consume(used);
        if complete {
            return Ok(Some(serde_json::from_slice(&bytes)?));
        }
    }
}

pub async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &impl Serialize,
) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() >= MAX_FRAME {
        anyhow::bail!("supervisor frame exceeds 1 MiB");
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Bound durable references even for malformed or unsupported callers.
pub fn validate_staging_ids(ids: &[uuid::Uuid]) -> anyhow::Result<()> {
    anyhow::ensure!(
        ids.len() <= 32 && ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len(),
        "invalid staging references"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_reject_truncation_oversize_and_unknown_fields() {
        for bytes in [
            br#"{"operation":"close"}"#.to_vec(),
            vec![b' '; MAX_FRAME + 1],
            b"{\"operation\":\"close\",\"extra\":true}\n".to_vec(),
        ] {
            let mut reader = tokio::io::BufReader::new(bytes.as_slice());
            assert!(read_frame::<Request>(&mut reader).await.is_err());
        }
        let mut reader = &b"{\"operation\":\"close\"}\n{\"operation\":\"close\"}\n"[..];
        assert!(matches!(
            read_frame::<Request>(&mut reader).await.unwrap(),
            Some(Request::Close {})
        ));
        assert!(matches!(
            read_frame::<Request>(&mut reader).await.unwrap(),
            Some(Request::Close {})
        ));
        assert!(read_frame::<Request>(&mut reader).await.unwrap().is_none());
    }

    #[test]
    fn environment_rejects_smuggling_ambient_variables_and_oversize() {
        for (key, value) in [
            ("#KEY", "value"),
            ("A=B", "value"),
            ("KEY", "value\nOTHER=x"),
            ("KEY", "value\0"),
        ] {
            assert!(validate_environment(&HashMap::from([(key.into(), value.into())])).is_err());
        }
        assert!(validate_environment(&HashMap::from([(
            "KEY".into(),
            "x".repeat(MAX_ENVIRONMENT)
        )]))
        .is_err());
        assert!(validate_daemon_environment(&HashMap::from([(
            "SECRET_KEY".into(),
            "synthetic".into()
        )]))
        .is_err());
        assert!(
            validate_environment(&HashMap::from([("KEY".into(), "a=b $literal".into())])).is_ok()
        );
    }

    #[test]
    fn socket_paths_are_absolute_bounded_and_without_traversal() {
        for path in ["relative", "/tmp/../state"] {
            assert!(socket_path(Path::new(path)).is_err());
        }
        assert!(socket_path(Path::new(&format!("/tmp/{}", "x".repeat(104)))).is_err());
    }

    #[test]
    fn creation_requires_matching_implementation_identity_and_budget() {
        let lease = uuid::Uuid::new_v4();
        let name = format!("symbi-{lease}");
        let valid = Create {
            staging: Vec::new(),
            version: VERSION,
            implementation: IMPLEMENTATION.into(),
            lease,
            name: name.clone(),
            docker_binary: "/usr/bin/docker".into(),
            docker_environment: HashMap::new(),
            environment: HashMap::new(),
            arguments: vec![
                "create".into(),
                "--interactive".into(),
                "--name".into(),
                name,
            ],
            startup_ms: 1000,
            lifetime_ms: 2000,
            resources: crate::admission::WorkerResources::vm(64, 1),
        };
        assert!(valid.validate().is_ok());
        let mut changed = valid.clone();
        changed.implementation = "older-source".into();
        assert!(changed.validate().is_err());
        let mut changed = valid.clone();
        changed.lease = uuid::Uuid::new_v4();
        assert!(changed.validate().is_err());
        let mut changed = valid.clone();
        changed.arguments[0] = "run".into();
        assert!(changed.validate().is_err());
        let mut changed = valid;
        changed.startup_ms = changed.lifetime_ms + 1;
        assert!(changed.validate().is_err());
    }
}

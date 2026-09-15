//! Runtime-issued guest capabilities for the current managed tool session.
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const TOOLS_PORT: u32 = 4051;
pub const INFERENCE_PORT: u32 = 4052;

/// Created only by the runtime broker, never by deserializing project input.
/// The socket paths are included in the prepared boundary's content digest.
#[derive(Debug, Clone, Serialize)]
pub struct GuestServices {
    pub(crate) tools: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) inference: Option<PathBuf>,
}

impl GuestServices {
    fn endpoints(&self) -> impl Iterator<Item = (u32, &Path)> {
        std::iter::once((TOOLS_PORT, self.tools.as_path()))
            .chain(self.inference.as_deref().map(|path| (INFERENCE_PORT, path)))
    }

    pub fn ports(&self) -> Vec<u32> {
        self.endpoints().map(|(port, _)| port).collect()
    }

    pub(super) fn validate(&self) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt};
            for (_, path) in self.endpoints() {
                let metadata = std::fs::symlink_metadata(path)?;
                // SAFETY: geteuid has no pointer preconditions.
                if !path.is_absolute()
                    || !metadata.file_type().is_socket()
                    || metadata.uid() != unsafe { libc::geteuid() }
                {
                    anyhow::bail!("guest service requires a runtime-owned local broker socket");
                }
            }
            Ok(())
        }
        #[cfg(not(unix))]
        anyhow::bail!("guest services require Linux local sockets")
    }

    #[cfg(unix)]
    pub(super) fn install(&self, vsock: &Path) -> anyhow::Result<()> {
        self.validate()?;
        for (port, target) in self.endpoints() {
            // Firecracker maps guest-initiated CID 2/port connections to this
            // exact Unix pathname. Only these two runtime-issued capabilities
            // exist in the private lease directory; no host network is exposed.
            let mut path = vsock.as_os_str().to_owned();
            path.push(format!("_{port}"));
            std::os::unix::fs::symlink(target, PathBuf::from(path))?;
        }
        // The independent supervisor removes these links with the VM directory.
        // remove_dir_all does not traverse their external socket targets.
        Ok(())
    }
}

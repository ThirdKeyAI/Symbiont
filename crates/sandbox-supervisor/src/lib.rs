//! Independent ownership of stopped-container creation, lifetimes and recovery.
//! The control socket and lease store belong to the operator, outside workers.

pub mod admission;
#[cfg(unix)]
pub mod host;
pub mod inspection;
pub mod protocol;
#[cfg(unix)]
pub mod service;
#[cfg(unix)]
pub mod staging;
#[cfg(unix)]
mod store;

pub const INTERNAL_COMMAND: &str = "__sandbox_supervisor";

/// Called by the standalone binary and the shipping CLI before loading project
/// configuration or environment files. The service receives no model prompts.
pub async fn run(arguments: &[String]) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    if arguments.len() == 2 && arguments[0] == "--host-reap" {
        let profile = host::HostProfile::load_for_recovery(std::path::Path::new(&arguments[1]))?;
        return service::reap_host(profile).await;
    }
    #[cfg(target_os = "linux")]
    if arguments.len() == 2 && arguments[0] == "--host-profile" {
        let profile = host::HostProfile::load(std::path::Path::new(&arguments[1]))?;
        return service::serve_host(profile).await;
    }
    let persistent = arguments.len() == 3 && arguments[2] == "--persistent";
    if (arguments.len() != 2 && !persistent) || arguments[0] != "--state-dir" {
        anyhow::bail!("expected --state-dir /absolute/private/directory [--persistent]");
    }
    #[cfg(unix)]
    return service::serve(std::path::Path::new(&arguments[1]), persistent).await;
    #[cfg(not(unix))]
    anyhow::bail!("container supervision requires Unix local sockets");
}

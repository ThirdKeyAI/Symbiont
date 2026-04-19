//! E2E-9: Sandbox volume refusal.
//!
//! `DockerConfig::with_volume` / `validate()` must refuse mounts that
//! would punch a hole through the sandbox. No container is spawned —
//! this exercises the config layer directly because that's where the
//! security decision lives.

#![cfg(feature = "e2e")]

use symbi_runtime::sandbox::DockerConfig;

fn base() -> DockerConfig {
    DockerConfig::for_image("python:3.12-slim")
}

#[test]
fn refuses_docker_socket_mount() {
    assert!(base().with_volume("/var/run/docker.sock:/sock").is_err());
    assert!(base().with_volume("/run/docker.sock:/sock").is_err());
}

#[test]
fn refuses_etc_mount() {
    assert!(base().with_volume("/etc:/data:ro").is_err());
    assert!(base().with_volume("/etc/passwd:/data:ro").is_err());
}

#[test]
fn refuses_proc_sys_mounts() {
    assert!(base().with_volume("/proc:/proc").is_err());
    assert!(base().with_volume("/sys:/sys").is_err());
}

#[test]
fn refuses_host_root() {
    assert!(base().with_volume("/:/host").is_err());
}

#[test]
fn refuses_traversal_segments() {
    assert!(base().with_volume("/home/../etc:/mnt").is_err());
    assert!(base().with_volume("/opt/..//etc:/mnt").is_err());
}

#[test]
fn refuses_kubelet_and_rancher_paths() {
    assert!(base().with_volume("/var/lib/kubelet:/x").is_err());
    assert!(base().with_volume("/var/lib/rancher:/x").is_err());
    assert!(base().with_volume("/var/lib/docker:/x").is_err());
}

#[test]
fn allows_named_volume() {
    assert!(base().with_volume("my-volume:/data").is_ok());
}

#[test]
fn allows_plain_host_path_with_container_path() {
    // Under /tmp or /home/user/... these should be fine; we don't
    // check existence at config time.
    assert!(base().with_volume("/tmp/sandbox:/data").is_ok());
    assert!(base().with_volume("/home/user/code:/workspace:ro").is_ok());
}

#[test]
fn validate_refuses_injected_dangerous_volume() {
    // Operator bypasses the builder by pushing directly into `volumes`;
    // validate() must still refuse.
    let mut cfg = base();
    cfg.volumes.push("/var/run/docker.sock:/sock".to_string());
    assert!(cfg.validate().is_err());
}

#[test]
fn refuses_volume_without_container_path() {
    assert!(base().with_volume("/data").is_err());
    assert!(base().with_volume("").is_err());
}

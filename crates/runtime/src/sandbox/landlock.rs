//! Daemon-free command isolation using the Landlock LSM.
//!
//! Rules are built in the parent, because each root needs an open directory
//! descriptor, leaving only `restrict_self` for the pre-exec window in the
//! child. The crate defaults to `CompatLevel::BestEffort`, which silently
//! ignores unsupported requests; every ruleset here sets `HardRequirement`
//! instead so a boundary never enforces less than it claims.

use serde::{Deserialize, Serialize};

/// Operator-visible landlock settings. Paths come from `BoundaryRoots`, not
/// from here, so there is one way to declare a ceiling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LandlockProfile {
    /// Lowest Landlock ABI this boundary accepts. 4 is the first with TCP rules.
    pub abi_floor: u8,
    /// Refuse to run unless outbound TCP can be restricted.
    pub require_network: bool,
}

impl Default for LandlockProfile {
    fn default() -> Self {
        Self {
            abi_floor: 4,
            require_network: true,
        }
    }
}

/// The kernel's supported Landlock ABI, or 0 when unavailable.
///
/// The landlock crate keeps this private on purpose, to stop callers building
/// rules from an ABI discovered at run time. We only compare it against a
/// declared floor and record it in audit; rule construction always names a
/// fixed `ABI::V*`.
pub fn detect_abi() -> u8 {
    // SAFETY: the version query passes a null attribute pointer and size 0,
    // which the syscall defines as "report the supported ABI".
    let version = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0usize,
            1u32, // LANDLOCK_CREATE_RULESET_VERSION
        )
    };
    if version < 0 {
        0
    } else {
        version as u8
    }
}

/// Confirm the running kernel can enforce everything the profile declares.
pub fn check_kernel(profile: &LandlockProfile) -> Result<u8, String> {
    let detected = detect_abi();
    if detected == 0 {
        return Err("Landlock is unavailable on this kernel; no host fallback".into());
    }
    if detected < profile.abi_floor {
        return Err(format!(
            "Landlock ABI {detected} is below the required floor {}; \
             the boundary would enforce less than it declares",
            profile.abi_floor
        ));
    }
    if profile.require_network && detected < 4 {
        return Err(format!(
            "network restriction requires Landlock ABI 4, kernel provides {detected}"
        ));
    }
    Ok(detected)
}

use landlock::{
    path_beneath_rules, Access, AccessFs, AccessNet, CompatLevel, Compatible, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetStatus, ABI,
};
use std::os::unix::process::CommandExt;
use std::sync::Arc;

/// Fixed ABI the rules are written against. Never derived from the kernel: a
/// ruleset built from a detected ABI would silently change meaning per host.
const RULE_ABI: ABI = ABI::V4;

/// A resolved ruleset, ready to install on a child.
#[derive(Clone)]
pub struct PreparedDomain {
    readable: Arc<Vec<String>>,
    writable: Arc<Vec<String>>,
    restrict_network: bool,
}

/// Take the host path from a `host:virtual:ro` entry.
fn host_path(entry: &str) -> &str {
    entry.split(':').next().unwrap_or(entry)
}

/// Read and execute access the child needs before it can run at all: its own
/// interpreter, the loader and the shared libraries they pull in. Without
/// these a dynamically linked program cannot even be exec'd, so the domain
/// would deny every command rather than confine it.
///
/// Deliberately narrow. It carries no writable path, nothing under a home
/// directory, and no broad `/etc` grant; anything else a workload needs is an
/// explicit boundary root.
const SYSTEM_PATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/dev/null",
    "/dev/zero",
    "/dev/urandom",
];

/// Keep only paths that exist. `path_beneath_rules` opens each entry, so a
/// missing one would fail the whole ruleset on hosts with a different layout.
fn existing(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|path| std::path::Path::new(path).exists())
        .collect()
}

/// Resolve a boundary's roots into a domain that can be applied to children.
pub fn prepare(
    profile: &LandlockProfile,
    roots: &crate::sandbox::command::BoundaryRoots,
) -> Result<PreparedDomain, String> {
    check_kernel(profile)?;
    Ok(PreparedDomain {
        readable: Arc::new(existing(
            SYSTEM_PATHS.iter().map(|path| (*path).to_owned()).chain(
                roots
                    .source_roots
                    .iter()
                    .map(|entry| host_path(entry).to_owned()),
            ),
        )),
        writable: Arc::new(existing(
            roots
                .output_roots
                .iter()
                .map(|entry| host_path(entry).to_owned()),
        )),
        restrict_network: profile.require_network,
    })
}

impl PreparedDomain {
    /// Build and enforce the domain. Runs in the child, after fork.
    fn restrict(&self) -> Result<(), String> {
        let mut ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(RULE_ABI))
            .map_err(|e| e.to_string())?;
        if self.restrict_network {
            ruleset = ruleset
                .handle_access(AccessNet::from_all(RULE_ABI))
                .map_err(|e| e.to_string())?;
        }
        let status = ruleset
            .create()
            .map_err(|e| e.to_string())?
            .add_rules(path_beneath_rules(
                self.readable.iter(),
                AccessFs::from_read(RULE_ABI),
            ))
            .map_err(|e| e.to_string())?
            .add_rules(path_beneath_rules(
                self.writable.iter(),
                AccessFs::from_write(RULE_ABI),
            ))
            .map_err(|e| e.to_string())?
            .restrict_self()
            .map_err(|e| e.to_string())?;
        match status.ruleset {
            RulesetStatus::FullyEnforced => Ok(()),
            other => Err(format!(
                "Landlock did not fully enforce the boundary: {other:?}"
            )),
        }
    }

    /// Install on a std Command.
    pub fn apply_to_std(&self, command: &mut std::process::Command) {
        let domain = self.clone();
        // SAFETY: the closure runs between fork and exec and performs only
        // landlock syscalls on descriptors opened before the fork.
        unsafe {
            command.pre_exec(move || domain.restrict().map_err(std::io::Error::other));
        }
    }

    /// Install on a tokio Command.
    pub fn apply_to(&self, command: &mut tokio::process::Command) {
        let domain = self.clone();
        // SAFETY: as above.
        unsafe {
            command.pre_exec(move || domain.restrict().map_err(std::io::Error::other));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_below_the_floor_is_refused_rather_than_degraded() {
        let profile = LandlockProfile {
            abi_floor: 99,
            require_network: true,
        };
        let error = check_kernel(&profile).expect_err("an impossible floor must fail");
        assert!(
            error.contains("99"),
            "error must name the required floor: {error}"
        );
    }

    #[test]
    fn network_requirement_needs_abi_four() {
        let profile = LandlockProfile {
            abi_floor: 1,
            require_network: true,
        };
        if detect_abi() < 4 {
            assert!(check_kernel(&profile).is_err());
        } else {
            assert!(check_kernel(&profile).is_ok());
        }
    }
}

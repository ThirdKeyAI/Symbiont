//! Aggregate worker capacity owned by the supervisor, including unresolved leases.
use serde::{Deserialize, Serialize};

pub const MAX_WORKERS: usize = 512;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerResources {
    pub memory_bytes: u64,
    /// One CPU is 1,000,000,000 units, matching Docker NanoCpus.
    pub cpu_nanos: u64,
}

impl WorkerResources {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.memory_bytes == 0 || self.cpu_nanos == 0 {
            anyhow::bail!("worker admission requires positive memory and CPU limits");
        }
        Ok(())
    }

    pub fn docker(memory: &str, cpus: f64) -> anyhow::Result<Self> {
        let digits = memory.trim_end_matches(['b', 'k', 'm', 'g', 'B', 'K', 'M', 'G']);
        if digits.is_empty()
            || memory.len() - digits.len() > 1
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            anyhow::bail!("invalid admission memory limit");
        }
        let multiplier = match memory
            .get(digits.len()..)
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "b" => 1,
            "k" => 1024,
            "m" => 1024 * 1024,
            "g" => 1024 * 1024 * 1024,
            _ => anyhow::bail!("invalid admission memory suffix"),
        };
        let memory_bytes = digits
            .parse::<u64>()?
            .checked_mul(multiplier)
            .ok_or_else(|| anyhow::anyhow!("admission memory limit overflow"))?;
        if !cpus.is_finite() || cpus <= 0.0 || cpus > 1024.0 {
            anyhow::bail!("invalid admission CPU limit");
        }
        let resources = Self {
            memory_bytes,
            cpu_nanos: (cpus * 1_000_000_000.0).ceil() as u64,
        };
        resources.validate()?;
        Ok(resources)
    }

    pub fn vm(memory_mib: u32, vcpus: u8) -> Self {
        Self {
            memory_bytes: u64::from(memory_mib) * 1024 * 1024,
            cpu_nanos: u64::from(vcpus) * 1_000_000_000,
        }
    }
}

/// Trusted capacity configuration, stored in the supervisor's private directory.
/// Limits describe admitted workload quotas; host/VMM overhead additionally
/// requires OS controls and a capacity reserve in the deployment profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionLimits {
    pub max_workers: usize,
    pub memory_bytes: u64,
    pub cpu_nanos: u64,
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            max_workers: 16,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            cpu_nanos: 8 * 1_000_000_000,
        }
    }
}

impl AdmissionLimits {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_workers == 0
            || self.max_workers > MAX_WORKERS
            || self.memory_bytes == 0
            || self.cpu_nanos == 0
        {
            anyhow::bail!("invalid supervisor admission limits");
        }
        Ok(())
    }

    pub fn admit(
        &self,
        existing: impl IntoIterator<Item = Option<WorkerResources>>,
        requested: WorkerResources,
    ) -> anyhow::Result<()> {
        self.validate()?;
        requested.validate()?;
        let mut workers = 1usize;
        let mut memory = requested.memory_bytes;
        let mut cpus = requested.cpu_nanos;
        for resources in existing {
            // Old records without resource metadata must reconcile before new
            // work. An uncertain worker never has an implicit zero charge.
            let resources = resources.ok_or_else(|| {
                anyhow::anyhow!("worker capacity unknown; reconcile legacy leases before admission")
            })?;
            resources.validate()?;
            workers = workers
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("worker count overflow"))?;
            memory = memory
                .checked_add(resources.memory_bytes)
                .ok_or_else(|| anyhow::anyhow!("worker memory accounting overflow"))?;
            cpus = cpus
                .checked_add(resources.cpu_nanos)
                .ok_or_else(|| anyhow::anyhow!("worker CPU accounting overflow"))?;
        }
        if workers > self.max_workers || memory > self.memory_bytes || cpus > self.cpu_nanos {
            anyhow::bail!("shared worker capacity exhausted: workers={workers}/{}, memory_bytes={memory}/{}, cpu_nanos={cpus}/{}", self.max_workers, self.memory_bytes, self.cpu_nanos);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_workers_share_all_limits_and_unknown_capacity_refuses() {
        let limits = AdmissionLimits {
            max_workers: 2,
            memory_bytes: 128 * 1024 * 1024,
            cpu_nanos: 1_000_000_000,
        };
        let docker = WorkerResources::docker("64m", 0.5).unwrap();
        assert!(limits.admit([Some(docker)], docker).is_ok());
        assert!(limits.admit([Some(docker), Some(docker)], docker).is_err());
        assert!(limits
            .admit([Some(docker)], WorkerResources::vm(64, 1))
            .is_err());
        assert!(limits
            .admit([Some(docker)], WorkerResources::docker("65m", 0.5).unwrap())
            .is_err());
        assert!(limits.admit([None], docker).is_err());
    }

    #[test]
    fn resource_parsing_rejects_overflow_zero_and_nonfinite_values() {
        for memory in ["0", "1mm", "-2g", "18446744073709551615g", ""] {
            assert!(WorkerResources::docker(memory, 1.0).is_err());
        }
        for cpus in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            assert!(WorkerResources::docker("1g", cpus).is_err());
        }
        assert_eq!(
            WorkerResources::docker("1G", 1.0).unwrap(),
            WorkerResources::vm(1024, 1)
        );
        let limits = AdmissionLimits {
            max_workers: 2,
            memory_bytes: u64::MAX,
            cpu_nanos: u64::MAX,
        };
        let huge = WorkerResources {
            memory_bytes: u64::MAX,
            cpu_nanos: 1,
        };
        assert!(limits.admit([Some(huge)], huge).is_err());
    }
}

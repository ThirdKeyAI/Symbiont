//! Operator snapshots of retained capacity and separately sampled worker usage.
use crate::admission::{AdmissionLimits, WorkerResources};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityTotals {
    pub workers: usize,
    pub memory_bytes: Option<u64>,
    pub cpu_nanos: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub lease: uuid::Uuid,
    pub backend: String,
    pub phase: String,
    pub resources: Option<WorkerResources>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacitySnapshot {
    pub observed_at_unix_ms: u64,
    pub state_dir: PathBuf,
    pub limits: AdmissionLimits,
    pub reserved: CapacityTotals,
    pub available: CapacityTotals,
    pub unknown_resource_leases: usize,
    pub admission_blocked: bool,
    pub workers: Vec<WorkerSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerMeasurement {
    pub lease: uuid::Uuid,
    pub observed_at_unix_ms: u64,
    pub source: String,
    /// Docker CLI fields retain its displayed precision and cache adjustment.
    pub cpu_percent: Option<String>,
    pub memory_usage: Option<String>,
    /// VMM process/cgroup counters are not guest application measurements.
    pub cpu_time_micros: Option<u64>,
    pub memory_bytes: Option<u64>,
}

#[cfg(unix)]
mod local {
    use super::*;
    use crate::{
        service::{self, State},
        store::{Phase, Record},
    };
    use anyhow::Context;
    use std::{
        fs::OpenOptions,
        io::Read,
        os::unix::fs::OpenOptionsExt,
        path::Path,
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Semaphore;

    static READERS: Semaphore = Semaphore::const_new(2);
    static SAMPLERS: Semaphore = Semaphore::const_new(2);

    fn now() -> anyhow::Result<u64> {
        Ok(SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .try_into()?)
    }

    pub(crate) async fn capacity(state: &State) -> anyhow::Result<CapacitySnapshot> {
        let _permit = READERS
            .try_acquire()
            .context("capacity inspection is busy")?;
        state
            .storage(|store| {
                snapshot(
                    store.root.clone(),
                    store.admission_limits(),
                    store.records()?,
                )
            })
            .await
    }

    fn snapshot(
        root: PathBuf,
        limits: AdmissionLimits,
        mut records: Vec<Record>,
    ) -> anyhow::Result<CapacitySnapshot> {
        records.sort_by_key(|record| record.lease);
        let unknown = records.iter().filter(|r| r.resources.is_none()).count();
        let (mut memory, mut cpu) = (0u64, 0u64);
        for resources in records.iter().filter_map(|r| r.resources) {
            memory = memory
                .checked_add(resources.memory_bytes)
                .context("reserved memory overflow")?;
            cpu = cpu
                .checked_add(resources.cpu_nanos)
                .context("reserved CPU overflow")?;
        }
        let blocked = unknown > 0
            || records.len() >= limits.max_workers
            || memory >= limits.memory_bytes
            || cpu >= limits.cpu_nanos;
        let reserved = CapacityTotals {
            workers: records.len(),
            memory_bytes: (unknown == 0).then_some(memory),
            cpu_nanos: (unknown == 0).then_some(cpu),
        };
        let available = CapacityTotals {
            workers: limits.max_workers.saturating_sub(records.len()),
            memory_bytes: (unknown == 0).then_some(limits.memory_bytes.saturating_sub(memory)),
            cpu_nanos: (unknown == 0).then_some(limits.cpu_nanos.saturating_sub(cpu)),
        };
        let workers = records
            .into_iter()
            .map(|r| {
                let (backend, phase) = match r.state {
                    Phase::Creating => ("container", "creating"),
                    Phase::Created { .. } => ("container", "created"),
                    Phase::Uncertain => ("container", "uncertain"),
                    Phase::VmCreating => ("firecracker", "creating"),
                    Phase::VmCreated { .. } => ("firecracker", "created"),
                };
                WorkerSnapshot {
                    lease: r.lease,
                    backend: backend.into(),
                    phase: phase.into(),
                    resources: r.resources,
                }
            })
            .collect();
        Ok(CapacitySnapshot {
            observed_at_unix_ms: now()?,
            state_dir: root,
            limits,
            reserved,
            available,
            unknown_resource_leases: unknown,
            admission_blocked: blocked,
            workers,
        })
    }

    async fn retained(state: &State, lease: uuid::Uuid) -> anyhow::Result<Record> {
        state
            .storage(move |store| {
                store
                    .records()?
                    .into_iter()
                    .find(|r| r.lease == lease)
                    .context("worker lease is no longer retained")
            })
            .await
    }

    pub(crate) async fn usage(
        state: Arc<State>,
        lease: uuid::Uuid,
    ) -> anyhow::Result<WorkerMeasurement> {
        let _permit = SAMPLERS
            .try_acquire()
            .context("worker measurement is busy")?;
        let record = retained(&state, lease).await?;
        let measurement = match &record.state {
            Phase::Created { container_id } => {
                let mut command = service::daemon_command(&record);
                command.args([
                    "stats",
                    "--no-stream",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                    container_id,
                ]);
                let output =
                    tokio::time::timeout(Duration::from_secs(4), service::collect(command))
                        .await
                        .context("worker measurement timed out")??;
                anyhow::ensure!(output.success, "Docker worker measurement unavailable");
                docker_sample(&record, container_id, &output.stdout)?
            }
            Phase::VmCreated { .. } => {
                let record = record.clone();
                tokio::task::spawn_blocking(move || vm_sample(&record)).await??
            }
            _ => anyhow::bail!(
                "worker creation or cleanup is uncertain; no measured usage available"
            ),
        };
        let current = retained(&state, lease).await?;
        anyhow::ensure!(
            serde_json::to_value(&current)? == serde_json::to_value(&record)?,
            "worker lease changed during measurement"
        );
        Ok(measurement)
    }

    fn docker_sample(record: &Record, id: &str, output: &str) -> anyhow::Result<WorkerMeasurement> {
        let value: serde_json::Value = serde_json::from_str(output)?;
        anyhow::ensure!(
            value["ID"] == id && value["Name"] == record.name,
            "Docker measurement identity mismatch"
        );
        let cpu = value["CPUPerc"]
            .as_str()
            .context("missing Docker CPU measurement")?;
        let number: f64 = cpu
            .strip_suffix('%')
            .context("invalid Docker CPU measurement")?
            .parse()?;
        let memory = value["MemUsage"]
            .as_str()
            .context("missing Docker memory measurement")?;
        anyhow::ensure!(
            cpu.len() <= 32
                && number.is_finite()
                && number >= 0.0
                && memory.len() <= 128
                && memory.contains(" / ")
                && memory.bytes().all(|b| b.is_ascii_graphic() || b == b' '),
            "invalid Docker usage fields"
        );
        // Docker emits zero/empty limits when a worker is stopped or statistics
        // are unavailable. Such a row is not a confirmed zero-usage sample.
        anyhow::ensure!(
            !memory.ends_with(" / 0B"),
            "Docker worker has no live memory measurement"
        );
        Ok(WorkerMeasurement {
            lease: record.lease,
            observed_at_unix_ms: now()?,
            source: "docker_cli".into(),
            cpu_percent: Some(cpu.into()),
            memory_usage: Some(memory.into()),
            cpu_time_micros: None,
            memory_bytes: None,
        })
    }

    fn bounded_read(path: &Path) -> anyhow::Result<String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        let mut text = String::new();
        file.take(8193).read_to_string(&mut text)?;
        anyhow::ensure!(text.len() <= 8192, "oversized host accounting field");
        Ok(text)
    }

    fn process_stat(pid: u32, start: u64) -> anyhow::Result<(u64, u64)> {
        let text = bounded_read(Path::new(&format!("/proc/{pid}/stat")))?;
        let fields: Vec<_> = text
            .rsplit_once(") ")
            .context("invalid VMM process stat")?
            .1
            .split_whitespace()
            .collect();
        anyhow::ensure!(
            !matches!(fields.first(), Some(&"Z" | &"X" | &"x")),
            "VMM process has exited; live usage is unavailable"
        );
        let number = |index: usize| -> anyhow::Result<u64> {
            Ok(fields
                .get(index)
                .context("incomplete VMM process stat")?
                .parse()?)
        };
        anyhow::ensure!(number(19)? == start, "VMM process identity changed");
        // SAFETY: sysconf constants have no memory preconditions.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        anyhow::ensure!(ticks > 0 && page > 0, "host accounting units unavailable");
        let cpu = number(11)?
            .checked_add(number(12)?)
            .and_then(|n| n.checked_mul(1_000_000))
            .context("VMM CPU counter overflow")?
            / (ticks as u64);
        let memory = number(21)?
            .checked_mul(page as u64)
            .context("VMM resident memory overflow")?;
        Ok((cpu, memory))
    }

    fn vm_sample(record: &Record) -> anyhow::Result<WorkerMeasurement> {
        let Phase::VmCreated {
            pid,
            start_ticks,
            boot_id,
        } = &record.state
        else {
            anyhow::bail!("VMM identity unavailable");
        };
        anyhow::ensure!(
            bounded_read(Path::new("/proc/sys/kernel/random/boot_id"))?.trim() == boot_id,
            "VMM host boot changed"
        );
        let (mut cpu, mut memory) = process_stat(*pid, *start_ticks)?;
        let source = if let Some(jail) = &record.jail {
            let directory = jail.cgroup(record.lease);
            crate::host::trusted_path(&directory)?;
            memory = bounded_read(&directory.join("memory.current"))?
                .trim()
                .parse()?;
            let stat = bounded_read(&directory.join("cpu.stat"))?;
            cpu = stat
                .lines()
                .find_map(|line| line.strip_prefix("usage_usec "))
                .context("VMM cgroup CPU counter unavailable")?
                .parse()?;
            "vmm_cgroup"
        } else {
            "vmm_process"
        };
        process_stat(*pid, *start_ticks)?;
        Ok(WorkerMeasurement {
            lease: record.lease,
            observed_at_unix_ms: now()?,
            source: source.into(),
            cpu_percent: None,
            memory_usage: None,
            cpu_time_micros: Some(cpu),
            memory_bytes: Some(memory),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn record() -> Record {
            let lease = uuid::Uuid::new_v4();
            Record {
                lease,
                name: format!("symbi-{lease}"),
                docker_binary: "/usr/bin/docker".into(),
                docker_environment: Default::default(),
                state: Phase::Uncertain,
                resources: Some(WorkerResources::vm(64, 1)),
                jail: None,
                staging: Vec::new(),
            }
        }

        #[test]
        fn retained_unknown_cleanup_and_legacy_charges_are_not_free_capacity() {
            let limits = AdmissionLimits {
                max_workers: 2,
                memory_bytes: 128 * 1024 * 1024,
                cpu_nanos: 2_000_000_000,
            };
            let mut worker = record();
            let before =
                snapshot("/tmp/fixture".into(), limits.clone(), vec![worker.clone()]).unwrap();
            assert_eq!(before.reserved.workers, 1);
            assert_eq!(before.available.memory_bytes, Some(64 * 1024 * 1024));
            assert_eq!(before.workers[0].phase, "uncertain");
            worker.resources = None;
            let legacy = snapshot("/tmp/fixture".into(), limits, vec![worker]).unwrap();
            assert!(legacy.admission_blocked);
            assert_eq!(legacy.unknown_resource_leases, 1);
            assert_eq!(legacy.reserved.memory_bytes, None);
            assert_eq!(legacy.available.cpu_nanos, None);
        }

        #[test]
        fn reduced_limits_and_arithmetic_overflow_never_reopen_capacity() {
            let limits = AdmissionLimits {
                max_workers: 1,
                memory_bytes: 1,
                cpu_nanos: 1,
            };
            let value = snapshot("/tmp/fixture".into(), limits, vec![record()]).unwrap();
            assert!(value.admission_blocked);
            assert_eq!(value.available.memory_bytes, Some(0));
            assert_eq!(value.available.workers, 0);
            let mut huge = record();
            huge.resources.as_mut().unwrap().memory_bytes = u64::MAX;
            assert!(snapshot(
                "/tmp/fixture".into(),
                AdmissionLimits::default(),
                vec![huge, record()]
            )
            .is_err());
        }

        #[test]
        fn docker_samples_require_exact_identity_and_live_finite_measurements() {
            let worker = record();
            let id = "a".repeat(64);
            let valid = serde_json::json!({"ID":id,"Name":worker.name,"CPUPerc":"12.34%","MemUsage":"8.2MiB / 64MiB"});
            assert_eq!(
                docker_sample(&worker, &id, &valid.to_string())
                    .unwrap()
                    .cpu_percent
                    .as_deref(),
                Some("12.34%")
            );
            for (key, value) in [
                ("ID", "other"),
                ("Name", "other"),
                ("CPUPerc", "NaN%"),
                ("MemUsage", "0B / 0B"),
            ] {
                let mut invalid = valid.clone();
                invalid[key] = value.into();
                assert!(
                    docker_sample(&worker, &id, &invalid.to_string()).is_err(),
                    "accepted {key}"
                );
            }
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn process_samples_reject_reused_identity_and_report_real_counters() {
            let pid = std::process::id();
            let stat = bounded_read(Path::new(&format!("/proc/{pid}/stat"))).unwrap();
            let start = stat
                .rsplit_once(") ")
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap()
                .parse()
                .unwrap();
            let mut worker = record();
            worker.state = Phase::VmCreated {
                pid,
                start_ticks: start,
                boot_id: bounded_read(Path::new("/proc/sys/kernel/random/boot_id"))
                    .unwrap()
                    .trim()
                    .into(),
            };
            let sample = vm_sample(&worker).unwrap();
            assert_eq!(sample.source, "vmm_process");
            assert!(sample.memory_bytes.unwrap() > 0);
            assert!(process_stat(pid, start + 1).is_err());
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn exited_process_is_not_a_live_zero_memory_sample() {
            let mut child = std::process::Command::new("/bin/sh")
                .args(["-c", "exit 0"])
                .spawn()
                .unwrap();
            let pid = child.id();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut exited = None;
            while std::time::Instant::now() < deadline {
                let stat = bounded_read(Path::new(&format!("/proc/{pid}/stat"))).unwrap();
                let fields: Vec<_> = stat
                    .rsplit_once(") ")
                    .unwrap()
                    .1
                    .split_whitespace()
                    .collect();
                if fields[0] == "Z" {
                    exited = Some(process_stat(pid, fields[19].parse().unwrap()));
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            child.wait().unwrap();
            assert!(exited
                .expect("fixture never became an exited process")
                .is_err());
        }
    }
}

#[cfg(unix)]
pub(crate) use local::{capacity, usage};

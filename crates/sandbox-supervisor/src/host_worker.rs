//! Capacity and lifetime ownership outlive the caller and its process groups.
use super::{reply, Active, State};
use crate::{
    protocol::{self, CreateHost, Reply, Request},
    store::{Phase, Record},
};
use anyhow::Context;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::BufReader,
    net::unix::{OwnedReadHalf, OwnedWriteHalf},
    time::sleep_until,
};

pub(super) async fn handle(
    mut reader: BufReader<OwnedReadHalf>,
    mut writer: OwnedWriteHalf,
    state: Arc<State>,
    spec: CreateHost,
) -> anyhow::Result<()> {
    let validation = spec.validate().and_then(|_| {
        state
            .delegated
            .as_ref()
            .context("Landlock requires an externally managed delegated worker service")
            .map(|_| ())
    });
    if let Err(error) = validation {
        reply(
            &mut writer,
            &Reply::Failed {
                message: error.to_string(),
            },
        )
        .await?;
        return Ok(());
    }
    let parent = state
        .delegated
        .as_ref()
        .expect("validated delegated service")
        .clone();
    let started = Instant::now();
    let deadline = started + Duration::from_millis(spec.lifetime_ms);
    let startup = started + Duration::from_millis(spec.startup_ms);
    if !state
        .active
        .lock()
        .map_err(|_| anyhow::anyhow!("lease registry poisoned"))?
        .insert(spec.lease)
    {
        anyhow::bail!("lease already active");
    }
    let _active = Active {
        state: state.clone(),
        lease: spec.lease,
    };
    let mut record = Record {
        lease: spec.lease,
        name: format!("symbi-{}", spec.lease),
        // Legacy field is retained for on-disk compatibility; never executed.
        docker_binary: std::env::current_exe()?,
        docker_environment: HashMap::new(),
        state: Phase::HostCreating {
            parent: parent.identity.clone(),
        },
        resources: Some(spec.resources),
        jail: None,
        staging: spec.staging.clone(),
        origin: spec.origin.clone(),
    };
    let registration = record.clone();
    if let Err(error) = state
        .storage(move |store| {
            let _pins = crate::staging::pin(&store.root, &registration.staging)?;
            store.insert(&registration)
        })
        .await
    {
        reply(
            &mut writer,
            &Reply::Failed {
                message: error.to_string(),
            },
        )
        .await?;
        return Ok(());
    }
    if reply(&mut writer, &Reply::Registered { lease: spec.lease })
        .await
        .is_err()
    {
        state.forget(spec.lease).await?;
        return Ok(());
    }
    let operation = async {
        anyhow::ensure!(Instant::now() < startup, "delegated worker startup expired");
        let group = parent.create(&spec)?;
        record.state = Phase::HostCreated {
            parent: parent.identity.clone(),
            cgroup: group.identity.clone(),
        };
        state.write(&record).await?;
        anyhow::ensure!(Instant::now() < startup, "delegated worker startup expired");
        reply(
            &mut writer,
            &Reply::HostCreated {
                cgroup: group.identity,
            },
        )
        .await?;
        tokio::select! {
            biased;
            _=protocol::read_frame::<Request>(&mut reader)=>{},
            _=sleep_until(deadline.into())=>anyhow::bail!("delegated worker lifetime expired"),
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if let Err(error) = operation {
        let _ = reply(
            &mut writer,
            &Reply::Failed {
                message: error.to_string(),
            },
        )
        .await;
    }
    if reconcile(&state, &record).await? {
        let _ = reply(&mut writer, &Reply::Closed {}).await;
    }
    Ok(())
}

pub(super) async fn reconcile(state: &State, record: &Record) -> anyhow::Result<bool> {
    let (parent, child) = match &record.state {
        Phase::HostCreating { parent } => (parent, None),
        Phase::HostCreated { parent, cgroup } => (parent, Some(cgroup)),
        _ => anyhow::bail!("not a delegated worker lease"),
    };
    if let Some(group) = parent.open()? {
        group.remove_child(record.lease, child).await?;
    }
    // Removed groups reject late joins through previously retained descriptors.
    // Any failed kill/read/rmdir keeps both the record and its resource charge.
    state.forget(record.lease).await?;
    Ok(true)
}

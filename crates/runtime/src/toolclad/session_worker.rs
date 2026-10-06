//! Bounded actors own interactive workers; registry locks never cover I/O.
use super::{infer_state, parse_session_tool_name, strip_ansi, SessionCall};
use crate::{
    sandbox::{
        command::{CommandBoundary, CommandTier},
        files::{FileAccessPlan, StagedFiles},
        streams::{Reader, StdioStreams, StreamGuard, Writer},
    },
    toolclad::{
        manifest::SessionDef,
        session_state::{SessionTranscript, TranscriptDirection},
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
};

const MAX_SESSIONS: usize = 16;
const MAX_CLOSED_RUNS: usize = 1024;
const MAX_TRANSCRIPT: usize = 4 * 1024 * 1024;
type Completion = Option<Result<(), String>>;

struct Interaction {
    command: String,
    command_name: String,
    finalize: bool,
    deadline: Instant,
    journal: Option<crate::reasoning::effect_journal::EffectJournal>,
    reply: oneshot::Sender<Result<serde_json::Value, String>>,
}

struct Slot {
    run: String,
    binding: String,
    file_grants: String,
    manifest: String,
    expires: Instant,
    inbox: mpsc::Sender<Interaction>,
    stop: watch::Sender<bool>,
    done: watch::Receiver<Completion>,
    transcript: Arc<Mutex<SessionTranscript>>,
}
impl Drop for Slot {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

#[derive(Default)]
struct Registry {
    slots: HashMap<String, Slot>,
    closed_runs: HashMap<String, Instant>,
    shutdown: bool,
}
impl Registry {
    fn prune(&mut self) {
        let now = Instant::now();
        // A run timeout must not discard an in-flight cleanup acknowledgement.
        self.slots
            .retain(|_, slot| slot.expires > now || !matches!(&*slot.done.borrow(), Some(Ok(()))));
        self.closed_runs.retain(|_, expires| *expires > now);
    }
}

#[derive(Default)]
pub(super) struct SessionManager {
    registry: Mutex<Registry>,
}
impl SessionManager {
    pub async fn execute(&self, call: SessionCall<'_>) -> Result<serde_json::Value, String> {
        if Instant::now() >= call.deadline || Instant::now() >= call.run_deadline {
            return Err("session authorization expired before dispatch".into());
        }
        call.boundary.validate()?;
        call.files.check_publication_authority()?;
        let worker_limit = match call.boundary.tier {
            CommandTier::Docker => call.boundary.docker.max_execution_time,
            CommandTier::GVisor => call.boundary.gvisor.docker.max_execution_time,
            CommandTier::Firecracker => call.boundary.firecracker.as_ref().ok_or("missing Firecracker configuration")?.max_execution_time,
            _ => return Err("interactive terminal transport requires Docker, gVisor or Firecracker; selected transport unavailable".into()),
        };
        let definition = call
            .manifest
            .session
            .as_ref()
            .ok_or("missing session definition")?;
        let (base, command_name) = parse_session_tool_name(call.tool)?;
        let command = definition
            .commands
            .get(&command_name)
            .ok_or("unknown session command")?;
        super::validate_terminal_input(call.command, &command.pattern)?;
        let settings = Settings::new(definition, worker_limit, call.run_deadline)?;
        let file_grants = crate::reasoning::prepared::digest_json(&call.files.descriptor())?;
        let key = crate::reasoning::prepared::digest_json(&serde_json::json!({
            "run": call.run, "binding": call.binding, "contract": call.contract, "manifest": base
        }))?;
        let inbox = {
            let mut registry = self.registry.lock().map_err(|e| e.to_string())?;
            registry.prune();
            if registry.shutdown || registry.closed_runs.contains_key(call.run) {
                return Err(
                    "execution run is closed; no further terminal effects are allowed".into(),
                );
            }
            if registry.slots.values().any(|slot| {
                slot.run == call.run
                    && slot.binding == call.binding
                    && slot.manifest == base
                    && slot.file_grants != file_grants
            }) {
                return Err("terminal file grants changed; start a new run".into());
            }
            if let Some(slot) = registry.slots.get(&key) {
                if slot.inbox.is_closed() {
                    return Err("terminal session is closed; start a new run".into());
                }
                slot.inbox.clone()
            } else {
                if registry.slots.len() >= MAX_SESSIONS {
                    return Err("interactive session capacity exhausted".into());
                }
                tokio::runtime::Handle::try_current()
                    .map_err(|_| "interactive sessions require a Tokio runtime")?;
                let (inbox, receiver) = mpsc::channel(1);
                let (stop, stopped) = watch::channel(false);
                let (finished, done) = watch::channel(None);
                let transcript = Arc::new(Mutex::new(SessionTranscript::default()));
                tokio::spawn(crate::sandbox::worker_origin::inherit(run_actor(
                    settings,
                    call.boundary.clone(),
                    call.files.with_journal(None),
                    receiver,
                    stopped,
                    finished,
                    transcript.clone(),
                )));
                registry.slots.insert(
                    key,
                    Slot {
                        run: call.run.to_owned(),
                        binding: call.binding.to_owned(),
                        file_grants,
                        manifest: base,
                        expires: call.run_deadline,
                        inbox: inbox.clone(),
                        stop,
                        done,
                        transcript,
                    },
                );
                inbox
            }
        };
        let (reply, response) = oneshot::channel();
        let request = Interaction {
            command: call.command.to_owned(),
            command_name,
            finalize: command.finalize,
            deadline: call.deadline,
            journal: call.files.effect_journal(),
            reply,
        };
        tokio::time::timeout_at(call.deadline.into(), inbox.send(request))
            .await
            .map_err(|_| "session command expired while waiting for admission")?
            .map_err(|_| "terminal session closed before admission")?;
        // Dropping response cancels the owning actor's active interaction.
        tokio::time::timeout_at(call.deadline.into(), response)
            .await
            .map_err(|_| "terminal interaction timed out; cleanup requested")?
            .map_err(|_| "terminal session closed before a response")?
    }

    pub fn cancel_run(&self, run: &str, deadline: Instant) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.prune();
            // Runs which never admitted a terminal need no tombstone. Admission
            // is synchronous under this same lock; cancelled futures cannot
            // resume after it to create a worker.
            if !registry.slots.values().any(|slot| slot.run == run)
                && !registry.closed_runs.contains_key(run)
            {
                return;
            }
            if registry.closed_runs.len() >= MAX_CLOSED_RUNS
                && !registry.closed_runs.contains_key(run)
            {
                registry.shutdown = true;
                for slot in registry.slots.values() {
                    let _ = slot.stop.send(true);
                }
            } else {
                registry.closed_runs.insert(run.to_owned(), deadline);
                for slot in registry.slots.values().filter(|s| s.run == run) {
                    let _ = slot.stop.send(true);
                }
            }
        }
    }

    pub async fn close_run(&self, run: &str, deadline: Instant) -> Result<(), String> {
        self.cancel_run(run, deadline);
        let (keys, completions) = {
            let registry = self.registry.lock().map_err(|e| e.to_string())?;
            registry
                .slots
                .iter()
                .filter(|(_, s)| s.run == run)
                .map(|(key, slot)| (key.clone(), slot.done.clone()))
                .unzip::<_, _, Vec<_>, Vec<_>>()
        };
        await_cleanup(completions).await?;
        let mut registry = self.registry.lock().map_err(|e| e.to_string())?;
        for key in keys {
            registry.slots.remove(&key);
        }
        Ok(())
    }

    pub fn cancel_all(&self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.shutdown = true;
            for slot in registry.slots.values() {
                let _ = slot.stop.send(true);
            }
        }
    }

    pub async fn close_all(&self) -> Result<(), String> {
        self.cancel_all();
        let completions = {
            let registry = self.registry.lock().map_err(|e| e.to_string())?;
            registry
                .slots
                .values()
                .map(|slot| slot.done.clone())
                .collect()
        };
        await_cleanup(completions).await?;
        self.registry
            .lock()
            .map_err(|e| e.to_string())?
            .slots
            .clear();
        Ok(())
    }

    pub fn transcript(&self, run: &str, manifest: &str) -> Option<SessionTranscript> {
        let registry = self.registry.lock().ok()?;
        let mut slots = registry
            .slots
            .values()
            .filter(|s| s.run == run && s.manifest == manifest);
        let slot = slots.next()?;
        if slots.next().is_some() {
            return None;
        }
        let transcript = slot.transcript.lock().ok()?.clone();
        Some(transcript)
    }
}
impl Drop for SessionManager {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

async fn await_cleanup(completions: Vec<watch::Receiver<Completion>>) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut errors = Vec::new();
        for mut done in completions {
            loop {
                let result = done.borrow().clone();
                if let Some(result) = result {
                    if let Err(error) = result {
                        errors.push(error);
                    }
                    break;
                }
                if done.changed().await.is_err() {
                    errors.push("terminal worker ended without cleanup acknowledgement".into());
                    break;
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    })
    .await
    .map_err(|_| "terminal cleanup acknowledgement timed out".to_string())?
}

struct Settings {
    argv: Vec<String>,
    prompt: regex::Regex,
    life: Instant,
    startup: Duration,
    idle: Duration,
    interaction: Duration,
    output: usize,
    max_interactions: u32,
}
impl Settings {
    fn new(
        def: &SessionDef,
        worker_limit: Duration,
        owner_deadline: Instant,
    ) -> Result<Self, String> {
        if def.startup_timeout_seconds == 0
            || def.idle_timeout_seconds == 0
            || def.session_timeout_seconds == 0
            || def.max_interactions == 0
            || def.max_interactions > 10000
        {
            return Err("terminal startup, idle, lifetime and interaction bounds must be positive and finite".into());
        }
        let argv = shlex::split(&def.startup_command)
            .filter(|argv| !argv.is_empty() && !argv[0].is_empty())
            .ok_or("invalid terminal startup argv")?;
        if def.startup_command.len() > 64 * 1024 || argv.iter().any(|a| a.contains('\0')) {
            return Err("terminal startup argv exceeds input limit".into());
        }
        let prompt = regex::Regex::new(&format!(r"\A(?:{})\z", def.ready_pattern))
            .map_err(|e| format!("invalid terminal ready pattern: {e}"))?;
        if prompt.is_match("") {
            return Err("terminal ready pattern must not match empty output".into());
        }
        let output = def
            .interaction
            .as_ref()
            .map_or(1_048_576, |i| i.output_max_bytes);
        let wait = def.interaction.as_ref().map_or(2000, |i| i.output_wait_ms);
        if output == 0 || output > MAX_TRANSCRIPT as u64 || wait == 0 {
            return Err(
                "terminal output must be bounded to at most 4 MiB with a positive wait".into(),
            );
        }
        let interaction = Duration::from_millis(
            wait.checked_mul(5)
                .ok_or("terminal wait exceeds supported range")?,
        );
        let life = Instant::now()
            .checked_add(Duration::from_secs(def.session_timeout_seconds).min(worker_limit))
            .ok_or("terminal lifetime exceeds supported range")?
            .min(owner_deadline);
        Ok(Self {
            argv,
            prompt,
            life,
            startup: Duration::from_secs(def.startup_timeout_seconds),
            idle: Duration::from_secs(def.idle_timeout_seconds),
            interaction,
            output: output as usize,
            max_interactions: def.max_interactions,
        })
    }
}

struct LivePty {
    guard: StreamGuard,
    stdin: Writer,
    stdout: Reader,
    stderr: Reader,
    stderr_closed: bool,
    used: usize,
}
impl LivePty {
    async fn start(
        boundary: &CommandBoundary,
        settings: &Settings,
        deadline: Instant,
        stop: &mut watch::Receiver<bool>,
        reply: &mut oneshot::Sender<Result<serde_json::Value, String>>,
    ) -> Result<Self, String> {
        let (cancel, cancelled) = watch::channel(false);
        let initialize = boundary.spawn_terminal(
            &settings.argv,
            deadline.saturating_duration_since(Instant::now()),
            settings.life.saturating_duration_since(Instant::now()),
            cancelled,
        );
        tokio::pin!(initialize);
        let reason = tokio::select! {
            biased;
            _ = stop.changed() => "terminal startup cancelled",
            _ = reply.closed() => "terminal caller cancelled during startup",
            _ = tokio::time::sleep_until(deadline.into()) => "terminal startup timed out",
            result = &mut initialize => return result.map(Self::new),
        };
        cancel.send_replace(true);
        // Never drop the initialization owner and report successful cleanup.
        // If attachment won the cancellation race, remove it before returning.
        match initialize.await {
            Ok(container) => Self::new(container).finish().await?,
            Err(error) => return Err(format!("{reason}: {error}")),
        }
        Err(reason.into())
    }

    fn new(worker: StdioStreams) -> Self {
        Self {
            stdin: worker.stdin,
            stdout: worker.stdout,
            stderr: worker.stderr,
            guard: worker.guard,
            stderr_closed: false,
            used: 0,
        }
    }
    fn count(&mut self, bytes: usize) -> Result<(), String> {
        self.used = self.used.saturating_add(bytes);
        if self.used > self.guard.output_limit {
            return Err("terminal lifetime output limit exceeded".into());
        }
        Ok(())
    }
    async fn prompt(
        &mut self,
        pattern: &regex::Regex,
        limit: usize,
    ) -> Result<(String, String), String> {
        let mut output = Vec::new();
        let mut stdout = [0; 8192];
        let mut stderr = [0; 8192];
        loop {
            tokio::select! {
                read = self.stdout.read(&mut stdout) => {
                    let size = read.map_err(|e| format!("terminal read failed: {e}"))?;
                    if size == 0 { return Err("terminal closed before the ready prompt".into()); }
                    self.count(size)?;
                    if output.len().saturating_add(size) > limit { return Err("terminal interaction output limit exceeded".into()); }
                    output.extend_from_slice(&stdout[..size]);
                    let text = strip_ansi(&String::from_utf8_lossy(&output));
                    if let Some(line) = text.lines().last() {
                        let line = line.trim();
                        if !line.is_empty() && pattern.is_match(line) {
                            let prompt = line.to_owned();
                            return Ok((text, prompt));
                        }
                    }
                },
                read = self.stderr.read(&mut stderr), if !self.stderr_closed => {
                    let size = read.map_err(|e| format!("terminal attachment stderr failed: {e}"))?;
                    self.stderr_closed = size == 0;
                    self.count(size)?;
                },
            }
        }
    }
    async fn finish(&mut self) -> Result<(), String> {
        self.guard.finish().await
    }
}

fn record(
    transcript: &Arc<Mutex<SessionTranscript>>,
    used: &mut usize,
    direction: TranscriptDirection,
    text: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let next = used.saturating_add(text.len()).saturating_add(256);
    if next > MAX_TRANSCRIPT {
        return Err("terminal transcript capacity exhausted".into());
    }
    transcript
        .lock()
        .map_err(|e| e.to_string())?
        .append(direction, text, name);
    *used = next;
    Ok(())
}
fn bounded_deadline(duration: Duration, cap: Instant) -> Instant {
    Instant::now().checked_add(duration).unwrap_or(cap).min(cap)
}

async fn run_actor(
    settings: Settings,
    boundary: CommandBoundary,
    files: FileAccessPlan,
    mut inbox: mpsc::Receiver<Interaction>,
    mut stop: watch::Receiver<bool>,
    done: watch::Sender<Completion>,
    transcript: Arc<Mutex<SessionTranscript>>,
) {
    enum Event {
        Request(Interaction),
        Stop,
        Output(Vec<u8>),
        Stderr(usize),
        End(String),
    }
    let mut live: Option<LivePty> = None;
    let mut staged: Option<StagedFiles> = None;
    let mut published = false;
    let mut idle = settings.life;
    let mut count = 0;
    let mut transcript_used = 0;
    let id = format!("session-{}", uuid::Uuid::new_v4());
    let mut cleanup_result = Ok(());
    let mut out = [0; 8192];
    let mut err = [0; 8192];
    loop {
        if *stop.borrow() || Instant::now() >= settings.life {
            break;
        }
        let next = if let Some(pty) = live.as_mut() {
            tokio::select! {
                biased;
                _ = stop.changed() => Event::Stop,
                _ = tokio::time::sleep_until(idle.min(settings.life).into()) => Event::Stop,
                request = inbox.recv() => request.map_or(Event::Stop, Event::Request),
                read = pty.stdout.read(&mut out) => match read {
                    Ok(0) => Event::End("terminal exited while idle".into()),
                    Ok(n) => Event::Output(out[..n].to_vec()),
                    Err(e) => Event::End(format!("terminal idle read failed: {e}")),
                },
                read = pty.stderr.read(&mut err), if !pty.stderr_closed => match read {
                    Ok(n) => Event::Stderr(n), Err(e) => Event::End(format!("terminal attachment failed: {e}")),
                },
            }
        } else {
            tokio::select! {
                biased;
                _ = stop.changed() => Event::Stop,
                _ = tokio::time::sleep_until(settings.life.into()) => Event::Stop,
                request = inbox.recv() => request.map_or(Event::Stop, Event::Request),
            }
        };
        let request = match next {
            Event::Stop => break,
            Event::End(error) => {
                cleanup_result = Err(error);
                break;
            }
            Event::Output(bytes) => {
                let recorded = live.as_mut().unwrap().count(bytes.len()).and_then(|()| {
                    record(
                        &transcript,
                        &mut transcript_used,
                        TranscriptDirection::System,
                        &strip_ansi(&String::from_utf8_lossy(&bytes)),
                        None,
                    )
                });
                if let Err(error) = recorded {
                    cleanup_result = Err(error);
                    break;
                }
                continue;
            }
            Event::Stderr(size) => {
                let pty = live.as_mut().unwrap();
                pty.stderr_closed = size == 0;
                if let Err(error) = pty.count(size) {
                    cleanup_result = Err(error);
                    break;
                }
                continue;
            }
            Event::Request(request) => request,
        };
        let Interaction {
            command,
            command_name,
            finalize,
            deadline,
            journal,
            mut reply,
        } = request;
        if reply.is_closed() || Instant::now() >= deadline {
            let _ = reply.send(Err("terminal command expired before execution".into()));
            continue;
        }
        let started = Instant::now();
        let start_deadline = bounded_deadline(settings.startup, deadline.min(settings.life));
        if live.is_none() {
            match files.stage(&boundary) {
                Ok(prepared) => staged = Some(prepared),
                Err(error) => {
                    cleanup_result = Err(error.clone());
                    let _ = reply.send(Err(error));
                    break;
                }
            }
            match LivePty::start(
                &staged.as_ref().unwrap().boundary,
                &settings,
                start_deadline,
                &mut stop,
                &mut reply,
            )
            .await
            {
                Ok(pty) => live = Some(pty),
                Err(error) => {
                    // Initialization errors may include a failed container
                    // removal. They cannot be reported as a clean run ending.
                    cleanup_result = Err(error.clone());
                    let _ = reply.send(Err(error));
                    break;
                }
            }
        }
        let operation = async {
            if count >= settings.max_interactions {
                return Err("terminal maximum interactions exhausted".into());
            }
            if count == 0 {
                let (output, _) = tokio::time::timeout_at(
                    start_deadline.into(),
                    live.as_mut()
                        .unwrap()
                        .prompt(&settings.prompt, settings.output),
                )
                .await
                .map_err(|_| "terminal startup timed out")??;
                record(
                    &transcript,
                    &mut transcript_used,
                    TranscriptDirection::System,
                    &output,
                    None,
                )?;
            }
            if Instant::now() >= deadline {
                return Err("terminal authorization expired before write".into());
            }
            record(
                &transcript,
                &mut transcript_used,
                TranscriptDirection::Command,
                &command,
                Some(&command_name),
            )?;
            let pty = live.as_mut().unwrap();
            pty.stdin
                .write_all(format!("{command}\n").as_bytes())
                .await
                .map_err(|e| format!("terminal write failed: {e}"))?;
            pty.stdin
                .flush()
                .await
                .map_err(|e| format!("terminal flush failed: {e}"))?;
            let limit = bounded_deadline(settings.interaction, deadline.min(settings.life));
            let (output, prompt) = tokio::time::timeout_at(
                limit.into(),
                pty.prompt(&settings.prompt, settings.output),
            )
            .await
            .map_err(|_| "terminal response timed out")??;
            record(
                &transcript,
                &mut transcript_used,
                TranscriptDirection::Response,
                &output,
                Some(&command_name),
            )?;
            count += 1;
            Ok(serde_json::json!({
                "status":"success", "execution_status":"prompt_observed", "session_id":id,
                "session_closed": false, "created_files": [],
                "file_publication": if files.has_output() { "pending" } else { "none" },
                "duration_ms":started.elapsed().as_millis(), "timestamp":chrono::Utc::now().to_rfc3339(),
                "exit_code":null, "stderr":"", "results":{"output":output,"prompt":prompt,
                    "session_state":infer_state(&prompt),"interaction_count":count}
            }))
        };
        let mut result = tokio::select! {
            biased;
            _ = stop.changed() => Err("terminal run cancelled".into()),
            _ = reply.closed() => Err("terminal caller cancelled".into()),
            _ = tokio::time::sleep_until(deadline.min(settings.life).into()) => Err("terminal interaction timed out".into()),
            result = operation => result,
        };
        if result.is_err() || finalize || count >= settings.max_interactions {
            if let Some(mut pty) = live.take() {
                if let Err(error) = pty.finish().await {
                    result = Err(error);
                }
                cleanup_result = pty.guard.finish_cleanup().await;
                if let Err(error) = &cleanup_result {
                    result = Err(error.clone());
                }
            }
            if let Ok(value) = &mut result {
                value["session_closed"] = serde_json::json!(true);
            }
            if result.is_ok() && finalize {
                result = async {
                    let mut value = result?;
                    if reply.is_closed()
                        || *stop.borrow()
                        || Instant::now() >= deadline.min(settings.life)
                    {
                        return Err(
                            "terminal finalization cancelled or expired before file publication"
                                .into(),
                        );
                    }
                    value["created_files"] = staged
                        .as_ref()
                        .ok_or("missing terminal file staging")?
                        .publish_using(journal.as_ref())
                        .await?;
                    value["file_publication"] = serde_json::json!(if files.has_output() {
                        "published"
                    } else {
                        "none"
                    });
                    value["execution_status"] = serde_json::json!("session_finalized");
                    published = true;
                    Ok(value)
                }
                .await;
            } else if result.is_ok() && files.has_output() {
                result = Err("terminal interaction limit reached with unpublished output; finalize before the limit".into());
            }
            let _ = reply.send(result);
            break;
        }
        idle = bounded_deadline(settings.idle, settings.life);
        if reply.send(result).is_err() {
            break;
        }
    }
    if let Some(mut pty) = live {
        if let Err(error) = pty.guard.finish_cleanup().await {
            cleanup_result = Err(error);
        }
    }
    if files.has_output() && !published && cleanup_result.is_ok() {
        cleanup_result = Err(
            "terminal closed with unpublished file output; explicit finalization required".into(),
        );
    }
    done.send_replace(Some(cleanup_result));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retrying_cleanup_cannot_discard_a_failed_acknowledgement() {
        let manager = SessionManager::default();
        let (inbox, _) = mpsc::channel(1);
        let (stop, _) = watch::channel(false);
        let (_, done) = watch::channel(Some(Err("synthetic removal failure".into())));
        let deadline = Instant::now() + Duration::from_secs(1);
        manager.registry.lock().unwrap().slots.insert(
            "fixture".into(),
            Slot {
                run: "run".into(),
                binding: "binding".into(),
                file_grants: "files".into(),
                manifest: "fixture".into(),
                expires: deadline,
                inbox,
                stop,
                done,
                transcript: Arc::new(Mutex::new(SessionTranscript::default())),
            },
        );
        for _ in 0..2 {
            assert!(manager
                .close_run("run", deadline)
                .await
                .unwrap_err()
                .contains("removal failure"));
            assert!(manager
                .close_all()
                .await
                .unwrap_err()
                .contains("removal failure"));
        }
        assert_eq!(manager.registry.lock().unwrap().slots.len(), 1);
    }
}

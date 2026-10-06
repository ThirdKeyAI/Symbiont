//! Real Docker lifecycle tests. Run explicitly with --ignored; a missing daemon
//! or cached image fails the test. All payloads and observers are synthetic.
#![cfg(target_os = "linux")]

use std::{
    collections::HashMap, os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration,
};
use symbi_sandbox_supervisor::protocol::{self, Create, Reply, Request, IMPLEMENTATION, VERSION};
use tokio::{
    io::BufReader,
    net::UnixStream,
    process::{Child, Command},
    time::{sleep, timeout},
};

struct Fixture {
    root: tempfile::TempDir,
    helper: Option<Child>,
    label: String,
    image: String,
}

impl Fixture {
    async fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(root.path().join("output")).unwrap();
        // Only the output directory is mounted. Control files remain private.
        std::fs::set_permissions(
            root.path().join("output"),
            std::fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        let output = docker(&[
            "image",
            "inspect",
            "--format",
            "{{.Id}}",
            "python:3.12-slim",
        ])
        .await;
        let image = output.trim().to_owned();
        assert!(image.starts_with("sha256:"), "cached image is required");
        let label = format!("symbi.supervisor-e2e={}", uuid::Uuid::new_v4());
        let wrapper = format!(
            r#"#!/usr/bin/python3
import os, pathlib, subprocess, sys, time
root = pathlib.Path({root:?})
mode = {mode:?}
if sys.argv[1] == 'create':
    with (root / 'create-count').open('ab') as count: count.write(b'1')
    # Emulate a submitted request whose Docker client has already read its
    # environment, so later service cleanup cannot retract the queued request.
    position = sys.argv.index('--env-file') + 1
    environment = pathlib.Path(sys.argv[position]).read_bytes()
    retained = root / 'submitted-environment'
    retained.write_bytes(environment)
    retained.chmod(0o600)
    sys.argv[position] = str(retained)
    (root / 'create-entered').touch()
    if mode == 'uncertain':
        if os.fork() != 0: sys.exit(1)
        os.setsid()
        for fd in (0, 1, 2):
            os.dup2(os.open('/dev/null', os.O_RDWR), fd)
    if mode in ('delayed', 'uncertain'): time.sleep(3)
    result = subprocess.run(['/usr/bin/docker', *sys.argv[1:]], capture_output=True)
    if result.returncode == 0: (root / 'created-id').write_bytes(result.stdout)
    sys.stdout.buffer.write(result.stdout)
    sys.stderr.buffer.write(result.stderr)
    sys.exit(result.returncode)
if sys.argv[1] == 'rm' and (root / 'refuse-removal').exists():
    sys.stderr.write('synthetic temporary removal failure')
    sys.exit(1)
os.execv('/usr/bin/docker', ['docker', *sys.argv[1:]])
"#,
            root = root.path().to_str().unwrap()
        );
        std::fs::write(root.path().join("docker-client"), wrapper).unwrap();
        std::fs::set_permissions(
            root.path().join("docker-client"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let mut fixture = Self {
            root,
            helper: None,
            label,
            image,
        };
        fixture.start().await;
        fixture
    }

    fn state(&self) -> std::path::PathBuf {
        self.root.path().join("leases")
    }

    async fn start(&mut self) {
        self.helper = Some(
            Command::new(env!("CARGO_BIN_EXE_symbi-sandbox-supervisor"))
                .args(["--state-dir", self.state().to_str().unwrap()])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
        );
        timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(mut stream) =
                    UnixStream::connect(self.state().join(protocol::SOCKET)).await
                {
                    protocol::write_frame(
                        &mut stream,
                        &Request::Ping {
                            version: VERSION,
                            implementation: IMPLEMENTATION.into(),
                        },
                    )
                    .await
                    .unwrap();
                    if matches!(
                        protocol::read_frame::<Reply>(&mut BufReader::new(stream)).await,
                        Ok(Some(Reply::Ready { .. }))
                    ) {
                        break;
                    }
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("supervisor failed to become ready");
    }

    async fn kill(&mut self) {
        if let Some(mut helper) = self.helper.take() {
            helper.kill().await.unwrap();
            helper.wait().await.unwrap();
        }
    }

    async fn register(&self, startup: u64, lifetime: u64) -> (BufReader<UnixStream>, uuid::Uuid) {
        let (mut stream, lease) = self.submit(startup, lifetime, |_| {}).await;
        assert!(
            matches!(next(&mut stream).await, Some(Reply::Registered { lease: actual }) if actual == lease)
        );
        (stream, lease)
    }

    async fn submit(
        &self,
        startup: u64,
        lifetime: u64,
        amend: impl FnOnce(&mut Create),
    ) -> (BufReader<UnixStream>, uuid::Uuid) {
        let lease = uuid::Uuid::new_v4();
        let name = format!("symbi-{lease}");
        let args = vec![
            "create",
            "--interactive",
            "--name",
            &name,
            "--pull",
            "never",
            "--init",
            "--restart",
            "no",
            "--read-only",
            "--network",
            "none",
            "--user",
            "65534:65534",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--memory",
            "64m",
            "--memory-swap",
            "64m",
            "--cpus",
            "0.5",
            "--pids-limit",
            "16",
            "--log-driver",
            "none",
        ];
        let mut arguments = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        arguments.extend(["--mount".into(), format!("type=bind,src={},dst=/workspace", self.root.path().join("output").display()),
            "--label".into(), self.label.clone(), "--entrypoint".into(), "python".into(), self.image.clone(), "-c".into(),
            "import os,pathlib,time; assert os.getuid()==65534; assert os.environ['EXPLICIT']=='synthetic'; pathlib.Path('/workspace/started').touch(); pid=os.fork(); os.setsid() if pid==0 else None; exec(\"while True:\\n pathlib.Path('/workspace/ticks').write_text(str(time.monotonic()))\\n time.sleep(0.05)\")".into()]);
        let mut request = Create {
            origin: None,
            staging: Vec::new(),
            version: VERSION,
            implementation: IMPLEMENTATION.into(),
            lease,
            name,
            docker_binary: self.root.path().join("docker-client"),
            docker_environment: HashMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            arguments,
            environment: HashMap::from([("EXPLICIT".into(), "synthetic".into())]),
            startup_ms: startup,
            lifetime_ms: lifetime,
            resources: symbi_sandbox_supervisor::admission::WorkerResources::docker("64m", 0.5)
                .unwrap(),
        };
        amend(&mut request);
        let request = Request::Create(Box::new(request));
        let mut stream = UnixStream::connect(self.state().join(protocol::SOCKET))
            .await
            .unwrap();
        protocol::write_frame(&mut stream, &request).await.unwrap();
        (BufReader::new(stream), lease)
    }

    async fn absent(&self, lease: uuid::Uuid) {
        timeout(Duration::from_secs(12), async {
            loop {
                let containers =
                    docker(&["ps", "-aq", "--filter", &format!("label={}", self.label)]).await;
                if containers.trim().is_empty()
                    && !self.state().join(format!("{lease}.json")).exists()
                {
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("owned container or lease survived cleanup");
    }

    async fn start_worker(&self, stream: &mut BufReader<UnixStream>) -> Child {
        let Some(Reply::Created { id }) = next(stream).await else {
            panic!("container was not ready")
        };
        assert!(!self.root.path().join("output/started").exists());
        let child = Command::new("docker")
            .args(["start", "--attach", &id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        wait_file(&self.root.path().join("output/ticks")).await;
        child
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(helper) = &mut self.helper {
            let _ = helper.start_kill();
        }
        let output = std::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={}", self.label)])
            .output()
            .unwrap();
        for id in String::from_utf8_lossy(&output.stdout).split_whitespace() {
            let _ = std::process::Command::new("docker")
                .args(["rm", "--force", "--volumes", id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

async fn next(stream: &mut BufReader<UnixStream>) -> Option<Reply> {
    timeout(Duration::from_secs(15), protocol::read_frame(stream))
        .await
        .unwrap()
        .unwrap()
}
async fn wait_file(path: &Path) {
    timeout(Duration::from_secs(8), async {
        while !path.exists() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expected external effect missing");
}
async fn docker(args: &[&str]) -> String {
    let output = timeout(
        Duration::from_secs(15),
        Command::new("docker").args(args).output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "Docker operation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn shared_admission_retains_capacity_through_cleanup_failure_and_restart() {
    let mut fixture = Fixture::new("normal").await;
    fixture.kill().await;
    let limits = symbi_sandbox_supervisor::admission::AdmissionLimits {
        max_workers: 1,
        memory_bytes: 64 * 1024 * 1024,
        cpu_nanos: 1_000_000_000,
    };
    std::fs::write(
        fixture.state().join("admission.conf"),
        serde_json::to_vec(&limits).unwrap(),
    )
    .unwrap();
    fixture.start().await;
    let (mut first, lease) = fixture.register(5000, 30000).await;
    let mut attachment = fixture.start_worker(&mut first).await;
    std::fs::write(
        fixture.root.path().join("refuse-removal"),
        b"temporary outage",
    )
    .unwrap();
    drop(first);

    for restart in [false, true] {
        if restart {
            fixture.kill().await;
            fixture.start().await;
        }
        let (mut rejected, _) = fixture.submit(5000, 10000, |_| {}).await;
        assert!(
            matches!(next(&mut rejected).await, Some(Reply::Failed { message }) if message.contains("shared worker capacity exhausted"))
        );
        assert_eq!(
            std::fs::read(fixture.root.path().join("create-count")).unwrap(),
            b"1"
        );
        assert!(fixture.state().join(format!("{lease}.json")).exists());
    }

    // Both backend kinds enter the same admission owner. Refusal occurs before
    // artifact access or any VMM launch, even though these files do not exist.
    let vm = protocol::CreateVm {
        origin: None,
        version: VERSION,
        implementation: IMPLEMENTATION.into(),
        lease: uuid::Uuid::new_v4(),
        binary: "/missing/firecracker".into(),
        kernel: "/missing/kernel".into(),
        rootfs: "/missing/rootfs".into(),
        boot_args: String::new(),
        vcpus: 1,
        memory_mib: 64,
        lifetime_ms: 10000,
        startup_ms: 5000,
    };
    let mut socket = UnixStream::connect(fixture.state().join(protocol::SOCKET))
        .await
        .unwrap();
    protocol::write_frame(&mut socket, &Request::CreateVm(Box::new(vm)))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut BufReader::new(socket)).await, Some(Reply::Failed { message }) if message.contains("shared worker capacity exhausted"))
    );

    std::fs::remove_file(fixture.root.path().join("refuse-removal")).unwrap();
    fixture.absent(lease).await;
    attachment.wait().await.unwrap();
    std::fs::remove_file(fixture.root.path().join("output/started")).unwrap();
    std::fs::remove_file(fixture.root.path().join("output/ticks")).unwrap();
    let (mut next_worker, next_lease) = fixture.register(5000, 10000).await;
    let mut attachment = fixture.start_worker(&mut next_worker).await;
    assert_eq!(
        std::fs::read(fixture.root.path().join("create-count")).unwrap(),
        b"11"
    );
    drop(next_worker);
    fixture.absent(next_lease).await;
    attachment.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn shared_admission_checks_actual_docker_limits_before_worker_start() {
    let fixture = Fixture::new("normal").await;
    for memory in [true, false] {
        let (mut stream, lease) = fixture
            .submit(5000, 10000, |request| {
                if memory {
                    request.resources.memory_bytes /= 2;
                } else {
                    request.resources.cpu_nanos /= 2;
                }
            })
            .await;
        assert!(matches!(
            next(&mut stream).await,
            Some(Reply::Registered { .. })
        ));
        assert!(
            matches!(next(&mut stream).await, Some(Reply::Failed { message }) if message.contains("actual Docker limits exceed"))
        );
        assert!(matches!(next(&mut stream).await, Some(Reply::Closed {})));
        fixture.absent(lease).await;
        assert!(!fixture.root.path().join("output/started").exists());
    }
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn useful_work_and_disconnected_owner_reap_detached_descendants() {
    let fixture = Fixture::new("normal").await;
    let (mut stream, lease) = fixture.register(5000, 20000).await;
    let mut attachment = fixture.start_worker(&mut stream).await;
    drop(stream);
    fixture.absent(lease).await;
    attachment.wait().await.unwrap();
    let before = std::fs::read(fixture.root.path().join("output/ticks")).unwrap();
    sleep(Duration::from_millis(250)).await;
    assert_eq!(
        before,
        std::fs::read(fixture.root.path().join("output/ticks")).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn service_deadline_expires_even_while_owner_connection_stays_open() {
    let fixture = Fixture::new("normal").await;
    let (mut stream, lease) = fixture.register(5000, 6000).await;
    let mut attachment = fixture.start_worker(&mut stream).await;
    assert!(matches!(next(&mut stream).await, Some(Reply::Closed {})));
    fixture.absent(lease).await;
    attachment.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn delayed_create_after_deadline_never_starts_and_is_removed() {
    let fixture = Fixture::new("delayed").await;
    let (mut stream, lease) = fixture.register(200, 500).await;
    assert!(matches!(
        next(&mut stream).await,
        Some(Reply::Failed { .. })
    ));
    assert!(matches!(next(&mut stream).await, Some(Reply::Closed {})));
    wait_file(&fixture.root.path().join("created-id")).await;
    fixture.absent(lease).await;
    assert!(!fixture.root.path().join("output/started").exists());
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn failed_client_retains_unknown_record_until_late_container_arrives() {
    let fixture = Fixture::new("uncertain").await;
    let (mut stream, lease) = fixture.register(5000, 10000).await;
    assert!(matches!(
        next(&mut stream).await,
        Some(Reply::Failed { .. })
    ));
    assert!(next(&mut stream).await.is_none());
    let record = fixture.state().join(format!("{lease}.json"));
    assert!(std::fs::read_to_string(&record)
        .unwrap()
        .contains("uncertain"));
    assert!(!fixture.root.path().join("created-id").exists());
    wait_file(&fixture.root.path().join("created-id")).await;
    fixture.absent(lease).await;
    assert!(!fixture.root.path().join("output/started").exists());
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn restart_recovers_running_container_after_supervisor_sigkill() {
    let mut fixture = Fixture::new("normal").await;
    let (mut stream, lease) = fixture.register(5000, 20000).await;
    let mut attachment = fixture.start_worker(&mut stream).await;
    fixture.kill().await;
    assert!(next(&mut stream).await.is_none());
    assert!(fixture.state().join(format!("{lease}.json")).exists());
    fixture.start().await;
    fixture.absent(lease).await;
    attachment.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn restart_recovers_creation_in_flight_after_supervisor_sigkill() {
    let mut fixture = Fixture::new("delayed").await;
    let (stream, lease) = fixture.register(5000, 10000).await;
    wait_file(&fixture.root.path().join("create-entered")).await;
    fixture.kill().await;
    drop(stream);
    fixture.start().await;
    wait_file(&fixture.root.path().join("created-id")).await;
    fixture.absent(lease).await;
    assert!(!fixture.root.path().join("output/started").exists());
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn removal_failure_is_not_acknowledged_and_recovery_retries() {
    let fixture = Fixture::new("normal").await;
    let (mut stream, lease) = fixture.register(5000, 20000).await;
    let mut attachment = fixture.start_worker(&mut stream).await;
    let failure = fixture.root.path().join("refuse-removal");
    std::fs::write(&failure, b"synthetic fault").unwrap();
    protocol::write_frame(stream.get_mut(), &Request::Close {})
        .await
        .unwrap();
    assert!(
        next(&mut stream).await.is_none(),
        "cleanup failure must not return Closed"
    );
    assert!(fixture.state().join(format!("{lease}.json")).exists());
    std::fs::remove_file(failure).unwrap();
    fixture.absent(lease).await;
    attachment.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Docker and cached python:3.12-slim"]
async fn recovery_refuses_container_with_a_different_ownership_label() {
    let mut fixture = Fixture::new("normal").await;
    let (mut stream, lease) = fixture.register(5000, 20000).await;
    let Some(Reply::Created { id }) = next(&mut stream).await else {
        panic!("container not ready")
    };
    fixture.kill().await;
    let original = fixture.state().join(format!("{lease}.json"));
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&original).unwrap()).unwrap();
    let other = uuid::Uuid::new_v4();
    record["lease"] = other.to_string().into();
    record["name"] = format!("symbi-{other}").into();
    let changed = fixture.state().join(format!("{other}.json"));
    std::fs::write(&changed, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::set_permissions(&changed, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_file(original).unwrap();
    fixture.start().await;
    sleep(Duration::from_secs(2)).await;
    assert_eq!(
        docker(&["inspect", "--format", "{{.Id}}", &id])
            .await
            .trim(),
        id
    );
    assert!(
        changed.exists(),
        "ownership mismatch must remain unresolved"
    );
}

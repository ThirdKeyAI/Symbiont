//! Enforcement tests spawn children. A landlock domain cannot be lifted once
//! applied, so restricting the test process itself would poison every test
//! that ran afterwards.
#![cfg(target_os = "linux")]

use std::process::Command;
use symbi_runtime::sandbox::command::BoundaryRoots;
use symbi_runtime::sandbox::landlock::{detect_abi, prepare, LandlockProfile};

fn allowed_only(dir: &std::path::Path) -> BoundaryRoots {
    BoundaryRoots {
        source_roots: vec![format!("{}:/workspace:ro", dir.display())],
        output_roots: vec![],
    }
}

#[test]
fn a_restricted_child_reads_its_grant_and_is_refused_everything_else() {
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("allowed"), b"ok").expect("write");
    let denied = tempfile::tempdir().expect("tempdir");
    std::fs::write(denied.path().join("secret"), b"no").expect("write");

    let domain =
        prepare(&LandlockProfile::default(), &allowed_only(dir.path())).expect("prepare domain");

    let mut allowed_read = Command::new("/bin/cat");
    allowed_read.arg(dir.path().join("allowed"));
    domain.apply_to_std(&mut allowed_read);
    assert!(
        allowed_read.status().expect("spawn").success(),
        "the granted path must stay readable"
    );

    let mut denied_read = Command::new("/bin/cat");
    denied_read.arg(denied.path().join("secret"));
    domain.apply_to_std(&mut denied_read);
    assert!(
        !denied_read.status().expect("spawn").success(),
        "a path outside every grant must be refused"
    );
}

#[test]
fn prepared_read_authority_stays_with_the_original_directory() {
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    let root = tempfile::tempdir().expect("tempdir");
    let granted = root.path().join("granted");
    let retained = root.path().join("retained");
    let denied = root.path().join("denied");
    std::fs::create_dir(&granted).unwrap();
    std::fs::create_dir(&denied).unwrap();
    std::fs::write(granted.join("data"), b"allowed").unwrap();
    std::fs::write(denied.join("data"), b"private").unwrap();
    let domain = prepare(&LandlockProfile::default(), &allowed_only(&granted)).unwrap();

    std::fs::rename(&granted, &retained).unwrap();
    std::os::unix::fs::symlink(&denied, &granted).unwrap();
    let mut command = Command::new("/bin/cat");
    command.arg(granted.join("data"));
    domain.apply_to_std(&mut command);
    let output = command.output().unwrap();
    assert!(
        !output.status.success(),
        "a substituted root gained read access"
    );
    assert!(output.stdout.is_empty());

    let mut command = Command::new("/bin/cat");
    command.arg(retained.join("data"));
    domain.apply_to_std(&mut command);
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"allowed");
}

#[tokio::test]
async fn prepared_write_authority_stays_with_the_original_directory() {
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    let root = tempfile::tempdir().expect("tempdir");
    let granted = root.path().join("granted");
    let retained = root.path().join("retained");
    std::fs::create_dir(&granted).unwrap();
    let roots = BoundaryRoots {
        source_roots: vec![],
        output_roots: vec![granted.display().to_string()],
    };
    let domain = prepare(&LandlockProfile::default(), &roots).unwrap();
    std::fs::rename(&granted, &retained).unwrap();
    std::fs::create_dir(&granted).unwrap();
    let sentinel = granted.join("data");
    std::fs::write(&sentinel, b"private").unwrap();

    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args(["-c", "printf changed > \"$1\"", "write"])
        .arg(&sentinel);
    domain.apply_to(&mut command);
    let output = command.output().await.unwrap();
    assert!(
        !output.status.success(),
        "a substituted root gained write access"
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"private");

    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args(["-c", "printf allowed > \"$1\"", "write"])
        .arg(retained.join("data"));
    domain.apply_to(&mut command);
    assert!(command.output().await.unwrap().status.success());
    assert_eq!(std::fs::read(retained.join("data")).unwrap(), b"allowed");
}

#[test]
fn a_missing_declared_root_is_an_error_before_spawn() {
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    assert!(prepare(&LandlockProfile::default(), &allowed_only(&missing)).is_err());
    let roots = BoundaryRoots {
        source_roots: vec![],
        output_roots: vec![missing.display().to_string()],
    };
    assert!(prepare(&LandlockProfile::default(), &roots).is_err());
}

#[test]
fn kernel_controls_and_supervisor_state_cannot_be_granted() {
    if detect_abi() < 6 {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let mut profile = LandlockProfile::default();
    profile.supervisor.state_dir = root.path().join("private-state");
    std::fs::create_dir(&profile.supervisor.state_dir).unwrap();
    let alias = root.path().join("sys-alias");
    std::os::unix::fs::symlink("/sys", &alias).unwrap();
    for path in [
        std::path::Path::new("/sys"),
        std::path::Path::new("/proc"),
        alias.as_path(),
        root.path(),
        profile.supervisor.state_dir.as_path(),
    ] {
        assert!(
            prepare(&profile, &allowed_only(path)).is_err(),
            "accepted {path:?}"
        );
        assert!(prepare(
            &profile,
            &BoundaryRoots {
                source_roots: vec![],
                output_roots: vec![path.display().to_string()]
            }
        )
        .is_err());
    }
}

fn run_python(profile: &LandlockProfile, script: &str, args: &[String]) -> std::process::Output {
    let domain = prepare(profile, &BoundaryRoots::default()).unwrap();
    let mut command = Command::new("/usr/bin/python3");
    command.env_clear().args(["-c", script]).args(args);
    domain.apply_to_std(&mut command);
    command.output().unwrap()
}

#[test]
fn sockets_cannot_reach_the_host_but_private_ipc_and_explicit_ip_access_work() {
    use std::io::Read;
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixDatagram, UnixListener};
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let unix_path = root.path().join("control.sock");
    let unix = UnixListener::bind(&unix_path).unwrap();
    unix.set_nonblocking(true).unwrap();
    let datagram_path = root.path().join("control.dgram");
    let unix_datagram = UnixDatagram::bind(&datagram_path).unwrap();
    unix_datagram.set_nonblocking(true).unwrap();
    let abstract_name = format!("symbi-landlock-{}", uuid::Uuid::new_v4());
    let abstract_unix =
        UnixListener::bind_addr(&SocketAddr::from_abstract_name(abstract_name.as_bytes()).unwrap())
            .unwrap();
    abstract_unix.set_nonblocking(true).unwrap();
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    tcp.set_nonblocking(true).unwrap();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_nonblocking(true).unwrap();
    let script = r#"
import ctypes, errno, json, socket, sys
restricted, tcp_port, udp_port, path, datagram_path, abstract, uring = json.loads(sys.argv[1])
for family, kind, address in [
    (socket.AF_INET, socket.SOCK_STREAM, ('127.0.0.1', tcp_port)),
    (socket.AF_INET, socket.SOCK_DGRAM, ('127.0.0.1', udp_port)),
    (socket.AF_UNIX, socket.SOCK_STREAM, path),
    (socket.AF_UNIX, socket.SOCK_STREAM, '\0' + abstract),
]:
    denied = restricted or family == socket.AF_UNIX
    try:
        with socket.socket(family, kind) as connection:
            connection.settimeout(2)
            connection.connect(address)
            connection.sendall(b'probe')
    except OSError as error:
        assert denied and error.errno == errno.EACCES, (address, error)
    else:
        assert not denied, address
left, right = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM | socket.SOCK_CLOEXEC | socket.SOCK_NONBLOCK)
with left, right:
    left.sendall(b'private')
    assert right.recv(7) == b'private'
    try: left.connect(path)
    except OSError: pass
    else: raise AssertionError('stream pair reconnected to the host')
# Datagram socket pairs can send to a different pathname peer. They must not
# provide an alternate route around the socket creation restriction.
try:
    left, right = socket.socketpair(socket.AF_UNIX, socket.SOCK_DGRAM)
    with left, right: left.sendto(b'probe', datagram_path)
except OSError as error: assert error.errno == errno.EACCES
else: raise AssertionError('datagram pair reached a host socket')
libc = ctypes.CDLL(None, use_errno=True)
for syscall in uring:
    # Invalid arguments are safe even if the filter regresses; it must refuse
    # before argument validation, not merely fail due to the invalid FD.
    assert libc.syscall(ctypes.c_long(syscall), -1, 0, 0, 0, 0, 0) == -1
    assert ctypes.get_errno() == errno.EACCES
print('useful result: 10')
"#;
    for restricted in [true, false] {
        let profile = LandlockProfile {
            require_network: restricted,
            ..Default::default()
        };
        let args = serde_json::json!([
            restricted,
            tcp.local_addr().unwrap().port(),
            udp.local_addr().unwrap().port(),
            unix_path,
            datagram_path,
            abstract_name,
            [
                libc::SYS_io_uring_setup,
                libc::SYS_io_uring_enter,
                libc::SYS_io_uring_register
            ]
        ]);
        let output = run_python(&profile, script, &[args.to_string()]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"useful result: 10\n");
        assert_eq!(
            unix.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            abstract_unix.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let mut received = [0; 5];
        assert_eq!(
            unix_datagram.recv(&mut received).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        if restricted {
            assert_eq!(
                tcp.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            assert_eq!(
                udp.recv(&mut received).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        } else {
            let (mut connection, _) = tcp.accept().unwrap();
            connection
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            connection.read_exact(&mut received).unwrap();
            assert_eq!(&received, b"probe");
            assert_eq!(udp.recv(&mut received).unwrap(), 5);
            assert_eq!(&received, b"probe");
        }
    }
}

#[test]
fn signals_stay_inside_the_worker_domain() {
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    // Signal zero probes authority without risking the test runner if the
    // boundary regresses. The shipping E2E also observes a handled SIGUSR1.
    let output = run_python(
        &LandlockProfile::default(),
        r#"
import errno, os, signal, sys
try: os.kill(int(sys.argv[1]), 0)
except OSError as error: assert error.errno == errno.EPERM
else: raise AssertionError('outside process accepted signal')
reader, writer = os.pipe()
pid = os.fork()
if pid == 0:
    signal.alarm(3)
    os.close(reader)
    signal.signal(signal.SIGUSR1, lambda *_: os._exit(0))
    os.write(writer, b'ready')
    signal.pause()
    os._exit(1)
os.close(writer)
assert os.read(reader, 5) == b'ready'
os.kill(pid, signal.SIGUSR1)
assert os.waitpid(pid, 0)[1] == 0
"#,
        &[std::process::id().to_string()],
    );
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn inherited_file_and_socket_descriptors_close_but_stdio_remains_usable() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    let private = tempfile::tempfile().unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut inherited = Vec::new();
    for fd in [private.as_raw_fd(), socket.as_raw_fd()] {
        // SAFETY: valid descriptors, duplicated without CLOEXEC deliberately.
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD, 64) };
        assert!(duplicate >= 64);
        // SAFETY: this test owns each newly allocated descriptor.
        inherited.push(unsafe { OwnedFd::from_raw_fd(duplicate) });
    }
    let args: Vec<_> = inherited
        .iter()
        .map(|fd| fd.as_raw_fd().to_string())
        .collect();
    let output = run_python(
        &LandlockProfile::default(),
        r#"
import errno, os, sys
for fd in sys.argv[1:]:
    try: os.fstat(int(fd))
    except OSError as error: assert error.errno == errno.EBADF
    else: raise AssertionError('inherited descriptor survived exec')
print('stdout works')
print('stderr works', file=sys.stderr)
assert sys.stdin.read() == ''
"#,
        &args,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"stdout works\n");
    assert_eq!(output.stderr, b"stderr works\n");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn compatibility_syscall_abis_are_refused() {
    use std::os::unix::process::ExitStatusExt;
    if detect_abi() < 6 {
        eprintln!("skipped: kernel Landlock ABI below 6");
        return;
    }
    // Both would otherwise bypass a filter comparing only native syscall
    // numbers. Use harmless getpid calls so a regression has no side effects.
    for code in [
        "b8270000400f05c3", // mov eax, __X32_SYSCALL_BIT | SYS_getpid; syscall; ret
        "b814000000cd80c3", // mov eax, i386 SYS_getpid; int 0x80; ret
    ] {
        let output = run_python(
            &LandlockProfile::default(),
            r#"
import ctypes, mmap, resource, sys
resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
code = bytes.fromhex(sys.argv[1])
with mmap.mmap(-1, len(code), prot=mmap.PROT_READ | mmap.PROT_WRITE | mmap.PROT_EXEC) as page:
    page.write(code)
    address = ctypes.addressof(ctypes.c_char.from_buffer(page))
    ctypes.CFUNCTYPE(ctypes.c_long)(address)()
"#,
            &[code.to_string()],
        );
        assert_eq!(output.status.signal(), Some(libc::SIGSYS), "{output:?}");
    }
}

#[test]
fn restricted_children_cannot_restore_namespaces_but_can_start_threads() {
    if detect_abi() < 6 {
        return;
    }
    let domain = prepare(&LandlockProfile::default(), &BoundaryRoots::default()).unwrap();
    let code = format!(
        r#"
import ctypes, errno, os, threading
libc = ctypes.CDLL(None, use_errno=True)
assert libc.unshare(0x10000000) == -1 and ctypes.get_errno() == errno.EACCES
assert libc.setns(-1, 0) == -1 and ctypes.get_errno() == errno.EACCES
assert libc.mount(None, None, None, 0, None) == -1 and ctypes.get_errno() == errno.EACCES
assert libc.syscall({}, None, 0) == -1 and ctypes.get_errno() == errno.ENOSYS
result = libc.syscall({}, 0x10000000 | 17, 0, 0, 0, 0)
if result == 0: os._exit(77)
assert result == -1 and ctypes.get_errno() == errno.EACCES
finished = []
thread = threading.Thread(target=lambda: finished.append(True))
thread.start(); thread.join()
assert finished == [True]
pid = os.fork()
if pid == 0: os._exit(0)
assert os.waitpid(pid, 0)[1] == 0
"#,
        libc::SYS_clone3,
        libc::SYS_clone
    );
    let mut command = Command::new("/usr/bin/python3");
    command.args(["-c", &code]);
    domain.apply_to_std(&mut command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

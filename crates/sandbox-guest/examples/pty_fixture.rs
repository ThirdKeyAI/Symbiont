//! Static guest-only terminal fixture for real transport and shipping tests.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    let canary = std::env::args().nth(1).ok_or("missing host canary path")?;
    // SAFETY: identity/terminal queries use initialized structures and stdio fds.
    let mut settings: libc::termios = unsafe { std::mem::zeroed() };
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(0, &mut settings) }, 0);
    assert_eq!(unsafe { libc::ioctl(0, libc::TIOCGWINSZ, &mut size) }, 0);
    let proof = serde_json::json!({
        "pty": unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 && libc::isatty(2) == 1 },
        "non_root": unsafe { libc::getuid() == 65534 && libc::getgid() == 65534 },
        "controlling_terminal": unsafe { libc::tcgetsid(0) == libc::getpid() && libc::getsid(0) == libc::getpid() },
        "foreground": unsafe { libc::tcgetpgrp(0) == libc::getpgrp() },
        "echo_off": settings.c_lflag & (libc::ECHO | libc::ECHONL) == 0,
        "canonical_off": settings.c_lflag & libc::ICANON == 0,
        "dimensions": size.ws_col == 80 && size.ws_row == 24,
        "host_file_denied": !std::path::Path::new(&canary).exists(),
        "ambient_absent": std::env::var_os("SYMBI_FIRECRACKER_AMBIENT_CANARY").is_none() && std::env::var_os("OPENAI_API_KEY").is_none(),
        "no_network_device": std::fs::read_dir("/sys/class/net")?.all(|entry| entry.is_ok_and(|entry| entry.file_name() == "lo")),
        "no_new_privileges": std::fs::read_to_string("/proc/self/status")?.lines().any(|line| line == "NoNewPrivs:\t1"),
    });
    assert!(
        proof
            .as_object()
            .unwrap()
            .values()
            .all(|value| value == true),
        "{proof}"
    );
    let cookie = std::process::id();
    let mut value = 0u32;
    let mut descendants = Vec::new();
    print!("READY> ");
    std::io::stdout().flush()?;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if let Some(number) = line.strip_prefix("add ") {
            value += number.parse::<u32>()?;
            std::fs::write("/tmp/pty-fixture-value", value.to_string())?;
            assert_eq!(
                std::fs::read_to_string("/tmp/pty-fixture-value")?,
                value.to_string()
            );
        } else if matches!(line.as_str(), "background" | "hang") {
            // The guest supervisor must remove descendants that create a session.
            descendants.push(
                std::process::Command::new("/bin/setsid")
                    .args(["/bin/sh", "-c", "sleep 30; printf late >/tmp/late-effect"])
                    .spawn()?,
            );
            if line == "hang" {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        } else if matches!(line.as_str(), "flood" | "stderr") {
            let bytes = "界".repeat(100_000);
            if line == "stderr" {
                std::io::stderr().write_all(bytes.as_bytes())?;
            } else {
                std::io::stdout().write_all(bytes.as_bytes())?;
            }
        } else if line == "exit" {
            std::process::exit(17);
        } else if !line.starts_with("echo ") {
            return Err("unexpected fixture command".into());
        }
        // stderr is part of the PTY output, not a separate pipe or console log.
        eprintln!("terminal-stderr");
        println!(
            "{}",
            serde_json::json!({"cookie":cookie,"value":value,"proof":proof,"echo":line.strip_prefix("echo ")})
        );
        print!("READY> ");
        std::io::stdout().flush()?;
    }
    for mut child in descendants {
        child.wait()?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("PTY fixture requires Linux");
}

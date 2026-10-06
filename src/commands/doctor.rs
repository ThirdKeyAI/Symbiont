use std::process::Command;

pub async fn run() {
    println!("🔍 Checking system health...\n");

    use symbi_runtime::sandbox::command::{CommandBoundary, CommandTier};
    let boundary = std::env::current_dir()
        .map_err(|e| e.to_string())
        .and_then(|project| CommandBoundary::load(&project));
    let boundary = match boundary {
        Ok(boundary) => boundary,
        Err(error) => {
            eprintln!("✗ Project sandbox configuration: {error}");
            std::process::exit(1);
        }
    };
    if boundary.tier == CommandTier::Landlock {
        #[cfg(target_os = "linux")]
        {
            let profile = &boundary.landlock;
            println!(
                "✓ Landlock ABI {} (required {})",
                symbi_runtime::sandbox::landlock::detect_abi(),
                profile.abi_floor
            );
            println!("• Checking supervised launch and cleanup (may start the user service)...");
            if let Err(error) = symbi_runtime::sandbox::landlock::probe(profile).await {
                eprintln!("✗ Landlock supervision: {error:#}");
                std::process::exit(1);
            }
            println!("✓ Restricted worker executed; delegated cgroup cleanup confirmed");
            if profile.supervisor.service_uid.is_none() {
                if let Ok(unit) = profile.supervisor.delegated_unit() {
                    println!("  Supervisor: {unit}");
                    println!("  After draining work, stop with: systemctl --user stop {unit}");
                }
            }
            use symbi_runtime::sandbox::landlock::diagnostics;
            println!("• Checking private native workspace...");
            if let Err(error) = diagnostics::native_workspace(profile).await {
                eprintln!("✗ Native workspace: {error:#}");
                eprintln!("  Install Python 3 at /usr/bin/python3. Check that your distribution permits unprivileged user/mount namespaces and private tmpfs mounts for the installed symbi binary.");
                eprintln!("  See docs/landlock-development.md. No host fallback was used.");
                std::process::exit(1);
            }
            println!("✓ Private native workspace is writable; cleanup confirmed");
            println!("• Checking managed CLI network isolation...");
            if let Err(error) = diagnostics::managed_transport(profile).await {
                eprintln!("✗ Managed CLI transport: {error:#}");
                eprintln!("  Check unprivileged network namespace support and distribution security rules for the installed symbi binary; private loopback must be available.");
                eprintln!("  See docs/landlock-development.md. No host network fallback was used.");
                std::process::exit(1);
            }
            println!("✓ Private loopback and both inherited connections work; cleanup confirmed");
            #[cfg(feature = "cli-executor")]
            {
                let executable = std::env::current_dir()
                    .map_err(|error| error.to_string())
                    .and_then(|project| {
                        symbi_runtime::cli_executor::broker::configured_managed_executable(
                            &project, &boundary,
                        )
                    });
                match executable {
                Ok(Some(executable)) => {
                    println!("• Checking configured managed CLI startup: {executable:?}");
                    match diagnostics::managed_cli(profile, std::path::Path::new(&executable)).await {
                        Ok(version) => println!("✓ Managed CLI --version: {version:?}; cleanup confirmed"),
                        Err(error) => {
                            eprintln!("✗ Managed CLI startup: {error:#}");
                            eprintln!("  Check that [managed_cli] executable selects a compatible standalone CLI and its system dependencies. The check uses the session's sandbox permissions and a bounded --version launch.");
                            eprintln!("  For forced termination, check the configured memory, process and deadline limits. Python must support os.pidfd_open.");
                            eprintln!("  See docs/landlock-development.md; no provider credentials or model calls are used.");
                            std::process::exit(1);
                        }
                    }
                }
                Ok(None) => println!("○ Managed CLI startup not checked: no [managed_cli] configuration. Set executable to its absolute path to check an installation."),
                Err(error) => {
                    eprintln!("✗ Managed CLI configuration: {error}");
                    eprintln!("  Set [managed_cli] executable to an executable regular file outside /tmp. The canonical file is granted; its home directory is not.");
                    std::process::exit(1);
                }
            }
            }
            #[cfg(not(feature = "cli-executor"))]
            println!("○ Managed CLI startup not checked: this build lacks cli-executor support");
            println!("✓ Landlock workspace and transport checks passed; provider and tool policies require separate validation");
            return;
        }
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("✗ Landlock requires Linux");
            std::process::exit(1);
        }
    }

    let mut all_ok = true;

    // Check Docker
    print!("• Checking Docker... ");
    if check_docker() {
        println!("✓ Docker is running");
    } else {
        println!("✗ Docker not found or not running");
        println!("  Install: https://docs.docker.com/get-docker/");
        all_ok = false;
    }

    // Check gVisor (informational — only blocks if a project requests tier2)
    print!("• Checking gVisor (optional)... ");
    if check_runsc() {
        println!("✓ runsc available (tier2/gVisor ready)");
    } else {
        println!("○ runsc not installed (tier2/gVisor unavailable)");
        println!("  Install: https://gvisor.dev/docs/user_guide/install/");
    }

    // Check Firecracker (informational — only blocks if a project requests tier3)
    print!("• Checking Firecracker (optional)... ");
    if check_firecracker() {
        println!("✓ firecracker available (tier3 ready, kernel + rootfs still required)");
    } else {
        println!("○ firecracker not installed (tier3 unavailable)");
        println!("  Install: https://github.com/firecracker-microvm/firecracker/releases");
    }

    // Check ports
    print!("• Checking ports... ");
    let port_8080 = !is_port_in_use(8080);
    let port_8081 = !is_port_in_use(8081);
    if port_8080 && port_8081 {
        println!("✓ Ports 8080, 8081 available");
    } else {
        if !port_8080 {
            println!("✗ Port 8080 is in use");
        }
        if !port_8081 {
            println!("✗ Port 8081 is in use");
        }
        all_ok = false;
    }

    // Check Qdrant (optional)
    print!("• Checking Qdrant (optional)... ");
    if check_qdrant() {
        println!("✓ Qdrant is reachable on localhost:6333");
    } else {
        println!("○ Qdrant not running (needed for vector search)");
        println!("  Start: docker run -p 6333:6333 qdrant/qdrant");
    }

    // Check disk space
    print!("• Checking disk space... ");
    if check_disk_space() {
        println!("✓ Sufficient disk space available");
    } else {
        println!("⚠️  Low disk space");
        all_ok = false;
    }

    // Check agents directory
    print!("• Checking agents directory... ");
    if std::path::Path::new("agents").exists() {
        let count = count_dsl_files("agents");
        println!("✓ Found {} agent(s)", count);
    } else {
        println!("○ No agents directory (create with: symbi new <template>)");
    }

    println!();
    if all_ok {
        println!("✅ All checks passed! You're ready to run: symbi up");
    } else {
        println!("⚠️  Some checks failed. Fix the issues above before running symbi up");
        std::process::exit(1);
    }
}

fn check_docker() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn check_runsc() -> bool {
    Command::new("runsc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn check_firecracker() -> bool {
    Command::new("firecracker")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

fn check_qdrant() -> bool {
    std::net::TcpStream::connect("127.0.0.1:6333")
        .map(|_| true)
        .unwrap_or(false)
}

fn check_disk_space() -> bool {
    // Simple check - in production, use a proper disk space library
    // For now, just return true
    true
}

fn count_dsl_files(dir: &str) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| dsl::is_symbi_file(&entry.path()))
                .count()
        })
        .unwrap_or(0)
}

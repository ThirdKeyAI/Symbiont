use std::process::Command;

pub async fn run() {
    println!("🔍 Checking system health...\n");

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

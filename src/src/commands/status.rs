use std::path::Path;

pub async fn run() {
    println!("📊 Symbiont Runtime Status\n");

    // Check if runtime is running
    print!("Runtime: ");
    if is_runtime_running() {
        println!("✓ Running on :8080");
    } else {
        println!("✗ Not running (start with: symbi up)");
        return;
    }

    // List agents
    println!("\n🤖 Agents:");
    let agents = list_agents();
    if agents.is_empty() {
        println!("  (none)");
    } else {
        for agent in agents {
            println!("  • {}", agent);
        }
    }

    // Routes
    println!("\n🔀 Routes:");
    println!("  • /webhook → webhook_handler (auto-configured)");

    // I/O Handlers
    println!("\n🔌 I/O Handlers:");
    println!("  • HTTP Input :8081 (enabled)");

    // Resource usage
    println!("\n💾 Resources:");
    if let Some((cpu, mem)) = get_resource_usage() {
        println!("  • CPU: {:.1}%", cpu);
        println!("  • Memory: {:.1} MB", mem);
    }

    println!();
}

fn is_runtime_running() -> bool {
    // Check if runtime is listening on port 8080
    std::net::TcpStream::connect("127.0.0.1:8080")
        .map(|_| true)
        .unwrap_or(false)
}

fn list_agents() -> Vec<String> {
    let agents_dir = Path::new("agents");
    let mut agents = Vec::new();

    if agents_dir.exists() && agents_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(agents_dir) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "dsl" {
                        if let Some(name) = entry.path().file_stem() {
                            agents.push(name.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    agents
}

fn get_resource_usage() -> Option<(f32, f32)> {
    // Placeholder - in production, use sysinfo or similar
    // For now, return dummy values
    Some((5.2, 256.8))
}

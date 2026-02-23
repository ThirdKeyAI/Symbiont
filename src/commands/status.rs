use std::path::Path;
use sysinfo::System;

pub async fn run() {
    println!("📊 Symbiont Runtime Status\n");

    let api_up = is_port_listening(8080);
    let http_up = is_port_listening(8081);

    // Check if runtime is running (either port indicates a running instance)
    print!("Runtime API :8080  ");
    if api_up {
        println!("✓ Running");
    } else {
        println!("✗ Not listening");
    }

    print!("HTTP Input  :8081  ");
    if http_up {
        println!("✓ Running");
    } else {
        println!("✗ Not listening");
    }

    if !api_up && !http_up {
        println!("\n✗ Not running (start with: symbi up)");
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
    if api_up {
        println!("  • /api/v1/* → management API");
        println!("  • /swagger-ui → API documentation");
    }

    // Resource usage
    println!("\n💾 Resources:");
    if let Some((cpu, mem)) = get_resource_usage() {
        println!("  • CPU: {:.1}%", cpu);
        println!("  • Memory: {:.1} MB", mem);
    }

    println!();
}

fn is_port_listening(port: u16) -> bool {
    std::net::TcpStream::connect(format!("127.0.0.1:{}", port))
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
    let mut sys = System::new_all();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu();
    let cpu = sys.global_cpu_info().cpu_usage();
    let mem = sys.used_memory() as f32 / 1024.0 / 1024.0;
    Some((cpu, mem))
}

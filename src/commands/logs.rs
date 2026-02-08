use clap::ArgMatches;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub async fn run(matches: &ArgMatches) {
    let follow = matches.get_flag("follow");
    let lines: usize = matches
        .get_one::<String>("lines")
        .unwrap()
        .parse()
        .unwrap_or(50);

    let log_file = Path::new("symbi.log");

    if !log_file.exists() {
        println!("⚠️  No log file found. Start the runtime with: symbi up");
        return;
    }

    if follow {
        println!("📝 Following logs (Ctrl+C to stop)...\n");
        tail_follow(log_file).await;
    } else {
        println!("📝 Last {} log lines:\n", lines);
        tail_last_n_lines(log_file, lines);
    }
}

fn tail_last_n_lines(path: &Path, n: usize) {
    match File::open(path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            let all_lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

            let start = if all_lines.len() > n {
                all_lines.len() - n
            } else {
                0
            };

            for line in &all_lines[start..] {
                println!("{}", colorize_log_line(line));
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to read log file: {}", e);
        }
    }
}

async fn tail_follow(path: &Path) {
    // Show last 10 lines first
    tail_last_n_lines(path, 10);

    // Simple implementation - in production, use notify or similar
    // For now, just poll the file
    let mut last_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        if let Ok(metadata) = std::fs::metadata(path) {
            let current_size = metadata.len();
            if current_size > last_size {
                // New content available
                if let Ok(file) = File::open(path) {
                    use std::io::Seek;
                    let mut file = file;
                    let _ = file.seek(std::io::SeekFrom::Start(last_size));
                    let reader = BufReader::new(file);
                    for line in reader.lines().map_while(Result::ok) {
                        println!("{}", colorize_log_line(&line));
                    }
                }
                last_size = current_size;
            }
        }
    }
}

fn colorize_log_line(line: &str) -> String {
    // Simple colorization based on log level
    if line.contains("ERROR") || line.contains("✗") {
        format!("\x1b[31m{}\x1b[0m", line) // Red
    } else if line.contains("WARN") || line.contains("⚠️") {
        format!("\x1b[33m{}\x1b[0m", line) // Yellow
    } else if line.contains("INFO") || line.contains("✓") {
        format!("\x1b[32m{}\x1b[0m", line) // Green
    } else if line.contains("DEBUG") {
        format!("\x1b[36m{}\x1b[0m", line) // Cyan
    } else {
        line.to_string()
    }
}

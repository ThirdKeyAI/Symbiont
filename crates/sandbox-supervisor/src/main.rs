#[tokio::main(worker_threads = 2)]
async fn main() {
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some(symbi_sandbox_supervisor::INTERNAL_COMMAND) {
        arguments.remove(0);
    }
    if let Err(error) = symbi_sandbox_supervisor::run(&arguments).await {
        eprintln!("sandbox supervisor failed: {error}");
        std::process::exit(1);
    }
}

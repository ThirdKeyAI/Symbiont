//! PID 1 for an operator-provisioned, read-only Firecracker root filesystem.
#[cfg(target_os = "linux")]
mod linux;

fn main() {
    #[cfg(target_os = "linux")]
    if let Err(error) = linux::run() {
        eprintln!("guest command service failed: {error}");
        std::process::exit(125);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("guest command service requires Linux");
        std::process::exit(125);
    }
}

//! A controlling PTY allocated entirely inside the dedicated guest.
use anyhow::Context;
use std::{
    fs::File,
    os::{fd::FromRawFd, unix::process::CommandExt},
    process::Child,
};

pub(super) fn spawn(request: &symbi_sandbox_guest::Command) -> anyhow::Result<(Child, File)> {
    std::fs::create_dir_all("/dev/pts")?;
    // SAFETY: NUL-terminated constants remain valid throughout mount. Device
    // nodes must work here; this private devpts deliberately omits MS_NODEV.
    if unsafe {
        libc::mount(
            c"devpts".as_ptr(),
            c"/dev/pts".as_ptr(),
            c"devpts".as_ptr(),
            libc::MS_NOSUID | libc::MS_NOEXEC,
            c"newinstance,ptmxmode=0600,mode=0600".as_ptr().cast(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error()).context("mount guest devpts");
    }
    // Open the multiplexer in this exact devpts instance. TIOCGPTPEER avoids
    // resolving a slave pathname between allocation and opening it.
    // SAFETY: open returns a fresh owned CLOEXEC descriptor.
    let fd = unsafe {
        libc::open(
            c"/dev/pts/ptmx".as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("open guest PTY");
    }
    // SAFETY: fd is owned exactly once after successful open.
    let master = unsafe { File::from_raw_fd(fd) };
    let unlocked: libc::c_int = 0;
    // SAFETY: the descriptor and integer pointer are valid for this ioctl.
    if unsafe { libc::ioctl(fd, libc::TIOCSPTLCK, &unlocked) } != 0 {
        return Err(std::io::Error::last_os_error()).context("unlock guest PTY");
    }
    // SAFETY: TIOCGPTPEER returns a new descriptor for this master's slave.
    let peer = unsafe {
        libc::ioctl(
            fd,
            libc::TIOCGPTPEER,
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if peer < 0 {
        return Err(std::io::Error::last_os_error()).context("open guest PTY peer");
    }
    // SAFETY: peer is a fresh descriptor transferred into File exactly once.
    let slave = unsafe { File::from_raw_fd(peer) };
    // SAFETY: the structures are initialized and all descriptors remain owned.
    let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(peer, &mut attributes) } != 0 {
        return Err(std::io::Error::last_os_error()).context("read guest terminal settings");
    }
    // Stream complete approved lines without the kernel canonical-line limit.
    // Keep signal and output processing; the program may configure its own tty.
    attributes.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON);
    attributes.c_cc[libc::VMIN] = 1;
    attributes.c_cc[libc::VTIME] = 0;
    let size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: syscall arguments refer to this owned slave and initialized values.
    if unsafe { libc::tcsetattr(peer, libc::TCSANOW, &attributes) } != 0
        || unsafe { libc::ioctl(peer, libc::TIOCSWINSZ, &size) } != 0
        || unsafe { libc::fchown(peer, 65534, 65534) } != 0
        || unsafe { libc::fchmod(peer, 0o600) } != 0
    {
        return Err(std::io::Error::last_os_error()).context("configure guest terminal");
    }
    let mut child = super::command(request);
    child
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave);
    // SAFETY: only async-signal-safe syscalls run after fork. Command has already
    // installed the slave on stdin; the common hook retains identity/rlimits.
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() < 0
                || libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) != 0
                || libc::tcsetpgrp(libc::STDIN_FILENO, libc::getpid()) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = child.spawn().context("start guest terminal command")?;
    Ok((child, master))
}

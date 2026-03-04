//! Seccomp BPF filter FD setup.
//!
//! Opens the pre-generated seccomp filter file and clears FD_CLOEXEC
//! so the FD number can be passed to bwrap's --seccomp flag.

use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};

use nix::fcntl::{FcntlArg, FdFlag, fcntl};

/// Open a seccomp filter file and clear FD_CLOEXEC so it survives exec.
/// Returns the raw FD number to pass to `bwrap --seccomp <fd>`.
///
/// The caller must keep the `File` alive until after `exec`/`Command::status()`.
pub fn open_seccomp_fd(path: &str) -> io::Result<(File, RawFd)> {
    let file = File::open(path)?;
    let fd = file.as_raw_fd();

    // Clear FD_CLOEXEC so bwrap can read it
    let mut flags = fcntl(fd, FcntlArg::F_GETFD)
        .map(FdFlag::from_bits_truncate)
        .map_err(|e| io::Error::other(format!("fcntl getfd: {e}")))?;
    flags.remove(FdFlag::FD_CLOEXEC);
    fcntl(fd, FcntlArg::F_SETFD(flags))
        .map_err(|e| io::Error::other(format!("fcntl setfd: {e}")))?;

    Ok((file, fd))
}

/// Build seccomp bwrap arguments from the FD number.
pub fn seccomp_args(fd: RawFd) -> Vec<String> {
    vec!["--seccomp".to_string(), fd.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_seccomp_fd_clears_cloexec() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cloister-seccomp-validate-{}", std::process::id()));
        std::fs::write(&path, b"test").unwrap();

        let (_file, fd) = open_seccomp_fd(path.to_str().unwrap()).unwrap();
        let flags = fcntl(fd, FcntlArg::F_GETFD)
            .map(FdFlag::from_bits_truncate)
            .unwrap();
        assert!(!flags.contains(FdFlag::FD_CLOEXEC));

        let _ = std::fs::remove_file(&path);
    }
}

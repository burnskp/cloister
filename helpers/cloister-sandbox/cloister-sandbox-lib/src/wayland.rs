use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

use nix::fcntl::{fcntl, FcntlArg, FdFlag, OFlag};
use nix::unistd::pipe2;
use wayrs_client::Connection;
use wayrs_protocols::security_context_v1::*;

use crate::socket;

/// Check if the compositor supports wp-security-context-v1.
/// Returns true if supported, false otherwise.
pub fn probe() -> bool {
    let mut conn = match Connection::<()>::connect() {
        Ok(c) => c,
        Err(_) => return false,
    };
    if conn.blocking_roundtrip().is_err() {
        return false;
    }
    conn.bind_singleton::<WpSecurityContextManagerV1>(1..=1)
        .is_ok()
}

/// Create a keep-alive pipe pair.
/// The read end has FD_CLOEXEC cleared so it survives exec — the child process
/// keeps it open, and when the child exits the compositor sees POLLHUP on the
/// write end.
pub fn make_keepalive_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let (read_fd, write_fd) =
        pipe2(OFlag::O_CLOEXEC).map_err(|e| io::Error::other(format!("pipe2: {e}")))?;
    let mut fd_flags = fcntl(read_fd.as_raw_fd(), FcntlArg::F_GETFD)
        .map(FdFlag::from_bits_truncate)
        .map_err(|e| io::Error::other(format!("fcntl getfd: {e}")))?;
    fd_flags.remove(FdFlag::FD_CLOEXEC);
    fcntl(read_fd.as_raw_fd(), FcntlArg::F_SETFD(fd_flags))
        .map_err(|e| io::Error::other(format!("fcntl setfd: {e}")))?;
    Ok((read_fd, write_fd))
}

/// Create a security-context listening socket and register it with the compositor.
/// Returns the keep-alive read fd (must survive exec so the child keeps it open).
pub fn setup_context(socket_path: &str, engine: &str, app_id: &str) -> io::Result<OwnedFd> {
    let listener = socket::bind_socket_restricted(socket_path)?;

    // Connect to compositor and bind the security context manager
    let mut conn = Connection::<()>::connect()
        .map_err(|e| io::Error::other(format!("wayland connect: {e}")))?;
    conn.blocking_roundtrip()
        .map_err(|e| io::Error::other(format!("roundtrip: {e}")))?;
    let manager = conn
        .bind_singleton::<WpSecurityContextManagerV1>(1..=1)
        .map_err(|e| io::Error::other(format!("bind security-context-v1: {e:?}")))?;

    // Keep-alive pipe: compositor holds write end, child inherits read end.
    let (read_fd, write_fd) = make_keepalive_pipe()?;

    // Register listener with compositor (consumes listener fd and write fd)
    let ctx = manager.create_listener(&mut conn, listener.into(), write_fd);
    ctx.set_sandbox_engine(
        &mut conn,
        CString::new(engine).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "engine contains null byte")
        })?,
    );
    ctx.set_app_id(
        &mut conn,
        CString::new(app_id).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "app-id contains null byte")
        })?,
    );
    ctx.commit(&mut conn);

    // Roundtrip to ensure the compositor has processed the commit and started
    // accepting connections on the listener fd. A bare flush() only guarantees
    // the bytes reach the socket buffer — without a roundtrip, fast-starting
    // clients (e.g. bwrap → immediate Wayland connect) can race with the
    // compositor's listener setup and get ECONNREFUSED.
    conn.blocking_roundtrip()
        .map_err(|e| io::Error::other(format!("roundtrip after commit: {e}")))?;

    // Disconnect from compositor (bwrap will connect via the restricted socket)
    drop(conn);

    Ok(read_fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::fcntl::{fcntl, FcntlArg, FdFlag};
    use std::os::fd::AsRawFd;

    #[test]
    fn keepalive_pipe_inherits_read_end() {
        let (read_fd, write_fd) = make_keepalive_pipe().expect("pipe");
        let read_flags = fcntl(read_fd.as_raw_fd(), FcntlArg::F_GETFD).expect("get read flags");
        let write_flags = fcntl(write_fd.as_raw_fd(), FcntlArg::F_GETFD).expect("get write flags");
        let read_flags = FdFlag::from_bits_truncate(read_flags);
        let write_flags = FdFlag::from_bits_truncate(write_flags);
        assert!(!read_flags.contains(FdFlag::FD_CLOEXEC));
        assert!(write_flags.contains(FdFlag::FD_CLOEXEC));
    }
}

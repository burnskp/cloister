//! SSH agent filtering proxy runtime.
//!
//! Spawns a Unix socket listener that intercepts SSH agent connections and
//! filters them by key fingerprint, forwarding only allowed operations to the
//! real upstream SSH agent.

use std::io::{self, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::socket;
use crate::ssh_proto::*;

/// Maximum concurrent client connections.
const MAX_CONNECTIONS: usize = 16;

/// Hard ceiling on idle timeout when the user sets timeout_seconds = 0.
/// Prevents connections from sitting idle forever and exhausting the pool.
const HARD_MAX_IDLE_TIMEOUT_SECS: u64 = 3600;

/// Forward a message to the real SSH agent and return the response.
fn forward_to_upstream(upstream: &mut UnixStream, msg: &[u8]) -> io::Result<Vec<u8>> {
    write_message(upstream, msg)?;
    read_message(upstream)
}

/// Write a pre-framed SSH_AGENT_FAILURE directly to a client stream.
fn send_failure(client: &mut UnixStream) -> io::Result<()> {
    client.write_all(SSH_AGENT_FAILURE)
}

/// Handle a single client connection: read messages, filter, and respond.
pub fn handle_client(
    mut client: UnixStream,
    upstream_path: &str,
    allowed: &[String],
    timeout: Option<Duration>,
) {
    let mut upstream = match UnixStream::connect(upstream_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cloister-ssh-filter: upstream connect: {e}");
            let _ = send_failure(&mut client);
            return;
        }
    };

    if let Some(timeout) = timeout {
        let set_timeouts = client
            .set_read_timeout(Some(timeout))
            .and_then(|()| client.set_write_timeout(Some(timeout)))
            .and_then(|()| upstream.set_read_timeout(Some(timeout)))
            .and_then(|()| upstream.set_write_timeout(Some(timeout)));
        if let Err(e) = set_timeouts {
            eprintln!("cloister-ssh-filter: failed to set timeouts: {e}");
            let _ = send_failure(&mut client);
            return;
        }
    }

    loop {
        let msg = match read_message(&mut client) {
            Ok(m) => m,
            Err(_) => return, // client disconnected
        };

        if msg.is_empty() {
            let _ = send_failure(&mut client);
            return;
        }

        let msg_type = msg[0];
        let response = match msg_type {
            SSH_AGENTC_REQUEST_IDENTITIES => match forward_to_upstream(&mut upstream, &msg) {
                Ok(resp) => filter_identities(&resp, allowed),
                Err(e) => {
                    eprintln!("cloister-ssh-filter: upstream forward: {e}");
                    vec![FAILURE_BYTE]
                }
            },
            SSH_AGENTC_SIGN_REQUEST => {
                if is_sign_allowed(&msg, allowed) {
                    match forward_to_upstream(&mut upstream, &msg) {
                        Ok(resp) => resp,
                        Err(e) => {
                            eprintln!("cloister-ssh-filter: upstream forward: {e}");
                            vec![FAILURE_BYTE]
                        }
                    }
                } else {
                    vec![FAILURE_BYTE]
                }
            }
            _ => {
                // Block all other operations (add/remove/lock/extension)
                vec![FAILURE_BYTE]
            }
        };

        if write_message(&mut client, &response).is_err() {
            return;
        }
    }
}

/// Handle returned from `start_listener` — used to clean up when done.
pub struct SshFilterHandle {
    pub socket_path: String,
    running: Arc<AtomicBool>,
}

impl SshFilterHandle {
    /// Stop the listener and clean up the socket file.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        // Unblock accept() with a dummy connect
        let _ = UnixStream::connect(&self.socket_path);
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for SshFilterHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start the SSH filter listener on a background thread.
///
/// Creates a restricted Unix socket at `socket_path`, spawns a listener thread
/// that accepts connections and filters SSH agent operations by fingerprint.
///
/// Returns a handle that must be kept alive for the duration of the sandboxed process.
pub fn start_listener(
    socket_path: &str,
    upstream_path: &str,
    allowed: Vec<String>,
    timeout_seconds: u64,
) -> io::Result<SshFilterHandle> {
    let listener = socket::bind_socket_restricted(socket_path)?;

    let timeout = if timeout_seconds == 0 {
        Some(Duration::from_secs(HARD_MAX_IDLE_TIMEOUT_SECS))
    } else {
        Some(Duration::from_secs(timeout_seconds))
    };

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let allowed = Arc::new(allowed);
    let upstream = Arc::new(upstream_path.to_string());
    let conn_count = Arc::new(AtomicUsize::new(0));

    thread::spawn(move || {
        listener_loop(
            listener,
            running_clone,
            allowed,
            upstream,
            conn_count,
            timeout,
        );
    });

    Ok(SshFilterHandle {
        socket_path: socket_path.to_string(),
        running,
    })
}

fn listener_loop(
    listener: UnixListener,
    running: Arc<AtomicBool>,
    allowed: Arc<Vec<String>>,
    upstream: Arc<String>,
    conn_count: Arc<AtomicUsize>,
    timeout: Option<Duration>,
) {
    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                let prev = conn_count.fetch_add(1, Ordering::AcqRel);
                if prev >= MAX_CONNECTIONS {
                    conn_count.fetch_sub(1, Ordering::AcqRel);
                    eprintln!(
                        "cloister-ssh-filter: connection limit ({MAX_CONNECTIONS}) reached, dropping connection"
                    );
                    drop(client);
                    continue;
                }
                let upstream = upstream.clone();
                let allowed = allowed.clone();
                let conn_count = conn_count.clone();
                thread::spawn(move || {
                    handle_client(client, &upstream, &allowed, timeout);
                    conn_count.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Err(_) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_max_idle_timeout_is_positive_and_bounded() {
        assert!(
            HARD_MAX_IDLE_TIMEOUT_SECS > 0,
            "hard idle timeout must be positive"
        );
        assert!(
            HARD_MAX_IDLE_TIMEOUT_SECS <= 86400,
            "hard idle timeout should not exceed 24 hours"
        );
    }
}

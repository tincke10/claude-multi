//! Run a child process with a hard timeout and a capped stdout.
//!
//! Shared by the statusline chain (user's original command, 1500 ms) and the
//! Keychain read. stderr is discarded. On timeout the child is killed and
//! reaped; any grandchild still holding the stdout pipe is abandoned (the
//! reader thread is detached, never joined past a short grace).

use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// How long to wait for the stdout reader after the child is gone (a leaked
/// grandchild may keep the pipe open).
const READER_GRACE: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub struct TimedOutput {
    /// `None` when the child was killed for exceeding the timeout.
    pub status: Option<ExitStatus>,
    /// At most `out_cap` bytes; the rest of the child's output is discarded.
    pub stdout: Vec<u8>,
    pub timed_out: bool,
}

/// Spawns `cmd` (stdin piped when `stdin` is given, stdout piped and capped,
/// stderr discarded) and waits at most `timeout`.
pub fn run_with_timeout(
    mut cmd: Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
    out_cap: usize,
) -> std::io::Result<TimedOutput> {
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let mut child = cmd.spawn()?;

    // Feed stdin from a thread: a child that never reads must not block us.
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }

    // Drain stdout from a thread into a shared buffer (capped; the excess is
    // read and dropped so the child never blocks on a full pipe).
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    if let Some(mut out) = child.stdout.take() {
        let buf = Arc::clone(&buf);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = out.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if let Ok(mut b) = buf.lock() {
                    let room = out_cap.saturating_sub(b.len());
                    b.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
            let _ = done_tx.send(());
        });
    } else {
        drop(done_tx);
    }

    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        match child.try_wait()? {
            Some(st) => break (Some(st), false),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            None => std::thread::sleep(POLL),
        }
    };

    let _ = done_rx.recv_timeout(READER_GRACE);
    let stdout = buf.lock().map(|b| b.clone()).unwrap_or_default();
    Ok(TimedOutput {
        status,
        stdout,
        timed_out,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    fn test_should_echo_stdin_when_child_is_cat() {
        let out = run_with_timeout(
            Command::new("cat"),
            Some(b"{\"a\":1}"),
            Duration::from_secs(5),
            1024,
        )
        .unwrap();
        assert!(!out.timed_out);
        assert!(out.status.unwrap().success());
        assert_eq!(out.stdout, b"{\"a\":1}");
    }

    #[test]
    fn test_should_report_timed_out_and_kill_child_when_it_sleeps() {
        let start = Instant::now();
        let out = run_with_timeout(sh("sleep 30"), None, Duration::from_millis(300), 1024).unwrap();
        assert!(out.timed_out);
        assert!(out.status.is_none());
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_should_not_hang_when_grandchild_keeps_the_pipe_open() {
        // `sh` stays alive running a non-exec'd pipeline; the timeout must
        // still return promptly.
        let start = Instant::now();
        let out = run_with_timeout(
            sh("sleep 30 | cat; echo done"),
            None,
            Duration::from_millis(300),
            1024,
        )
        .unwrap();
        assert!(out.timed_out);
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn test_should_truncate_stdout_to_cap_when_output_is_larger() {
        let out = run_with_timeout(
            sh("head -c 200000 /dev/zero | tr '\\0' 'a'"),
            None,
            Duration::from_secs(5),
            64 * 1024,
        )
        .unwrap();
        assert!(!out.timed_out);
        assert_eq!(out.stdout.len(), 64 * 1024);
    }

    #[test]
    fn test_should_return_exit_status_when_child_fails() {
        let out = run_with_timeout(sh("exit 3"), None, Duration::from_secs(5), 16).unwrap();
        assert_eq!(out.status.unwrap().code(), Some(3));
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn test_should_error_when_program_does_not_exist() {
        let r = run_with_timeout(
            Command::new("/definitely/not/here"),
            None,
            Duration::from_secs(1),
            16,
        );
        assert!(r.is_err());
    }
}

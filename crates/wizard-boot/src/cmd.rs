//! External commands: an absolute path, a fixed argv, no shell, a cleared
//! environment and a 20 second limit. The seam for tests is [`Runner`].

use std::fmt;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long any command may run before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// How much of a failed program's stderr goes to the log.
const ERR_KEPT: usize = 512;

/// Why a command did not succeed. Never holds what was on its stdin.
#[derive(Debug)]
pub enum CmdError {
    /// The program could not be started.
    Spawn(io::Error),
    /// It ran past [`TIMEOUT`] and was killed.
    Timeout,
    /// It exited with this code (`None`: killed by a signal), and said this
    /// on stderr (cut short).
    Exit(Option<i32>, String),
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmdError::Spawn(e) => write!(f, "could not start: {e}"),
            CmdError::Timeout => write!(f, "did not finish in {} s", TIMEOUT.as_secs()),
            CmdError::Exit(Some(c), msg) => write!(f, "exited with status {c}: {msg}"),
            CmdError::Exit(None, msg) => write!(f, "was killed by a signal: {msg}"),
        }
    }
}

/// Runs one external program. `prog` is an absolute path.
pub trait Runner {
    /// Runs `prog args` with `stdin` (when given) and waits for it.
    ///
    /// # Errors
    /// [`CmdError`]: the callers log it and carry on.
    fn run(&self, prog: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<(), CmdError>;
}

/// Runs the programs for real.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, prog: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<(), CmdError> {
        if !prog.starts_with('/') {
            return Err(CmdError::Spawn(io::Error::new(
                io::ErrorKind::InvalidInput,
                "program path is not absolute",
            )));
        }
        let mut child = Command::new(prog)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin")
            .env("LC_ALL", "C")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(CmdError::Spawn)?;
        // stderr is read while the program runs, so one that writes a lot
        // cannot fill the pipe and stall until the timeout; the first
        // ERR_KEPT bytes are kept for the log.
        let (tx, rx) = std::sync::mpsc::channel();
        if let Some(mut err) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut kept = Vec::new();
                let mut buf = [0u8; 4096];
                while let Ok(n @ 1..) = err.read(&mut buf) {
                    let room = ERR_KEPT.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
                let _ = tx.send(kept);
            });
        }
        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // The input is a few hundred bytes at most, far below the pipe
            // buffer, so this cannot block. A program that exits early
            // gives EPIPE; its exit status tells the story.
            let _ = pipe.write_all(data);
        }
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CmdError::Spawn(e));
                }
            }
            if start.elapsed() >= TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CmdError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        if status.success() {
            return Ok(());
        }
        // A grandchild that kept stderr open must not hold us here.
        let buf = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
        let msg = String::from_utf8_lossy(&buf).trim().replace('\n', " ");
        Err(CmdError::Exit(status.code(), msg))
    }
}

/// Runs a command with no stdin and logs a failure; true on success.
pub fn run_logged(r: &dyn Runner, prog: &str, args: &[&str]) -> bool {
    match r.run(prog, args, None) {
        Ok(()) => true,
        Err(e) => {
            log::error!("{prog} {}: {e}", args.join(" "));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_reports() {
        assert!(SystemRunner.run("/usr/bin/true", &[], None).is_ok());
        let Err(CmdError::Exit(Some(1), _)) = SystemRunner.run("/usr/bin/false", &[], None) else {
            panic!("expected exit 1");
        };
        assert!(matches!(
            SystemRunner.run("/nonexistent/x", &[], None),
            Err(CmdError::Spawn(_))
        ));
        assert!(matches!(
            SystemRunner.run("true", &[], None),
            Err(CmdError::Spawn(_))
        ));
    }

    #[test]
    fn stdin_reaches_the_program() {
        SystemRunner
            .run("/usr/bin/grep", &["-q", "^ok$"], Some(b"ok\n"))
            .unwrap();
        assert!(
            SystemRunner
                .run("/usr/bin/grep", &["-q", "^ok$"], Some(b"no\n"))
                .is_err()
        );
    }

    #[test]
    fn environment_is_cleared() {
        // Whatever HOME the test process has, the child has none.
        assert!(
            SystemRunner
                .run("/usr/bin/sh", &["-c", "test -z \"$HOME\""], None)
                .is_ok()
        );
    }

    #[test]
    fn a_program_that_floods_stderr_does_not_stall() {
        // 1 MiB of stderr, far past the 64 KiB pipe buffer, then a failure:
        // it ends at once and only the first ERR_KEPT bytes are kept
        let start = Instant::now();
        let r = SystemRunner.run(
            "/usr/bin/sh",
            &["-c", "head -c 1048576 /dev/zero | tr '\\0' e >&2; exit 3"],
            None,
        );
        assert!(start.elapsed() < Duration::from_secs(10));
        let Err(CmdError::Exit(Some(3), msg)) = r else {
            panic!("expected exit 3");
        };
        assert_eq!(msg.len(), ERR_KEPT);
    }

    #[test]
    fn a_slow_program_is_not_waited_for_here() {
        // Only the quick path is tested: the 20 s limit itself is covered
        // by reading the loop, a test of it would take 20 s.
        assert!(SystemRunner.run("/usr/bin/sleep", &["0.01"], None).is_ok());
    }
}

//! The terminal of the fallback: a trait the flow talks to, and the real
//! implementation on stdin and stdout (the console, set by the unit's
//! `StandardInput=tty`).

use crate::sig;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::termios::{LocalModes, OptionalActions, Termios, tcgetattr, tcsetattr};
use std::io::Write;
use std::time::Duration;
use zeroize::Zeroizing;

/// What a prompt returned.
pub enum Input {
    /// A line, without its newline. The bytes may be a password.
    Line(Zeroizing<Vec<u8>>),
    /// End of input; the console already waited before returning, so a
    /// caller that asks again cannot spin.
    Eof,
    /// Ctrl+C: the caller starts over.
    Interrupted,
    /// SIGTERM: stop.
    Terminated,
    /// The terminal hung up: stop, and let systemd restart us.
    Hangup,
}

/// Prompts and messages.
pub trait Console {
    /// Shows text (a newline is added).
    fn say(&mut self, text: &str);
    /// Asks for a line, echoing what is typed.
    fn ask(&mut self, prompt: &str) -> Input;
    /// Asks for a line with echo off.
    fn ask_secret(&mut self, prompt: &str) -> Input;
}

/// How long to wait after end of input before asking again.
pub const EOF_WAIT: Duration = Duration::from_secs(2);
/// A line longer than this is cut (a pipe with no newline must not grow
/// memory without bound).
const MAX_LINE: usize = 64 * 1024;

/// Echo off for as long as the value lives; the old modes come back on every
/// path out (return, early return, unwinding).
struct EchoOff {
    saved: Termios,
}

impl EchoOff {
    /// `None` when stdin is not a terminal (a pipe in tests).
    fn new() -> Option<EchoOff> {
        let stdin = rustix::stdio::stdin();
        let saved = tcgetattr(stdin).ok()?;
        let mut quiet = saved.clone();
        quiet.local_modes.remove(LocalModes::ECHO);
        tcsetattr(stdin, OptionalActions::Drain, &quiet).ok()?;
        Some(EchoOff { saved })
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        let _ = tcsetattr(rustix::stdio::stdin(), OptionalActions::Now, &self.saved);
    }
}

/// The real console.
#[derive(Default)]
pub struct Tty {
    pending: Zeroizing<Vec<u8>>,
}

impl Tty {
    /// A console on stdin and stdout.
    pub fn new() -> Tty {
        Tty::default()
    }

    fn print(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        // A terminal that went away is noticed by the read side.
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn take_line(&mut self, upto: usize, skip: usize) -> Input {
        let mut line: Zeroizing<Vec<u8>> = Zeroizing::new(self.pending[..upto].to_vec());
        self.pending.drain(..upto + skip);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Input::Line(line)
    }

    fn read_line(&mut self) -> Input {
        let stdin = rustix::stdio::stdin();
        loop {
            if let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
                return self.take_line(pos, 1);
            }
            if self.pending.len() >= MAX_LINE {
                let n = self.pending.len();
                return self.take_line(n, 0);
            }
            if sig::terminated() {
                return Input::Terminated;
            }
            if sig::hung_up() {
                return Input::Hangup;
            }
            if sig::take_interrupt() {
                self.pending.clear();
                return Input::Interrupted;
            }
            // Wake once a second to look at the signal flags: a signal that
            // lands between the checks above and the read would otherwise
            // be seen only after the next key.
            let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
            let timeout = Timespec {
                tv_sec: 1,
                tv_nsec: 0,
            };
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) | Err(Errno::INTR) => continue,
                Ok(_) => {}
                Err(e) => {
                    log::error!("poll on the terminal failed: {e}");
                    return self.eof();
                }
            }
            let mut chunk = Zeroizing::new([0u8; 256]);
            match rustix::io::read(stdin, &mut chunk[..]) {
                Ok(0) => {
                    if !self.pending.is_empty() {
                        let n = self.pending.len();
                        return self.take_line(n, 0);
                    }
                    return self.eof();
                }
                Ok(n) => self.pending.extend_from_slice(&chunk[..n]),
                Err(Errno::INTR | Errno::AGAIN) => {}
                Err(e) => {
                    log::error!("read from the terminal failed: {e}");
                    return self.eof();
                }
            }
        }
    }

    fn eof(&mut self) -> Input {
        std::thread::sleep(EOF_WAIT);
        Input::Eof
    }
}

impl Console for Tty {
    fn say(&mut self, text: &str) {
        self.print(&format!("{text}\n"));
    }

    fn ask(&mut self, prompt: &str) -> Input {
        self.print(prompt);
        self.read_line()
    }

    fn ask_secret(&mut self, prompt: &str) -> Input {
        self.print(prompt);
        let guard = EchoOff::new();
        let input = self.read_line();
        drop(guard);
        // The Enter key was not echoed.
        self.print("\n");
        input
    }
}

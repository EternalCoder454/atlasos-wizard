//! `fallback`, driven through a real pseudo-terminal: echo is off for the
//! passwords and back on afterwards (also after Ctrl+C and SIGTERM), the
//! errors are plain words, EOF never spins, and the account ends up made.

use super::*;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{LocalModes, tcgetattr};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

const GOOD: &str = "correct horse battery staple 42";
const OTHER: &str = "a different long passphrase 99";

struct Term {
    master: File,
    slave: File,
    seen: String,
    pos: usize,
}

impl Term {
    fn new() -> Term {
        let m = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&m).unwrap();
        unlockpt(&m).unwrap();
        let name = ptsname(&m, Vec::new()).unwrap();
        let slave = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(OsStr::from_bytes(name.to_bytes()))
            .unwrap();
        Term {
            master: File::from(m),
            slave,
            seen: String::new(),
            pos: 0,
        }
    }

    fn spawn(&self, sys: &Sys) -> Child {
        sys.command("fallback")
            .stdin(self.slave.try_clone().unwrap())
            .stdout(self.slave.try_clone().unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn pump(&mut self, wait_ms: i64) {
        let t = Timespec {
            tv_sec: 0,
            tv_nsec: wait_ms * 1_000_000,
        };
        let mut fds = [PollFd::new(&self.master, PollFlags::IN)];
        if poll(&mut fds, Some(&t)).unwrap_or(0) > 0 {
            let mut buf = [0u8; 4096];
            if let Ok(n) = self.master.read(&mut buf) {
                self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    }

    /// Waits until `needle` shows up after the last match.
    fn expect(&mut self, needle: &str) {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(i) = self.seen[self.pos..].find(needle) {
                self.pos += i + needle.len();
                return;
            }
            assert!(
                Instant::now() < end,
                "waited for {needle:?}; the screen shows:\n{}",
                &self.seen[self.pos..]
            );
            self.pump(100);
        }
    }

    fn send(&mut self, text: &str) {
        self.master.write_all(text.as_bytes()).unwrap();
        self.master.write_all(b"\n").unwrap();
    }

    fn echo_on(&self) -> bool {
        tcgetattr(&self.slave)
            .unwrap()
            .local_modes
            .contains(LocalModes::ECHO)
    }
}

fn wait_exit(child: &mut Child, secs: u64) -> std::process::ExitStatus {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return s;
        }
        assert!(Instant::now() < end, "the fallback did not exit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn signal(child: &Child, sig: &str) {
    let ok = Command::new("/usr/bin/kill")
        .args([sig, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(ok.success());
}

fn fresh() -> Sys {
    let s = Sys::new();
    s.write("etc/skel/.bashrc", "# skel\n");
    s
}

#[test]
fn creates_the_account_through_the_terminal() {
    if !is_root() {
        return; // the new home must be chowned to the new uid
    }
    let s = fresh();
    let mut t = Term::new();
    let mut child = t.spawn(&s);

    t.expect("The graphical setup could not start. Create your account here.");
    t.expect("Full name: ");
    t.send("Ada Lovelace");
    t.expect("User name [ada]: ");
    t.send("bad name");
    t.expect("can only have lowercase letters");
    t.expect("User name [ada]: ");
    t.send("root");
    t.expect("reserved for the system");
    t.expect("User name [ada]: ");
    t.send(""); // takes the suggestion
    t.expect("Password: ");
    t.send("short");
    t.expect("at least 8 characters");
    t.expect("Password: ");
    t.send(GOOD);
    t.expect("Password again: ");
    t.send(OTHER);
    t.expect("not the same");
    t.expect("Password: ");
    t.send(GOOD);
    t.expect("Password again: ");
    t.send(GOOD);
    t.expect("Your account is ready");

    assert!(wait_exit(&mut child, 15).success());
    assert!(t.echo_on(), "the terminal is back to echo");
    // What was typed for names is echoed; no password ever was.
    t.pump(200);
    assert!(t.seen.contains("Ada Lovelace"));
    assert!(!t.seen.contains(GOOD), "the password was echoed");
    assert!(!t.seen.contains(OTHER), "the password was echoed");

    assert_eq!(
        s.commands(),
        [
            "/usr/sbin/useradd -m -U -G wheel -c Ada Lovelace -- ada",
            "/usr/sbin/chpasswd -e",
            LOCK_CMDS[0],
            LOCK_CMDS[1],
            DM_CMD,
        ]
    );
    let stdin = s.read("chpasswd.stdin");
    assert!(stdin.starts_with("ada:$y$"), "a yescrypt hash on stdin");
    assert!(!stdin.contains(GOOD));
    assert_eq!(s.marker_state(), (true, true, true));
    assert_eq!(s.state_json()["account"]["stage"], "verified");
    assert_eq!(s.state_json()["finish"], "markers");
    assert!(s.read("etc/shadow").contains("ada:$y$"));
    assert_eq!(s.dropin(), None);
    // the home useradd made (0755 here, as under a login.defs without
    // HOME_MODE) is private by the time the account is verified
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(s.path("home/ada"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700,
        "the new home is private"
    );
}

/// Waits (with a pty) until echo is off: the password prompt is active.
fn wait_echo_off(t: &mut Term) {
    let end = Instant::now() + Duration::from_secs(5);
    while t.echo_on() && Instant::now() < end {
        t.pump(50);
    }
    assert!(!t.echo_on(), "echo is off at the password prompt");
}

#[test]
fn ctrl_c_at_a_password_prompt_starts_over_with_echo_restored() {
    let s = fresh();
    let mut t = Term::new();
    let mut child = t.spawn(&s);
    t.expect("Full name: ");
    t.send("Bob Builder");
    t.expect("User name [bob]: ");
    t.send("");
    t.expect("Password: ");
    wait_echo_off(&mut t);
    signal(&child, "-INT");
    t.expect("Starting over.");
    t.expect("Full name: ");
    assert!(t.echo_on(), "echo is back after Ctrl+C");
    signal(&child, "-TERM");
    assert!(wait_exit(&mut child, 15).success(), "SIGTERM exits 0");
    assert!(t.echo_on());
    assert!(s.commands().is_empty());
}

#[test]
fn sigterm_at_a_password_prompt_restores_the_terminal() {
    let s = fresh();
    let mut t = Term::new();
    let mut child = t.spawn(&s);
    t.expect("Full name: ");
    t.send("Bob Builder");
    t.expect("User name [bob]: ");
    t.send("");
    t.expect("Password: ");
    wait_echo_off(&mut t);
    signal(&child, "-TERM");
    assert!(wait_exit(&mut child, 15).success());
    assert!(t.echo_on(), "echo restored on SIGTERM");
}

#[test]
fn end_of_input_waits_instead_of_spinning() {
    let s = fresh();
    let child = s
        .command("fallback")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(4500));
    signal(&child, "-TERM");
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let prompts = text.matches("Full name: ").count();
    // One prompt now, then one every 2 s: 4.5 s is at most 4.
    assert!((2..=4).contains(&prompts), "{prompts} prompts:\n{text}");
    assert!(s.commands().is_empty());
}

fn piped(s: &Sys, input: &str) -> Output {
    let mut child = s
        .command("fallback")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn a_piped_script_works_without_a_terminal() {
    if !is_root() {
        return;
    }
    let s = fresh();
    let out = piped(&s, &format!("Eve Online\neve\n{GOOD}\n{GOOD}\n"));
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Your account is ready"), "{text}");
    assert!(!text.contains(GOOD));
    assert_eq!(s.marker_state(), (true, true, true));
}

fn add_half_made(s: &Sys, stage: &str) {
    s.write(
        "etc/passwd",
        &format!(
            "{}ada:x:1000:1000:Ada:/home/ada:/bin/bash\n",
            s.read("etc/passwd")
        ),
    );
    s.write(
        "etc/shadow",
        &format!("{}ada:!:19000::::::\n", s.read("etc/shadow")),
    );
    fs::create_dir_all(s.path("home/ada")).unwrap();
    let _ = std::os::unix::fs::chown(s.path("home/ada"), Some(1000), Some(1000));
    s.state(&format!(
        r#"{{"format":1,"boots":1,"account":{{"name":"ada","uid":1000,"stage":"{stage}"}}}}"#
    ));
}

#[test]
fn a_half_made_account_is_deleted_before_asking() {
    if !is_root() {
        return;
    }
    let s = fresh();
    add_half_made(&s, "created");
    fs::copy(s.path("etc/skel/.bashrc"), s.path("home/ada/.bashrc")).unwrap();
    let out = piped(&s, &format!("Ada Lovelace\nada\n{GOOD}\n{GOOD}\n"));
    assert!(out.status.success());
    let cmds = s.commands();
    assert_eq!(cmds[0], "/usr/sbin/userdel -r -- ada", "{cmds:?}");
    assert_eq!(
        cmds[1],
        "/usr/sbin/useradd -m -U -G wheel -c Ada Lovelace -- ada"
    );
    assert_eq!(s.state_json()["account"]["stage"], "verified");
}

#[test]
fn a_home_with_the_users_files_is_never_deleted() {
    if !is_root() {
        return;
    }
    let s = fresh();
    add_half_made(&s, "password-set");
    s.write("home/ada/thesis.odt", "years of work");
    let child = s
        .command("fallback")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    signal(&child, "-TERM");
    let mut child = child;
    child.wait().unwrap();
    assert!(s.exists("home/ada/thesis.odt"));
    assert!(!s.commands().iter().any(|c| c.contains("userdel")));
}

#[test]
fn an_account_that_exists_is_finished_without_asking() {
    if !is_root() {
        return;
    }
    let s = fresh();
    s.add_human("ada", 1000);
    s.state(
        r#"{"format":1,"boots":1,"gave_up":true,"account":{"name":"ada","uid":1000,"stage":"verified"}}"#,
    );
    let out = s.command("fallback").stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Full name"));
    assert_eq!(s.marker_state(), (true, true, true));
    assert!(s.commands().contains(&DM_CMD.to_string()));
}

#[test]
fn useradd_failure_is_told_in_plain_words_and_asked_again() {
    let s = fresh();
    let mut t = Term::new();
    let mut child = s
        .command("fallback")
        .env("TELAMON_WIZARD_TEST_FAIL", "useradd")
        .stdin(t.slave.try_clone().unwrap())
        .stdout(t.slave.try_clone().unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    t.expect("Full name: ");
    t.send("Zed");
    t.expect("User name [zed]: ");
    t.send("");
    t.expect("Password: ");
    t.send(GOOD);
    t.expect("Password again: ");
    t.send(GOOD);
    t.expect("The account could not be created: the system refused to add the user");
    t.expect("Full name: ");
    signal(&child, "-TERM");
    assert!(wait_exit(&mut child, 15).success());
    assert_eq!(s.marker_state(), (false, false, false));
    assert!(!t.seen.contains(GOOD));
}

const CANARY: &str = "CANARY-pw-7f3a-Plum-Orbit";

/// Every byte the fallback leaves anywhere (its terminal, its stderr, the
/// journal text it writes there when there is no journal, the state, the
/// command log, the files it edits) is free of the password, and the hash
/// only reaches the one place it is meant for.
#[test]
fn the_password_is_nowhere_after_a_run() {
    if !is_root() {
        return;
    }
    let s = fresh();
    let mut t = Term::new();
    let errlog = s.path("stderr.txt");
    let mut child = s
        .command("fallback")
        .env("RUST_LOG", "trace")
        .stdin(t.slave.try_clone().unwrap())
        .stdout(t.slave.try_clone().unwrap())
        .stderr(fs::File::create(&errlog).unwrap())
        .spawn()
        .unwrap();
    t.expect("Full name: ");
    t.send("Ada Lovelace");
    t.expect("User name [ada]: ");
    t.send("");
    t.expect("Password: ");
    t.send(CANARY);
    t.expect("Password again: ");
    t.send(CANARY);
    t.expect("Your account is ready");
    assert!(wait_exit(&mut child, 15).success());
    t.pump(200);
    assert!(!t.seen.contains(CANARY), "the terminal showed the password");

    let hash = {
        let shadow = s.read("etc/shadow");
        let line = shadow.lines().find(|l| l.starts_with("ada:")).unwrap();
        line.split(':').nth(1).unwrap().to_string()
    };
    assert!(hash.starts_with("$y$"), "{hash}");
    let mut hash_seen_in = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(s.dir.path(), &mut files);
    for p in files {
        let bytes = fs::read(&p).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(CANARY), "the password is in {}", p.display());
        if text.contains(&hash) {
            hash_seen_in.push(p.strip_prefix(s.dir.path()).unwrap().to_path_buf());
        }
    }
    hash_seen_in.sort();
    assert_eq!(
        hash_seen_in,
        [PathBuf::from("chpasswd.stdin"), PathBuf::from("etc/shadow")],
        "the hash is in shadow and on chpasswd's stdin, nowhere else"
    );
    // not a word of it in argv either
    assert!(!s.read("commands.log").contains(CANARY));
    assert!(!s.read("commands.log").contains(&hash));
}

#[test]
fn sighup_at_a_password_prompt_restores_the_terminal_and_asks_systemd_to_restart() {
    let s = fresh();
    let mut t = Term::new();
    let mut child = t.spawn(&s);
    t.expect("Full name: ");
    t.send("Bob Builder");
    t.expect("User name [bob]: ");
    t.send("");
    t.expect("Password: ");
    wait_echo_off(&mut t);
    signal(&child, "-HUP");
    let status = wait_exit(&mut child, 15);
    assert_eq!(
        status.code(),
        Some(1),
        "a hangup is a failure systemd restarts"
    );
    assert!(t.echo_on(), "echo restored on SIGHUP");
    assert!(s.commands().is_empty());
}

/// A password typed ahead of the prompt (before echo went off) is the
/// kernel's to echo, but one typed at the prompt never is, however fast the
/// keys come: the echo state is switched before the prompt text is written.
#[test]
fn a_password_sent_the_instant_the_prompt_shows_is_not_echoed() {
    let s = fresh();
    let mut t = Term::new();
    let mut child = t.spawn(&s);
    t.expect("Full name: ");
    t.send("Bob Builder");
    t.expect("User name [bob]: ");
    t.send("");
    t.expect("Password: ");
    // keys arrive at once, in several writes, with no pause
    for _ in 0..20 {
        t.master.write_all(CANARY.as_bytes()).unwrap();
    }
    t.master.write_all(b"\n").unwrap();
    t.pump(300);
    assert!(!t.seen.contains("CANARY"), "echoed:\n{}", t.seen);
    signal(&child, "-TERM");
    assert!(wait_exit(&mut child, 15).success());
}

/// The fallback never reads or writes the password through anything but the
/// terminal: its stdout has no password even when stdin is a pipe.
#[test]
fn a_piped_run_leaves_the_password_out_of_stdout_and_stderr() {
    if !is_root() {
        return;
    }
    let s = fresh();
    let mut child = s
        .command("fallback")
        .env("RUST_LOG", "trace")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("Ada Lovelace\nada\n{CANARY}\n{CANARY}\n").as_bytes())
        .unwrap();
    // end of input asks again forever by design: bound the wait
    let end = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > end {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    for (what, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        assert!(
            !String::from_utf8_lossy(bytes).contains(CANARY),
            "{what} holds the password"
        );
    }
}

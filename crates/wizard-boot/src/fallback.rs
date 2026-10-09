//! The text-mode fallback (DESIGN.md, "The fallback"): asks for the full
//! name, user name and password on the console, makes the account with
//! `useradd` and `chpasswd -e` (not AccountsService, so a broken one still
//! ends in an account), writes the done markers, cleans up and starts the
//! display manager. Nothing here logs, prints or stores the password; the
//! hash only travels on `chpasswd`'s stdin.

use crate::cmd::{Runner, run_logged};
use crate::console::{Console, Input};
use crate::half;
use crate::lock;
use crate::paths::Paths;
use crate::prepare::SYSTEMCTL;
use crate::text;
use std::time::SystemTime;
use wizard_core::accounts;
use wizard_core::markers;
use wizard_core::password;
use wizard_core::state::{self, Account, FINISH_MARKERS, Stage, State};
use wizard_core::validate;
use zeroize::Zeroizing;

const USERADD: &str = "/usr/sbin/useradd";
const CHPASSWD: &str = "/usr/sbin/chpasswd";
const DISPLAY_MANAGER: &str = "display-manager.service";

/// How the fallback ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// An account exists and the display manager was started.
    Done,
    /// SIGTERM: stopped on purpose.
    Terminated,
    /// The terminal hung up; the exit code is non-zero so systemd restarts.
    Hangup,
}

/// An answer from the console, or a reason to leave the question.
enum Ask<T> {
    Got(T),
    /// Ctrl+C: start the form over.
    Restart,
    Stop(Outcome),
}

fn state_of(paths: &Paths) -> State {
    match state::load(&paths.state_file()) {
        Ok(l) => {
            if let Some(w) = &l.warning {
                log::warn!("{w}");
            }
            l.state
        }
        Err(e) => {
            // The account matters more than the progress notes.
            log::error!("cannot read the state: {e}; going on without it");
            State::default()
        }
    }
}

fn save(paths: &Paths, s: &State) {
    if let Err(e) = s.save(&paths.state_file()) {
        log::error!("cannot save {}: {e}", paths.state_file().display());
    }
}

fn start_display_manager(run: &dyn Runner) {
    // Conflicts= in the unit means this also stops the fallback itself, and
    // SIGTERM then ends the process: nothing after this call matters.
    if run_logged(run, SYSTEMCTL, &["start", "--no-block", DISPLAY_MANAGER]) {
        log::info!("started {DISPLAY_MANAGER}");
    }
}

/// Markers, state, cleanup: setup is over.
fn finish(paths: &Paths, run: &dyn Runner, state: &mut State) {
    match markers::write_missing(paths.root(), SystemTime::now()) {
        Ok(()) => log::info!("done markers are in place"),
        Err(e) => log::error!("could not write the done markers: {e}"),
    }
    state.finish = Some(FINISH_MARKERS.to_string());
    save(paths, state);
    lock::cleanup(paths, run);
}

fn line_text(i: Input, con: &mut dyn Console) -> Ask<String> {
    match i {
        Input::Line(b) => match String::from_utf8(b.to_vec()) {
            Ok(s) => Ask::Got(s),
            Err(_) => {
                con.say(text::NAME_NOT_TEXT);
                Ask::Got(String::new())
            }
        },
        other => other_input(other),
    }
}

fn other_input<T>(i: Input) -> Ask<T> {
    match i {
        Input::Line(_) | Input::Eof => unreachable!("handled by the caller"),
        Input::Interrupted => Ask::Restart,
        Input::Terminated => Ask::Stop(Outcome::Terminated),
        Input::Hangup => Ask::Stop(Outcome::Hangup),
    }
}

/// Asks until `check` accepts. EOF asks again (the console waits first).
fn ask_valid(
    con: &mut dyn Console,
    prompt: &str,
    mut check: impl FnMut(&str) -> Result<String, &'static str>,
) -> Ask<String> {
    loop {
        let input = con.ask(prompt);
        if matches!(input, Input::Eof) {
            continue;
        }
        let value = match line_text(input, con) {
            Ask::Got(v) => v,
            Ask::Restart => return Ask::Restart,
            Ask::Stop(o) => return Ask::Stop(o),
        };
        match check(&value) {
            Ok(v) => return Ask::Got(v),
            Err(msg) => con.say(msg),
        }
    }
}

/// Asks for the password twice until it passes the rules and matches.
fn ask_password(con: &mut dyn Console, user: &str, full: &str) -> Ask<Zeroizing<Vec<u8>>> {
    loop {
        let first = match con.ask_secret(text::ASK_PASSWORD) {
            Input::Line(b) => b,
            Input::Eof => continue,
            other => return other_input(other),
        };
        if let Err(e) = password::check(&first, user, full) {
            con.say(text::password_error(e));
            continue;
        }
        let second = match con.ask_secret(text::ASK_PASSWORD_AGAIN) {
            Input::Line(b) => b,
            Input::Eof => continue,
            other => return other_input(other),
        };
        if *first != *second {
            con.say(text::PASSWORDS_DIFFER);
            continue;
        }
        return Ask::Got(first);
    }
}

/// The three answers.
fn ask_form(paths: &Paths, con: &mut dyn Console) -> Ask<(String, String, Zeroizing<Vec<u8>>)> {
    let full = match ask_valid(con, text::ASK_FULL_NAME, |v| match validate::full_name(v) {
        Ok("") => Err(text::FULL_NAME_EMPTY),
        Ok(n) => Ok(n.to_string()),
        Err(e) => Err(text::full_name_error(e)),
    }) {
        Ask::Got(v) => v,
        Ask::Restart => return Ask::Restart,
        Ask::Stop(o) => return Ask::Stop(o),
    };
    let suggestion = validate::derive_user_name(&full);
    let suggestion = (validate::user_name(&suggestion).is_ok()).then_some(suggestion);
    let prompt = match &suggestion {
        Some(s) => format!("{} [{s}]: ", text::ASK_USER_NAME),
        None => format!("{}: ", text::ASK_USER_NAME),
    };
    let user = match ask_valid(con, &prompt, |v| {
        let v = v.trim();
        let v = if v.is_empty() {
            suggestion.as_deref().unwrap_or("")
        } else {
            v
        };
        validate::user_name_available(v, &paths.passwd(), &paths.group())
            .map(|()| v.to_string())
            .map_err(text::user_name_error)
    }) {
        Ask::Got(v) => v,
        Ask::Restart => return Ask::Restart,
        Ask::Stop(o) => return Ask::Stop(o),
    };
    match ask_password(con, &user, &full) {
        Ask::Got(p) => Ask::Got((full, user, p)),
        Ask::Restart => Ask::Restart,
        Ask::Stop(o) => Ask::Stop(o),
    }
}

/// Makes the account. On failure the half-made account is removed again and
/// the reason (plain words) is returned.
fn create(
    paths: &Paths,
    run: &dyn Runner,
    state: &mut State,
    full: &str,
    user: &str,
    pw: &[u8],
) -> Result<(), &'static str> {
    let hash = password::hash(pw).map_err(|e| {
        log::error!("hashing failed: {}", e.code());
        text::hash_error(e)
    })?;
    let hash = Zeroizing::new(hash);
    // uid 0 = "not known yet", so a power cut right after `useradd` still
    // leaves a note of which name to clean up (half::remove).
    let mut acct = Account {
        name: user.to_string(),
        uid: 0,
        stage: Stage::Creating,
    };
    state.account = Some(acct.clone());
    save(paths, state);
    let failed = |reason: &'static str, state: &mut State| {
        match half::remove(paths, run, &acct_of(state)) {
            Ok(()) => {
                state.account = None;
                save(paths, state);
            }
            Err(k) => log::error!("the half-made account stays: {k:?}"),
        }
        Err(reason)
    };
    if !run_logged(
        run,
        USERADD,
        &["-m", "-U", "-G", "wheel", "-c", full, "--", user],
    ) {
        return failed(text::REASON_USERADD, state);
    }
    let Some(uid) = find_uid(paths, user) else {
        log::error!("useradd said yes but {user} is not in passwd");
        return failed(text::REASON_USERADD, state);
    };
    acct.uid = uid;
    acct.stage = Stage::Created;
    state.account = Some(acct.clone());
    save(paths, state);

    let mut input = Zeroizing::new(Vec::with_capacity(user.len() + hash.len() + 2));
    input.extend_from_slice(user.as_bytes());
    input.push(b':');
    input.extend_from_slice(hash.as_bytes());
    input.push(b'\n');
    if let Err(e) = run.run(CHPASSWD, &["-e"], Some(&input)) {
        log::error!("{CHPASSWD} -e: {e}");
        return failed(text::REASON_CHPASSWD, state);
    }
    acct.stage = Stage::PasswordSet;
    state.account = Some(acct.clone());
    save(paths, state);

    // `useradd -m` gives the home HOME_MODE, or 0755 under a login.defs
    // without one: take group and other access away before any session of
    // the account can exist.
    match accounts::secure_home(paths.root(), user, uid) {
        Ok(true) => log::info!("the new home was open to others; now 0700 or tighter"),
        Ok(false) => {}
        // `verify` below has the last word (a home that others can write is
        // refused there); a chmod that fails must not by itself leave the
        // machine without an account
        Err(e) => log::error!(
            "securing the home of {user} failed: {}; checking it as it is",
            e.code()
        ),
    }
    match accounts::verify(paths.root(), user, uid) {
        Ok(()) => {}
        // the text mode is the last way to an account: a home whose mode this
        // filesystem will not change (chmod ignored) is logged, not fatal
        Err(accounts::VerifyError::HomeWritableByOthers) => {
            log::error!("the home of {user} stays writable by others; going on");
        }
        Err(e) => {
            log::error!("verify of {user} failed: {}", e.code());
            return failed(text::REASON_VERIFY, state);
        }
    }
    acct.stage = Stage::Verified;
    state.account = Some(acct);
    save(paths, state);
    log::info!("account {user} (uid {uid}) created and verified");
    Ok(())
}

fn acct_of(state: &State) -> Account {
    state.account.clone().unwrap_or(Account {
        name: String::new(),
        uid: 0,
        stage: Stage::Creating,
    })
}

fn find_uid(paths: &Paths, user: &str) -> Option<u32> {
    let text = std::fs::read(paths.passwd()).ok()?;
    accounts::parse_passwd(&String::from_utf8_lossy(&text))
        .into_iter()
        .find(|e| e.name == user)
        .map(|e| e.uid)
}

/// Runs the fallback to its end.
pub fn run(paths: &Paths, run: &dyn Runner, con: &mut dyn Console) -> Outcome {
    let mut state = state_of(paths);

    // Setup already over (the unit was started late, or twice).
    if markers::is_done(paths.root()) {
        con.say(text::ALREADY_SET_UP);
        lock::cleanup(paths, run);
        start_display_manager(run);
        return Outcome::Done;
    }

    // A half-made account (a cut power, a failed helper) goes first.
    if let Some(a) = state.account.clone() {
        let whole = !a.stage.is_half_made()
            || (matches!(a.stage, Stage::Unknown(_))
                && accounts::verify(paths.root(), &a.name, a.uid).is_ok());
        if !whole && half::remove(paths, run, &a).is_ok() {
            state.account = None;
            save(paths, &state);
        }
    }

    // The state says an account was made and verified: finish without asking
    // when passwd agrees, or cannot be read (asking might make a second
    // account). A readable passwd without it means /etc was reset: forget the
    // note and fall through to ask. `finish` alone proves nothing.
    if let Some(a) = state.account.clone().filter(|a| a.stage == Stage::Verified) {
        match accounts::human_accounts(paths.root()) {
            Ok(h) if h.iter().any(|x| x.name == a.name && x.uid == a.uid) => {
                log::info!("the state records a made account; finishing without asking");
                con.say(text::ALREADY_SET_UP);
                finish(paths, run, &mut state);
                start_display_manager(run);
                return Outcome::Done;
            }
            Ok(_) => {
                log::error!(
                    "the state records account {} but passwd does not list it; setting up again",
                    a.name
                );
                state.account = None;
                save(paths, &state);
            }
            Err(e) => {
                log::error!(
                    "cannot read passwd or shadow ({e}) with a verified account in the state; finishing without asking"
                );
                con.say(text::ALREADY_SET_UP);
                finish(paths, run, &mut state);
                start_display_manager(run);
                return Outcome::Done;
            }
        }
    }

    // An account exists (made by the helper before it gave up, or by
    // Anaconda): one account per first run, so finish instead of asking.
    match accounts::human_accounts(paths.root()) {
        Ok(h) if !h.is_empty() => {
            log::info!("an account exists already; finishing without asking");
            con.say(text::ALREADY_SET_UP);
            finish(paths, run, &mut state);
            start_display_manager(run);
            return Outcome::Done;
        }
        Ok(_) => {}
        Err(e) => log::error!("cannot read passwd or shadow: {e}"),
    }

    con.say(text::INTRO);
    loop {
        let (full, user, pw) = match ask_form(paths, con) {
            Ask::Got(t) => t,
            Ask::Restart => {
                con.say(text::STARTING_OVER);
                continue;
            }
            Ask::Stop(o) => return o,
        };
        con.say(text::CREATING);
        match create(paths, run, &mut state, &full, &user, &pw) {
            Ok(()) => break,
            Err(reason) => con.say(&text::create_failed(reason)),
        }
    }
    con.say(text::DONE);
    finish(paths, run, &mut state);
    start_display_manager(run);
    Outcome::Done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;
    use std::collections::VecDeque;
    use std::fs;

    /// Answers from a script; an empty script ends the run (Terminated).
    struct Script {
        answers: VecDeque<Input>,
        said: Vec<String>,
        prompts: Vec<String>,
    }

    impl Script {
        fn new(items: Vec<Input>) -> Script {
            Script {
                answers: items.into(),
                said: Vec::new(),
                prompts: Vec::new(),
            }
        }
        fn next(&mut self) -> Input {
            self.answers.pop_front().unwrap_or(Input::Terminated)
        }
    }

    fn line(s: &str) -> Input {
        Input::Line(Zeroizing::new(s.as_bytes().to_vec()))
    }

    impl Console for Script {
        fn say(&mut self, text: &str) {
            self.said.push(text.to_string());
        }
        fn ask(&mut self, prompt: &str) -> Input {
            self.prompts.push(prompt.to_string());
            self.next()
        }
        fn ask_secret(&mut self, prompt: &str) -> Input {
            self.prompts.push(prompt.to_string());
            self.next()
        }
    }

    const GOOD: &str = "correct horse battery staple 42";

    fn root() -> (tempfile::TempDir, Paths) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::with_root(t.path());
        fs::create_dir_all(t.path().join("etc")).unwrap();
        fs::write(
            p.passwd(),
            "root:x:0:0::/root:/bin/bash\ntelamon-setup:x:975:975::/run/telamon-setup:/bin/sh\n",
        )
        .unwrap();
        fs::write(
            p.shadow(),
            "root:!:1:::::::\ntelamon-setup:!*:19000:0:99999:7:::\n",
        )
        .unwrap();
        fs::write(p.group(), "root:x:0:\nwheel:x:10:\n").unwrap();
        (t, p)
    }

    fn is_root() -> bool {
        rustix::process::getuid().is_root()
    }

    #[test]
    fn happy_path_with_retries() {
        if !is_root() {
            return;
        }
        let (_t, p) = root();
        let f = Fake::new(&p);
        let mut c = Script::new(vec![
            line("Ada Lovelace"),
            line("Root"),  // bad start: uppercase
            line(""),      // takes the suggestion "ada"
            line("short"), // too short
            line(GOOD),
            line("different"), // mismatch
            line(GOOD),
            line(GOOD),
        ]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Done);
        let calls = f.calls();
        assert_eq!(
            calls[0],
            "/usr/sbin/useradd -m -U -G wheel -c Ada Lovelace -- ada"
        );
        assert_eq!(calls[1], "/usr/sbin/chpasswd -e");
        assert!(
            calls.contains(
                &"/usr/bin/systemctl start --no-block display-manager.service".to_string()
            )
        );
        let stdin = fs::read_to_string(p.root().join("chpasswd.stdin")).unwrap();
        assert!(stdin.starts_with("ada:$y$"), "{stdin}");
        assert!(!stdin.contains(GOOD));
        assert!(markers::present(p.root()).all());
        let s = state::load(&p.state_file()).unwrap().state;
        assert_eq!(s.account.unwrap().stage, Stage::Verified);
        assert_eq!(s.finish.as_deref(), Some("markers"));
        // The messages are plain words, and never the password.
        let all = c.said.join("\n");
        assert!(all.contains("must start with a lowercase letter"));
        assert!(all.contains("at least 8 characters"));
        assert!(all.contains("not the same"));
        assert!(!all.contains(GOOD));
        assert!(c.prompts.iter().any(|p| p.contains("[ada]")));
    }

    #[test]
    fn eof_asks_again_and_ctrl_c_starts_over() {
        let (_t, p) = root();
        let f = Fake::new(&p);
        let mut c = Script::new(vec![
            Input::Eof,
            Input::Eof,
            line("Ada"),
            Input::Interrupted,
            line("Bob"),
            Input::Terminated,
        ]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Terminated);
        assert!(c.said.iter().any(|s| s == text::STARTING_OVER));
        assert_eq!(
            c.prompts
                .iter()
                .filter(|p| *p == text::ASK_FULL_NAME)
                .count(),
            4
        );
        assert!(f.calls().is_empty());
    }

    #[test]
    fn useradd_failure_is_reported_and_retried() {
        let (_t, p) = root();
        let f = Fake::new(&p).failing("useradd");
        let mut c = Script::new(vec![
            line("Ada"),
            line("ada"),
            line(GOOD),
            line(GOOD),
            // second round
            line("Ada"),
            line("ada"),
            Input::Terminated,
        ]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Terminated);
        assert!(c.said.iter().any(|s| s.contains("system refused to add")));
        assert!(!markers::is_done(p.root()));
    }

    #[test]
    fn chpasswd_failure_removes_the_half_made_account() {
        if !is_root() {
            return;
        }
        let (_t, p) = root();
        let f = Fake::new(&p).failing("chpasswd");
        let mut c = Script::new(vec![
            line("Ada"),
            line("ada"),
            line(GOOD),
            line(GOOD),
            Input::Terminated,
        ]);
        // skel is absent: the new home is empty, so it counts as pristine
        // (the fake makes no skel copy and the fake root has no /etc/skel).
        assert_eq!(run(&p, &f, &mut c), Outcome::Terminated);
        let calls = f.calls();
        assert!(
            calls.contains(&"/usr/sbin/userdel -r -- ada".to_string()),
            "{calls:?}"
        );
        assert!(!fs::read_to_string(p.passwd()).unwrap().contains("ada:"));
        let s = state::load(&p.state_file()).unwrap().state;
        assert!(s.account.is_none());
    }

    #[test]
    fn an_existing_account_finishes_without_asking() {
        if !is_root() {
            return;
        }
        let (_t, p) = root();
        let f = Fake::new(&p);
        // The helper made it and gave up afterwards.
        let mut pre = Script::new(vec![line("Ada"), line("ada"), line(GOOD), line(GOOD)]);
        assert_eq!(run(&p, &f, &mut pre), Outcome::Done);
        fs::remove_file(markers::telamon_path(p.root())).unwrap();
        fs::remove_file(markers::atlas_path(p.root())).unwrap();
        fs::remove_file(markers::plasma_path(p.root())).unwrap();
        let mut c = Script::new(vec![]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Done);
        assert!(c.prompts.is_empty());
        assert!(markers::present(p.root()).all());
    }

    fn verified_ada(p: &Paths) {
        State {
            account: Some(Account {
                name: "ada".into(),
                uid: 1000,
                stage: Stage::Verified,
            }),
            ..State::default()
        }
        .save(&p.state_file())
        .unwrap();
    }

    fn dm_started(f: &Fake) -> bool {
        f.calls()
            .contains(&"/usr/bin/systemctl start --no-block display-manager.service".into())
    }

    #[test]
    fn a_verified_state_account_in_passwd_finishes_without_asking() {
        let (_t, p) = root();
        fs::write(
            p.passwd(),
            "root:x:0:0::/root:/bin/bash\nada:x:1000:1000:Ada:/home/ada:/bin/bash\n",
        )
        .unwrap();
        fs::write(
            p.shadow(),
            "root:!:1:::::::\nada:$6$salt$hash:19000:0:99999:7:::\n",
        )
        .unwrap();
        verified_ada(&p);
        let f = Fake::new(&p);
        let mut c = Script::new(vec![]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Done);
        assert!(c.prompts.is_empty(), "never asks for a second account");
        assert!(markers::present(p.root()).all());
        assert!(dm_started(&f));
    }

    #[test]
    fn a_verified_state_account_missing_from_passwd_asks_again() {
        if !is_root() {
            return;
        }
        let (_t, p) = root();
        verified_ada(&p);
        let f = Fake::new(&p);
        let mut c = Script::new(vec![line("Ada"), line("ada"), line(GOOD), line(GOOD)]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Done);
        assert!(!c.prompts.is_empty(), "asks: no human account exists");
        assert!(fs::read_to_string(p.passwd()).unwrap().contains("ada:"));
        assert!(markers::present(p.root()).all());
        assert!(dm_started(&f));
    }

    #[test]
    fn a_verified_state_account_with_unreadable_passwd_finishes_without_asking() {
        let (_t, p) = root();
        fs::remove_file(p.passwd()).unwrap();
        fs::create_dir(p.passwd()).unwrap(); // reading a directory fails
        verified_ada(&p);
        let f = Fake::new(&p);
        let mut c = Script::new(vec![]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Done);
        assert!(c.prompts.is_empty());
        assert!(markers::present(p.root()).all());
        assert!(dm_started(&f));
    }

    #[test]
    fn done_machine_starts_the_display_manager_only() {
        let (_t, p) = root();
        markers::write_missing(p.root(), SystemTime::now()).unwrap();
        let f = Fake::new(&p);
        let mut c = Script::new(vec![]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Done);
        assert!(c.prompts.is_empty());
        assert!(
            f.calls()
                .contains(&"/usr/bin/systemctl start --no-block display-manager.service".into())
        );
    }

    #[test]
    fn half_made_account_in_the_state_is_deleted_first() {
        if !is_root() {
            return;
        }
        let (t, p) = root();
        // ada half-made: passwd entry, locked hash, empty home.
        fs::write(
            p.passwd(),
            "root:x:0:0::/root:/bin/bash\nada:x:1000:1000:Ada:/home/ada:/bin/bash\n",
        )
        .unwrap();
        fs::write(p.shadow(), "root:!:1:::::::\nada:!:1:::::::\n").unwrap();
        fs::create_dir_all(t.path().join("home/ada")).unwrap();
        let _ = std::os::unix::fs::chown(t.path().join("home/ada"), Some(1000), Some(1000));
        State {
            account: Some(Account {
                name: "ada".into(),
                uid: 1000,
                stage: Stage::Created,
            }),
            ..State::default()
        }
        .save(&p.state_file())
        .unwrap();
        let f = Fake::new(&p);
        let mut c = Script::new(vec![Input::Terminated]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Terminated);
        assert_eq!(f.calls(), ["/usr/sbin/userdel -r -- ada"]);
    }

    #[test]
    fn non_text_name_is_refused() {
        let (_t, p) = root();
        let f = Fake::new(&p);
        let mut c = Script::new(vec![
            Input::Line(Zeroizing::new(vec![0xff, 0xfe])),
            Input::Terminated,
        ]);
        assert_eq!(run(&p, &f, &mut c), Outcome::Terminated);
        assert!(c.said.iter().any(|s| s == text::NAME_NOT_TEXT));
    }
}

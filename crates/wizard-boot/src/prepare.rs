//! `prepare`: runs at every boot before the display manager. Gathers what
//! `wizard_core::boot::decide` needs, decides, saves the new state, acts.
//!
//! It never fails the boot: a problem is logged and the boot goes on, because
//! a failed `atlas-wizard-boot.service` must not keep the display manager (and
//! so a login screen) from starting.

use crate::cmd::{Runner, run_logged};
use crate::lock;
use crate::paths::Paths;
use std::fs;
use std::io;
use std::time::SystemTime;
use wizard_core::accounts;
use wizard_core::boot::{self, BootAction, BootInput, Cmdline};
use wizard_core::fsutil::write_atomic;
use wizard_core::markers;
use wizard_core::state::{self, Stage, State};

/// The setup autologin, exactly as DESIGN.md gives it.
pub const DROPIN: &str = "[Autologin]\nUser=atlas-setup\nSession=atlas-wizard\nRelogin=true\n";
const DROPIN_MODE: u32 = 0o644;

/// The text-mode fallback, started without waiting (it is ordered after this
/// unit, so waiting would never end).
pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
/// The fallback's unit name.
pub const FALLBACK_UNIT: &str = "atlas-wizard-fallback.service";

/// What `prepare` found that makes a decision impossible.
#[derive(Debug)]
enum Gather {
    /// passwd or shadow could not be read: "no accounts" would be a guess.
    AccountsUnreadable(io::Error),
}

/// The state, and whether it could be read at all.
struct Loaded {
    state: State,
    usable: bool,
}

fn load_state(paths: &Paths) -> Loaded {
    match state::load(&paths.state_file()) {
        Ok(l) => {
            if let Some(w) = &l.warning {
                log::warn!("{w}");
            }
            Loaded {
                state: l.state,
                usable: true,
            }
        }
        Err(e) => {
            log::error!("cannot read {}: {e}", paths.state_file().display());
            Loaded {
                state: State::default(),
                usable: false,
            }
        }
    }
}

fn gather(paths: &Paths, loaded: &mut Option<Loaded>) -> Result<BootInput, Gather> {
    let markers = markers::present(paths.root());
    if markers.any() {
        // `decide` answers from the markers alone (its first row), so the
        // fast path of a finished machine reads nothing else.
        return Ok(BootInput {
            markers,
            ..BootInput::default()
        });
    }
    let cmdline = match fs::read_to_string(paths.cmdline()) {
        Ok(t) => boot::parse_cmdline(&t),
        Err(e) => {
            log::warn!("cannot read the kernel command line: {e}");
            Cmdline::None
        }
    };
    let humans = accounts::human_accounts(paths.root()).map_err(Gather::AccountsUnreadable)?;
    let l = loaded.insert(load_state(paths));
    let state_account_verifies = match &l.state.account {
        // Only consulted for a stage this version does not know.
        Some(a) if matches!(a.stage, Stage::Unknown(_)) => {
            accounts::verify(paths.root(), &a.name, a.uid).is_ok()
        }
        _ => false,
    };
    Ok(BootInput {
        markers,
        cmdline,
        humans,
        state: l.state.clone(),
        state_account_verifies,
    })
}

/// Writes the done markers that are missing; a failure is logged.
fn write_markers(paths: &Paths) {
    match markers::write_missing(paths.root(), SystemTime::now()) {
        Ok(()) => log::info!("done markers are in place"),
        Err(e) => log::error!("could not write the done markers: {e}"),
    }
}

/// Writes the setup autologin unless it is already as it should be.
fn write_dropin(paths: &Paths) -> io::Result<()> {
    let p = paths.dropin();
    if fs::read(&p).is_ok_and(|b| b == DROPIN.as_bytes()) {
        return Ok(());
    }
    write_atomic(&p, DROPIN.as_bytes(), DROPIN_MODE)?;
    log::info!("wrote the setup autologin {}", p.display());
    Ok(())
}

fn start_fallback(paths: &Paths, run: &dyn Runner) {
    lock::remove_dropin(paths);
    if run_logged(run, SYSTEMCTL, &["start", "--no-block", FALLBACK_UNIT]) {
        log::info!("started {FALLBACK_UNIT}");
    }
}

/// Does `action`. Returns what was done, for the log and the tests.
fn act(paths: &Paths, run: &dyn Runner, action: BootAction) -> &'static str {
    match action {
        BootAction::Cleanup {
            write_missing_marker,
        } => {
            if write_missing_marker {
                write_markers(paths);
            }
            lock::cleanup(paths, run);
            "cleanup"
        }
        BootAction::MarkDoneAndCleanup => {
            write_markers(paths);
            lock::cleanup(paths, run);
            "mark-done-and-cleanup"
        }
        BootAction::FinishWithDefaults => {
            log::warn!("finishing with defaults: the settings for the new user are skipped");
            write_markers(paths);
            lock::cleanup(paths, run);
            "finish-with-defaults"
        }
        BootAction::RunWizard {
            resume_after_account,
        } => {
            match write_dropin(paths) {
                Ok(()) => {
                    log::info!(
                        "setup autologin on{}",
                        if resume_after_account {
                            " (resuming after the account)"
                        } else {
                            ""
                        }
                    );
                    "run-wizard"
                }
                Err(e) => {
                    // No way into the graphical setup: the text one is the
                    // way to an account.
                    log::error!("could not write the setup autologin: {e}");
                    start_fallback(paths, run);
                    "fallback"
                }
            }
        }
        BootAction::Fallback => {
            start_fallback(paths, run);
            "fallback"
        }
    }
}

/// Runs `prepare`. Always returns normally; see the module comment.
pub fn run(paths: &Paths, run: &dyn Runner) -> &'static str {
    let mut loaded = None;
    let input = match gather(paths, &mut loaded) {
        Ok(i) => i,
        Err(Gather::AccountsUnreadable(e)) => {
            log::error!("cannot read passwd or shadow ({e}); changing nothing");
            return "nothing";
        }
    };
    let mut action = boot::decide(&input);
    log::info!("decision: {action:?}");

    // The count of boots is what stops a broken graphical setup from coming
    // back forever. When it cannot be kept, the text mode takes over.
    let state_usable = loaded.as_ref().is_none_or(|l| l.usable);
    if !state_usable && matches!(action, BootAction::RunWizard { .. }) {
        log::error!("the state file cannot be read: using the text-mode fallback");
        action = BootAction::Fallback;
    }
    if loaded.is_some() {
        let next = boot::next_state(&input.state, &action);
        if next != input.state
            && let Err(e) = next.save(&paths.state_file())
        {
            log::error!("cannot save {}: {e}", paths.state_file().display());
            if matches!(action, BootAction::RunWizard { .. }) {
                log::error!("boots cannot be counted: using the text-mode fallback");
                action = BootAction::Fallback;
            }
        }
    }
    let done = act(paths, run, action);
    log::info!("prepare done: {done}");
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    fn root() -> (tempfile::TempDir, Paths) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::with_root(t.path());
        fs::create_dir_all(t.path().join("etc")).unwrap();
        fs::write(
            p.passwd(),
            "root:x:0:0::/root:/bin/bash\natlas-setup:x:975:975::/run/atlas-setup:/bin/sh\n",
        )
        .unwrap();
        fs::write(
            p.shadow(),
            "root:!:1:::::::\natlas-setup:!*:19000:0:99999:7:::\n",
        )
        .unwrap();
        (t, p)
    }

    #[test]
    fn first_boot_writes_the_autologin_and_counts() {
        let (_t, p) = root();
        let f = Fake::new(&p);
        assert_eq!(run(&p, &f), "run-wizard");
        assert_eq!(fs::read_to_string(p.dropin()).unwrap(), DROPIN);
        let s = state::load(&p.state_file()).unwrap().state;
        assert_eq!(s.boots, 1);
        assert!(f.calls().is_empty());
    }

    #[test]
    fn unchanged_dropin_is_not_rewritten() {
        let (_t, p) = root();
        let f = Fake::new(&p);
        run(&p, &f);
        let before = fs::metadata(p.dropin()).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_dropin(&p).unwrap();
        assert_eq!(
            fs::metadata(p.dropin()).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn third_boot_goes_to_the_fallback() {
        let (_t, p) = root();
        let f = Fake::new(&p);
        for _ in 0..3 {
            assert_eq!(run(&p, &f), "run-wizard");
        }
        assert_eq!(run(&p, &f), "fallback");
        assert!(!p.dropin().exists());
        assert_eq!(
            f.calls(),
            ["/usr/bin/systemctl start --no-block atlas-wizard-fallback.service"]
        );
    }

    #[test]
    fn done_machine_is_locked_once() {
        let (_t, p) = root();
        markers::write_missing(p.root(), SystemTime::now()).unwrap();
        let f = Fake::new(&p);
        assert_eq!(run(&p, &f), "cleanup");
        assert_eq!(f.calls().len(), 2);
        assert_eq!(run(&p, &f), "cleanup");
        assert_eq!(f.calls().len(), 2, "second boot ran nothing");
        assert!(!p.state_file().exists(), "a done boot never creates state");
    }

    #[test]
    fn unreadable_state_means_text_mode() {
        let (_t, p) = root();
        // A directory where the file should be: reading it is an error.
        fs::create_dir_all(p.state_file()).unwrap();
        let f = Fake::new(&p);
        assert_eq!(run(&p, &f), "fallback");
    }

    #[test]
    fn unwritable_dropin_means_text_mode() {
        let (_t, p) = root();
        // A file where the drop-in directory should be.
        fs::write(p.root().join("etc/plasmalogin.conf.d"), "x").unwrap();
        let f = Fake::new(&p);
        assert_eq!(run(&p, &f), "fallback");
    }
}

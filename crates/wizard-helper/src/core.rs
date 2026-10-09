//! What the four methods do, with no D-Bus in sight: the caller has already
//! been authorized by [`crate::service`]. Everything here is checked again
//! with `wizard-core` (the GUI is never trusted), one call runs at a time, and
//! every step that can be cut short leaves a state the next call can resolve.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::Mutex;
use wizard_core::accounts::{self, UID_MAX, UID_MIN, VerifyError};
use wizard_core::choices::{self, Choices};
use wizard_core::markers;
use wizard_core::password;
use wizard_core::state::{self, Account, FINISH_MARKERS, Stage, State};
use wizard_core::validate;
use zeroize::Zeroizing;

use crate::apply::choices_json;
use crate::backends::{
    AccountsApi, Runner, SYSTEMD_CALL_TIMEOUT, SettingsApplier, SystemdApi, UserSettingsRequest,
};
use crate::error::HelperError;
use crate::paths::Paths;

/// The whole of `CreateAccount` must end within this.
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(120);
/// `Finish` must end within this (the child has 60 s of it).
pub const FINISH_TIMEOUT: Duration = Duration::from_secs(120);

/// `finish` state: the account's settings are being written.
pub const FINISH_SETTINGS: &str = "settings";
/// `finish` state: the autologin drop-in is being written.
pub const FINISH_AUTOLOGIN: &str = "autologin";
/// `finish` state: every step is done.
pub const FINISH_DONE: &str = "done";

/// The state key that keeps "Sign in automatically" until `Finish`.
pub const AUTOLOGIN_KEY: &str = "autologin";

/// The unit `EndSetup` restarts.
pub const DISPLAY_MANAGER: &str = "display-manager.service";
/// The unit `GiveUp` starts.
pub const FALLBACK_UNIT: &str = "telamon-wizard-fallback.service";

/// A value of the `Finish` choices map, as the D-Bus layer converts it.
pub type ChoiceMap = BTreeMap<String, choices::Value>;

/// The logic of the four methods.
pub struct Core {
    paths: Paths,
    accounts: Arc<dyn AccountsApi>,
    systemd: Arc<dyn SystemdApi>,
    runner: Arc<dyn Runner>,
    applier: Arc<dyn SettingsApplier>,
    /// One call at a time; a second one waits here.
    gate: Mutex<()>,
    create_timeout: Duration,
    finish_timeout: Duration,
}

/// Runs `fut` for at most `limit`.
async fn within<T>(
    limit: Duration,
    what: &str,
    fut: impl Future<Output = Result<T, HelperError>>,
) -> Result<T, HelperError> {
    match tokio::time::timeout(limit, fut).await {
        Ok(r) => r,
        Err(_) => {
            log::error!("{what}: timed out after {limit:?}");
            Err(HelperError::failed(
                "timeout",
                "The operation took too long.",
            ))
        }
    }
}

fn io_failed(what: &str, e: &std::io::Error) -> HelperError {
    log::error!("{what}: {}", e.kind());
    HelperError::failed("io", "A file could not be read or written.")
}

/// Setup moved to the root text-mode fallback: the GUI's calls stop.
fn gave_up() -> HelperError {
    HelperError::invalid("gave-up", "Setup moved to text mode.")
}

/// The uid in `/org/freedesktop/Accounts/User<uid>`.
fn uid_from_path(path: &str) -> Option<u32> {
    path.strip_prefix("/org/freedesktop/Accounts/User")?
        .parse()
        .ok()
}

impl Core {
    /// A core over the given backends.
    pub fn new(
        paths: Paths,
        accounts: Arc<dyn AccountsApi>,
        systemd: Arc<dyn SystemdApi>,
        runner: Arc<dyn Runner>,
        applier: Arc<dyn SettingsApplier>,
    ) -> Core {
        Core {
            paths,
            accounts,
            systemd,
            runner,
            applier,
            gate: Mutex::new(()),
            create_timeout: CREATE_TIMEOUT,
            finish_timeout: FINISH_TIMEOUT,
        }
    }

    /// Another limit for `CreateAccount` (tests).
    pub fn with_create_timeout(mut self, limit: Duration) -> Core {
        self.create_timeout = limit;
        self
    }

    /// Another limit for `Finish` (tests).
    pub fn with_finish_timeout(mut self, limit: Duration) -> Core {
        self.finish_timeout = limit;
        self
    }

    /// The paths in use.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    fn load_state(&self) -> Result<State, HelperError> {
        let loaded = state::load(&self.paths.state()).map_err(|e| io_failed("state load", &e))?;
        if let Some(w) = &loaded.warning {
            log::warn!("{w}");
        }
        Ok(loaded.state)
    }

    fn save_state(&self, st: &State) -> Result<(), HelperError> {
        st.save(&self.paths.state())
            .map_err(|e| io_failed("state save", &e))
    }

    fn setup_done(&self) -> bool {
        markers::is_done_or_unknown(self.paths.root())
    }

    fn account_stage(
        &self,
        st: &mut State,
        name: &str,
        uid: u32,
        stage: Stage,
    ) -> Result<(), HelperError> {
        log::info!("account stage: {stage:?} (uid {uid})");
        st.account = Some(Account {
            name: name.to_string(),
            uid,
            stage,
        });
        self.save_state(st)
    }

    // ----- CreateAccount -------------------------------------------------

    /// Makes the account. `password` is zeroed on every path.
    pub async fn create_account(
        &self,
        name: String,
        full_name: String,
        password: Zeroizing<Vec<u8>>,
        autologin: bool,
    ) -> Result<u32, HelperError> {
        log::info!("CreateAccount: start (autologin {autologin})");
        let r = within(self.create_timeout, "CreateAccount", async {
            let _one = self.gate.lock().await;
            self.create_locked(name, full_name, password, autologin)
                .await
        })
        .await;
        match &r {
            Ok(uid) => log::info!("CreateAccount: done (uid {uid})"),
            Err(e) => log::warn!("CreateAccount: refused or failed: {e}"),
        }
        r
    }

    async fn create_locked(
        &self,
        name: String,
        full_name: String,
        password: Zeroizing<Vec<u8>>,
        autologin: bool,
    ) -> Result<u32, HelperError> {
        if self.setup_done() {
            return Err(HelperError::setup_done());
        }
        if self.load_state()?.gave_up {
            return Err(gave_up());
        }

        // Everything that can be refused is refused before AccountsService
        // is touched.
        validate::user_name(&name)
            .map_err(|e| HelperError::invalid(e.code(), "That user name is not allowed."))?;
        let full_name = validate::full_name(&full_name)
            .map_err(|e| HelperError::invalid(e.code(), "That full name is not allowed."))?
            .to_string();
        let hash = {
            let (n, f) = (name.clone(), full_name.clone());
            let (pw, result) = tokio::task::spawn_blocking(move || {
                let r = match password::check(&password, &n, &f) {
                    Ok(_) => password::hash(&password).map(Zeroizing::new).map_err(|e| {
                        HelperError::failed(e.code(), "The password could not be used.")
                    }),
                    Err(e) => Err(HelperError::invalid(
                        e.code(),
                        "That password is not allowed.",
                    )),
                };
                (password, r)
            })
            .await
            .map_err(|_| HelperError::failed("worker", "An internal worker failed."))?;
            drop(pw); // zeroed here, before any account exists
            result?
        };

        let mut st = self.load_state()?;
        if let Some(acc) = st.account.clone() {
            self.resolve_existing(&mut st, &acc).await?;
        }
        validate::user_name_available(&name, &self.paths.passwd(), &self.paths.group())
            .map_err(|e| HelperError::invalid(e.code(), "That user name is not available."))?;

        st.extra
            .insert(AUTOLOGIN_KEY.into(), serde_json::Value::Bool(autologin));
        self.account_stage(&mut st, &name, 0, Stage::Creating)?;
        let user_path = self
            .accounts
            .create_user(&name, &full_name)
            .await
            .map_err(|e| {
                log::error!("CreateUser failed: {e}");
                HelperError::accounts(
                    "accounts-create-failed",
                    "The account could not be created.",
                )
            })?;
        let uid = uid_from_path(&user_path).ok_or_else(|| {
            log::error!("CreateUser returned an unexpected path: {user_path}");
            HelperError::accounts(
                "accounts-bad-reply",
                "The account service gave an unexpected answer.",
            )
        })?;
        // AccountsService is trusted, but a uid outside the human range would
        // later be deleted or chowned by number: refuse it, and leave the state
        // at `creating` so the next call resolves (or refuses) it by name
        if !(UID_MIN..=UID_MAX).contains(&uid) {
            log::error!("CreateUser returned uid {uid}, outside {UID_MIN}..={UID_MAX}");
            return Err(HelperError::accounts(
                "accounts-bad-uid",
                "The account service gave an unexpected answer.",
            ));
        }
        self.account_stage(&mut st, &name, uid, Stage::Created)?;

        self.accounts
            .set_password(&user_path, &hash)
            .await
            .map_err(|e| {
                log::error!("SetPassword failed: {e}");
                HelperError::accounts("accounts-password-failed", "The password could not be set.")
            })?;
        drop(hash);
        self.account_stage(&mut st, &name, uid, Stage::PasswordSet)?;

        // The home's mode is whatever HOME_MODE / UMASK gave (0755 on a
        // system whose login.defs lacks HOME_MODE): take group and other
        // access away before any session of the account can exist.
        match accounts::secure_home(self.paths.root(), &name, uid) {
            Ok(true) => log::info!("the new home was open to others; now 0700 or tighter"),
            Ok(false) => {}
            // `verify` below has the last word (a home that others can write
            // is refused there); a chmod that fails must not by itself leave
            // the machine without an account
            Err(e) => log::error!(
                "securing the new home failed: {}; checking it as it is",
                e.code()
            ),
        }
        accounts::verify(self.paths.root(), &name, uid).map_err(|e| {
            log::error!("verify failed: {}", e.code());
            HelperError::failed(e.code(), "The new account did not check out.")
        })?;
        self.account_stage(&mut st, &name, uid, Stage::Verified)?;
        Ok(uid)
    }

    /// The state already names an account: refuse when it is done, otherwise
    /// delete the half-made one (only that uid, only when its home holds
    /// nothing but skeleton files) and clear it from the state.
    async fn resolve_existing(&self, st: &mut State, acc: &Account) -> Result<(), HelperError> {
        let known_half_made = matches!(
            acc.stage,
            Stage::Creating | Stage::Created | Stage::PasswordSet
        );
        // an unknown stage (from a newer wizard) counts as verified when the
        // account checks out, and as half-made when it does not
        if !known_half_made {
            match accounts::verify(self.paths.root(), &acc.name, acc.uid) {
                Ok(()) => {
                    return Err(HelperError::invalid(
                        "account-exists",
                        "An account was already created.",
                    ));
                }
                // passwd no longer has the account (an /etc reset, an admin's
                // userdel): the note is stale, as boot and the fallback treat it
                Err(e @ (VerifyError::NoPasswdEntry | VerifyError::UidMismatch))
                    if acc.stage == Stage::Verified =>
                {
                    log::warn!(
                        "the verified account ({}, uid {}) is gone ({}); clearing the note",
                        acc.name,
                        acc.uid,
                        e.code()
                    );
                    st.account = None;
                    return self.save_state(st);
                }
                // unreadable or incomplete files: refuse, never guess
                Err(e) if acc.stage == Stage::Verified => {
                    log::error!("verified account cannot be checked: {}", e.code());
                    return Err(HelperError::invalid(
                        "account-exists",
                        "An account was already created.",
                    ));
                }
                // an unknown stage that does not check out counts as half-made
                Err(_) => {}
            }
        }
        log::warn!(
            "a half-made account ({:?}, uid {}) is in the way; deleting it",
            acc.stage,
            acc.uid
        );

        let passwd = std::fs::read_to_string(self.paths.passwd())
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(String::new())
                } else {
                    Err(e)
                }
            })
            .map_err(|e| io_failed("passwd read", &e))?;
        let entry = accounts::parse_passwd(&passwd)
            .into_iter()
            .find(|e| e.name == acc.name);
        if let Some(entry) = &entry {
            // `creating` does not know the uid, so the state vouches for
            // nothing but the name: a real-looking account (a usable hash, a
            // system uid, files in its home) is never deleted on that alone
            if acc.stage == Stage::Creating && acc.uid == 0 {
                let shadow = std::fs::read_to_string(self.paths.join("etc/shadow"))
                    .or_else(|e| {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            Ok(String::new())
                        } else {
                            Err(e)
                        }
                    })
                    .map_err(|e| io_failed("shadow read", &e))?;
                let hash_unusable = accounts::parse_shadow(&shadow)
                    .get(&acc.name)
                    .is_none_or(|h| !accounts::usable_hash(h));
                if !(UID_MIN..=UID_MAX).contains(&entry.uid)
                    || !hash_unusable
                    || !home_holds_only_skel(&self.paths, &entry.home)
                {
                    log::error!(
                        "will not delete {}: stage creating, and the account is not plainly half-made",
                        acc.name
                    );
                    return Err(HelperError::failed(
                        "half-made-account-unclear",
                        "An earlier account could not be told apart from a real one.",
                    ));
                }
            }
            // `creating` does not know the uid yet (0): the passwd entry names it
            let uid_matches =
                acc.uid == entry.uid || (acc.stage == Stage::Creating && acc.uid == 0);
            if !uid_matches || !(UID_MIN..=UID_MAX).contains(&entry.uid) {
                log::error!(
                    "will not delete {}: the state says uid {} and passwd says {}",
                    acc.name,
                    acc.uid,
                    entry.uid
                );
                return Err(HelperError::failed(
                    "half-made-mismatch",
                    "An earlier account does not match what was recorded.",
                ));
            }
            if !home_holds_only_skel(&self.paths, &entry.home) {
                log::error!(
                    "will not delete {}: its home holds more than /etc/skel",
                    acc.name
                );
                return Err(HelperError::failed(
                    "half-made-has-files",
                    "An earlier account has files in its home folder.",
                ));
            }
            self.accounts.delete_user(entry.uid).await.map_err(|e| {
                log::error!("DeleteUser failed: {e}");
                HelperError::accounts(
                    "accounts-delete-failed",
                    "The earlier account could not be removed.",
                )
            })?;
            log::info!("deleted the half-made account (uid {})", entry.uid);
        } else {
            log::info!(
                "the half-made account {} does not exist in passwd",
                acc.name
            );
        }
        st.account = None;
        self.save_state(st)
    }

    // ----- Finish ---------------------------------------------------------

    /// Writes the account's settings and finishes setup.
    pub async fn finish(&self, choice_map: ChoiceMap) -> Result<(), HelperError> {
        log::info!("Finish: start");
        let r = within(self.finish_timeout, "Finish", async {
            let _one = self.gate.lock().await;
            self.finish_locked(choice_map).await
        })
        .await;
        match &r {
            Ok(()) => log::info!("Finish: done"),
            Err(e) => log::warn!("Finish: refused or failed: {e}"),
        }
        r
    }

    async fn finish_locked(&self, choice_map: ChoiceMap) -> Result<(), HelperError> {
        let mut st = self.load_state()?;
        // A repeat after a crash that got as far as the markers only has the
        // clean-up left; once that is recorded, setup is done for good.
        let resuming = self.setup_done() && st.finish.as_deref() == Some(FINISH_MARKERS);
        if self.setup_done() && !resuming {
            return Err(HelperError::setup_done());
        }
        if st.gave_up && !resuming {
            return Err(gave_up());
        }
        let c = choices::validate(&choice_map)
            .map_err(|e| HelperError::invalid(e.code(), "Those choices are not allowed."))?;

        let acc = match &st.account {
            Some(a) if a.stage == Stage::Verified => a.clone(),
            _ => {
                return Err(HelperError::invalid(
                    "no-account",
                    "There is no account to finish.",
                ));
            }
        };
        accounts::verify(self.paths.root(), &acc.name, acc.uid).map_err(|e| {
            log::error!("verify before Finish failed: {}", e.code());
            HelperError::failed(e.code(), "The account did not check out.")
        })?;

        if !resuming {
            self.write_account_settings(&mut st, &acc, &c).await?;
        } else {
            log::info!("Finish: resuming after the markers");
        }

        // after the markers only clean-up is left, and boot's `prepare`
        // repeats it, so a failure there is logged, not returned
        self.remove_setup_autologin();
        self.lock_setup_user().await;
        st.finish = Some(FINISH_DONE.into());
        self.save_state(&st)
    }

    async fn write_account_settings(
        &self,
        st: &mut State,
        acc: &Account,
        c: &Choices,
    ) -> Result<(), HelperError> {
        // the passwd entry gives the gid and the home
        let passwd = std::fs::read_to_string(self.paths.passwd())
            .map_err(|e| io_failed("passwd read", &e))?;
        let entry = accounts::parse_passwd(&passwd)
            .into_iter()
            .find(|e| e.name == acc.name && e.uid == acc.uid)
            .ok_or_else(|| {
                HelperError::failed("verify-no-passwd-entry", "The account did not check out.")
            })?;
        let home = self.paths.join(&entry.home);

        st.finish = Some(FINISH_SETTINGS.into());
        self.save_state(st)?;
        log::info!("Finish: writing the account's settings as uid {}", acc.uid);
        let req = UserSettingsRequest {
            name: acc.name.clone(),
            uid: acc.uid,
            gid: entry.gid,
            home,
            json: choices_json(c),
        };
        self.applier.apply(&req).await?;

        let autologin = st
            .extra
            .get(AUTOLOGIN_KEY)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        st.finish = Some(FINISH_AUTOLOGIN.into());
        self.save_state(st)?;
        if autologin {
            validate::user_name(&acc.name)
                .map_err(|e| HelperError::invalid(e.code(), "That user name is not allowed."))?;
            let text = format!("[Autologin]\nUser={}\nSession=plasma\n", acc.name);
            wizard_core::fsutil::write_atomic(&self.paths.user_autologin(), text.as_bytes(), 0o644)
                .map_err(|e| io_failed("autologin drop-in", &e))?;
            log::info!("Finish: wrote the autologin drop-in");
        }

        st.finish = Some(FINISH_MARKERS.into());
        self.save_state(st)?;
        markers::write_missing(self.paths.root(), SystemTime::now())
            .map_err(|e| io_failed("done markers", &e))?;
        log::info!("Finish: wrote the done markers");
        Ok(())
    }

    /// Removes `99-telamon-wizard.conf`, and Atlas Wizard's
    /// `99-atlas-wizard.conf` (which names a user and a session that are
    /// gone); a missing file is fine.
    fn remove_setup_autologin(&self) -> bool {
        let legacy = self.remove_autologin_file(&self.paths.legacy_setup_autologin());
        self.remove_autologin_file(&self.paths.setup_autologin()) && legacy
    }

    fn remove_autologin_file(&self, path: &std::path::Path) -> bool {
        match std::fs::remove_file(path) {
            Ok(()) => {
                log::info!("removed the setup autologin");
                if let Some(dir) = path.parent()
                    && let Err(e) = wizard_core::fsutil::sync_dir(dir)
                {
                    log::error!("cannot sync the autologin directory: {}", e.kind());
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                log::error!("cannot remove the setup autologin: {}", e.kind());
                false
            }
        }
    }

    /// Locks the setup user: account expired, shell `nologin`. Both commands
    /// are tried; failures are logged.
    async fn lock_setup_user(&self) {
        let user = self.paths.setup_user().to_string();
        for (program, args) in [
            ("/usr/bin/chage", vec!["-E", "0", user.as_str()]),
            (
                "/usr/sbin/usermod",
                vec!["-s", "/usr/sbin/nologin", user.as_str()],
            ),
        ] {
            match self.runner.run(program, &args).await {
                Ok(()) => log::info!("locked the setup user: {program} ok"),
                Err(e) => log::error!("locking the setup user: {e}"),
            }
        }
    }

    // ----- EndSetup and GiveUp -----------------------------------------------

    /// Restarts the display manager once `Finish` is complete.
    pub async fn end_setup(&self) -> Result<(), HelperError> {
        log::info!("EndSetup: start");
        let _one = self.gate.lock().await;
        let mut st = self.load_state()?;
        if self.setup_done() && st.finish.as_deref() == Some(FINISH_MARKERS) {
            // The markers are there but the tail of Finish never ran (a cut,
            // or GiveUp's session retry): run it now, as Finish would, but
            // only for a real account; else `prepare` cleans up at next boot.
            let verified = match &st.account {
                Some(a) if a.stage == Stage::Verified => {
                    accounts::verify(self.paths.root(), &a.name, a.uid).is_ok()
                }
                _ => false,
            };
            if !verified {
                log::warn!("EndSetup: the markers exist but there is no verified account");
                return Err(HelperError::failed(
                    "not-finished",
                    "Setup has not been finished yet.",
                ));
            }
            log::info!("EndSetup: finishing the clean-up after the markers");
            self.remove_setup_autologin();
            self.lock_setup_user().await;
            st.finish = Some(FINISH_DONE.into());
            self.save_state(&st)?;
        }
        if st.finish.as_deref() != Some(FINISH_DONE) {
            log::warn!("EndSetup: Finish has not completed");
            return Err(HelperError::failed(
                "not-finished",
                "Setup has not been finished yet.",
            ));
        }
        // The clean-up is what makes the setup user harmless once the login
        // screen is back: check it, redo it once, and otherwise stay put.
        let mut open = self.cleanup_gaps();
        if !open.is_empty() {
            log::warn!("EndSetup: the clean-up is incomplete ({open:?}); redoing it");
            self.remove_setup_autologin();
            self.lock_setup_user().await;
            open = self.cleanup_gaps();
            if !open.is_empty() {
                log::error!("EndSetup: the clean-up is still incomplete ({open:?})");
                return Err(HelperError::failed(
                    "cleanup-incomplete",
                    "Setup could not be closed completely.",
                ));
            }
        }
        within(SYSTEMD_CALL_TIMEOUT, "EndSetup", async {
            self.systemd
                .restart_unit(DISPLAY_MANAGER)
                .await
                .map_err(|e| {
                    log::error!("RestartUnit failed: {e}");
                    HelperError::failed("restart-failed", "The login screen could not be started.")
                })
        })
        .await?;
        log::info!("EndSetup: restarted {DISPLAY_MANAGER}");
        Ok(())
    }

    /// What of the setup user's clean-up is not in place: the autologin
    /// drop-in is gone, its account is expired (shadow expire field 0 or a
    /// day already past) and its shell is not a login shell. Anything unreadable
    /// counts as not in place.
    fn cleanup_gaps(&self) -> Vec<&'static str> {
        let mut gaps = Vec::new();
        for dropin in [
            self.paths.setup_autologin(),
            self.paths.legacy_setup_autologin(),
        ] {
            match std::fs::symlink_metadata(dropin) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                _ => {
                    gaps.push("autologin-drop-in");
                    break;
                }
            }
        }
        let user = self.paths.setup_user();
        let today = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / 86_400);
        let expired = std::fs::read_to_string(self.paths.join("etc/shadow"))
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.split(':').next() == Some(user))
                    .and_then(|l| l.split(':').nth(7).map(str::to_owned))
            })
            .and_then(|f| f.parse::<u64>().ok())
            .is_some_and(|day| day <= today);
        if !expired {
            gaps.push("account-not-expired");
        }
        let nologin = std::fs::read_to_string(self.paths.passwd())
            .ok()
            .is_some_and(|t| {
                accounts::parse_passwd(&t)
                    .iter()
                    .any(|e| e.name == user && !accounts::login_shell(&e.shell))
            });
        if !nologin {
            gaps.push("shell-not-nologin");
        }
        gaps
    }

    /// Records that the GUI gave up, removes the setup autologin and starts
    /// the text-mode fallback.
    pub async fn give_up(&self) -> Result<(), HelperError> {
        log::info!("GiveUp: start");
        let _one = self.gate.lock().await;
        if self.setup_done() {
            log::warn!("GiveUp: setup is done");
            return Err(HelperError::setup_done());
        }
        let mut st = self.load_state()?;
        // A made account, or a Finish under way, is fine: the text-mode
        // fallback finishes without asking when an account exists, and
        // refusing here would strand the machine (the session script calls
        // GiveUp after 3 failed starts and the fallback would never run).
        if st.finish.is_some()
            || st
                .account
                .as_ref()
                .is_some_and(|a| a.stage == Stage::Verified)
        {
            log::warn!("GiveUp: an account exists; the fallback will finish without asking");
        }
        st.gave_up = true;
        self.save_state(&st)?;
        if !self.remove_setup_autologin() {
            return Err(HelperError::failed(
                "io",
                "A file could not be read or written.",
            ));
        }
        within(SYSTEMD_CALL_TIMEOUT, "GiveUp", async {
            self.systemd.start_unit(FALLBACK_UNIT).await.map_err(|e| {
                log::error!("StartUnit failed: {e}");
                HelperError::failed(
                    "fallback-failed",
                    "The text-mode setup could not be started.",
                )
            })
        })
        .await?;
        log::info!("GiveUp: started {FALLBACK_UNIT}");
        Ok(())
    }
}

/// True when `home` (a passwd home path) does not exist, or is a real
/// directory whose entries are all names found in `/etc/skel`.
pub fn home_holds_only_skel(paths: &Paths, home: &str) -> bool {
    let home_path = Path::new(home);
    if !home_path.is_absolute()
        || home_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    let dir = paths.join(home_path);
    match std::fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Ok(m) if m.is_dir() => {}
        _ => return false,
    }
    let skel: HashSet<std::ffi::OsString> = match std::fs::read_dir(paths.join(crate::paths::SKEL))
    {
        Ok(rd) => rd.filter_map(Result::ok).map(|e| e.file_name()).collect(),
        Err(_) => HashSet::new(),
    };
    match std::fs::read_dir(&dir) {
        Ok(rd) => {
            let mut all = true;
            for e in rd {
                match e {
                    Ok(e) if skel.contains(&e.file_name()) => {}
                    _ => all = false,
                }
            }
            all
        }
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;

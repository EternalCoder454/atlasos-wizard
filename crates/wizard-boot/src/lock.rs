//! Cleanup once setup is done (DESIGN.md, "Locking `telamon-setup`"): remove the
//! setup autologin, lock the setup user, end its session, empty its home.
//!
//! Every step first checks whether it is needed, so a finished machine's boot
//! does no writes, and every step may be repeated after a crash. The helper's
//! `Finish` does the same locking; both are idempotent.

use crate::cmd::{Runner, run_logged};
use crate::paths::{LEGACY_SETUP_USER, Paths, SETUP_USER};
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wizard_core::accounts;

/// `chage` and `usermod` as installed on Fedora (`/usr/sbin` is `/usr/bin`).
const CHAGE: &str = "/usr/bin/chage";
const USERMOD: &str = "/usr/sbin/usermod";
const LOGINCTL: &str = "/usr/bin/loginctl";
const NOLOGIN: &str = "/usr/sbin/nologin";
const SETUP_SHELL: &str = "/bin/sh";
/// `terminate-user` returns before the user's processes are gone; how long to
/// wait for them, so none writes into the home after it is emptied.
const LOGIND_WAIT: Duration = Duration::from_secs(5);

/// Removes the setup autologin drop-in, and Atlas Wizard's under its old name
/// (it would log `atlas-setup` into a session that is gone). True when both
/// are gone afterwards.
pub fn remove_dropin(paths: &Paths) -> bool {
    let new = remove_one_dropin(&paths.dropin());
    let old = remove_one_dropin(&paths.legacy_dropin());
    new && old
}

fn remove_one_dropin(p: &Path) -> bool {
    match fs::remove_file(p) {
        Ok(()) => {
            log::info!("removed the setup autologin {}", p.display());
            if let Some(dir) = p.parent()
                && let Err(e) = wizard_core::fsutil::sync_dir(dir)
            {
                log::warn!("could not sync {}: {e}", dir.display());
            }
            true
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
        Err(e) => {
            log::error!("could not remove {}: {e}", p.display());
            false
        }
    }
}

/// Read a file as text, `None` (logged unless missing) when it cannot be.
fn read_text(p: &Path) -> Option<String> {
    match fs::read(p) {
        Ok(b) => Some(String::from_utf8_lossy(&b).into_owned()),
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::error!("could not read {}: {e}", p.display());
            }
            None
        }
    }
}

/// True when the shadow line for `name` has an expiry date in the past
/// (`chage -E 0` writes day 0).
fn expired(shadow: &str, name: &str, today: i64) -> Option<bool> {
    let line = shadow.lines().find(|l| l.split(':').next() == Some(name))?;
    let field = line.split(':').nth(7).unwrap_or("");
    Some(field.parse::<i64>().is_ok_and(|d| d >= 0 && d <= today))
}

/// Locks `telamon-setup`, and Atlas Wizard's `atlas-setup` when the machine
/// has it.
fn lock_setup_users(paths: &Paths, run: &dyn Runner) {
    lock_setup_user(paths, run, SETUP_USER);
    lock_setup_user(paths, run, LEGACY_SETUP_USER);
}

/// Locks the setup user `user` (expired, nologin shell) where it is not
/// already, and ends its logind user when it has one.
fn lock_setup_user(paths: &Paths, run: &dyn Runner, user: &str) {
    let Some(passwd) = read_text(&paths.passwd()) else {
        return;
    };
    let Some(entry) = accounts::parse_passwd(&passwd)
        .into_iter()
        .find(|e| e.name == user)
    else {
        log::info!("{user} does not exist; nothing to lock");
        return;
    };
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86400) as i64);
    match read_text(&paths.shadow()).and_then(|s| expired(&s, user, today)) {
        Some(true) => {}
        Some(false) => {
            if run_logged(run, CHAGE, &["-E", "0", user]) {
                log::info!("{user}: account expired");
            }
        }
        None => log::warn!("{user} has no shadow entry; not expiring it"),
    }
    if accounts::login_shell(&entry.shell) && run_logged(run, USERMOD, &["-s", NOLOGIN, user]) {
        log::info!("{user}: shell set to {NOLOGIN}");
    }
    let logind = paths.logind_user(entry.uid);
    if logind.exists() && run_logged(run, LOGINCTL, &["terminate-user", user]) {
        let start = Instant::now();
        while logind.exists() && start.elapsed() < LOGIND_WAIT {
            std::thread::sleep(Duration::from_millis(100));
        }
        if logind.exists() {
            log::warn!("{user}: still has processes after terminate-user");
        } else {
            log::info!("{user}: session ended");
        }
    }
}

/// The reverse of locking the setup user, for when the wizard must run again (a
/// cut between Finish's lock and its markers, or markers removed by hand):
/// the setup autologin needs an account that is not expired and a login
/// shell. Each step runs only when passwd or shadow show it is needed.
pub fn unlock_setup_user(paths: &Paths, run: &dyn Runner) {
    let Some(passwd) = read_text(&paths.passwd()) else {
        return;
    };
    let Some(entry) = accounts::parse_passwd(&passwd)
        .into_iter()
        .find(|e| e.name == SETUP_USER)
    else {
        log::warn!("{SETUP_USER} does not exist; cannot unlock it");
        return;
    };
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86400) as i64);
    if read_text(&paths.shadow()).and_then(|s| expired(&s, SETUP_USER, today)) == Some(true)
        && run_logged(run, CHAGE, &["-E", "-1", SETUP_USER])
    {
        log::info!("{SETUP_USER}: account no longer expired");
    }
    if !accounts::login_shell(&entry.shell)
        && run_logged(run, USERMOD, &["-s", SETUP_SHELL, SETUP_USER])
    {
        log::info!("{SETUP_USER}: shell set to {SETUP_SHELL}");
    }
}

/// Removes everything inside `dir`, never following a symlink; the directory
/// stays. Returns how many entries went. An entry that cannot be removed does
/// not stop the rest; the first such error is returned at the end. One that
/// vanished meanwhile is not an error.
pub fn empty_dir(dir: &Path) -> io::Result<usize> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut n = 0;
    let mut first_err = None;
    for entry in rd {
        let removed = entry.and_then(|entry| {
            let p = entry.path();
            // `file_type` of a DirEntry does not follow symlinks.
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            }
        });
        match removed {
            Ok(()) => n += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                log::error!("could not remove an entry of {}: {e}", dir.display());
                first_err.get_or_insert(e);
            }
        }
    }
    first_err.map_or(Ok(n), Err)
}

/// All of the cleanup. Failures are logged and the rest still runs.
pub fn cleanup(paths: &Paths, run: &dyn Runner) {
    remove_dropin(paths);
    lock_setup_users(paths, run);
    match empty_dir(&paths.setup_home()) {
        Ok(0) => {}
        Ok(n) => log::info!("emptied {} ({n} entries)", paths.setup_home().display()),
        Err(e) => log::error!("could not empty {}: {e}", paths.setup_home().display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    const PASSWD: &str = "root:x:0:0::/root:/bin/bash\ntelamon-setup:x:975:975:Telamon Setup:/run/telamon-setup:/bin/sh\n";
    const SHADOW_OPEN: &str = "root:!:1:::::::\ntelamon-setup:!*:19000:0:99999:7:::\n";
    const SHADOW_LOCKED: &str = "root:!:1:::::::\ntelamon-setup:!*:19000:0:99999:7::0:\n";

    fn setup(passwd: &str, shadow: &str) -> (tempfile::TempDir, Paths) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::with_root(t.path());
        fs::create_dir_all(t.path().join("etc")).unwrap();
        fs::write(p.passwd(), passwd).unwrap();
        fs::write(p.shadow(), shadow).unwrap();
        (t, p)
    }

    #[test]
    fn expiry_parsing() {
        assert_eq!(expired(SHADOW_OPEN, "telamon-setup", 20000), Some(false));
        assert_eq!(expired(SHADOW_LOCKED, "telamon-setup", 20000), Some(true));
        assert_eq!(expired(SHADOW_LOCKED, "nobody", 20000), None);
        // -1 means never; a future day is not yet expired.
        assert_eq!(
            expired("a:!:1:::::-1:\n", "a", 20000),
            Some(false),
            "never expires"
        );
        assert_eq!(expired("a:!:1:::::30000\n", "a", 20000), Some(false));
    }

    #[test]
    fn locks_an_open_setup_user() {
        let (_t, p) = setup(PASSWD, SHADOW_OPEN);
        let f = Fake::new(&p);
        cleanup(&p, &f);
        assert_eq!(
            f.calls(),
            [
                "/usr/bin/chage -E 0 telamon-setup",
                "/usr/sbin/usermod -s /usr/sbin/nologin telamon-setup"
            ]
        );
    }

    #[test]
    fn a_locked_machine_runs_no_command_and_writes_nothing() {
        let (_t, p) = setup(
            &PASSWD.replace("/bin/sh", "/usr/sbin/nologin"),
            SHADOW_LOCKED,
        );
        let f = Fake::new(&p);
        cleanup(&p, &f);
        cleanup(&p, &f);
        assert!(f.calls().is_empty());
    }

    #[test]
    fn terminates_only_with_a_logind_user() {
        let (t, p) = setup(
            &PASSWD.replace("/bin/sh", "/usr/sbin/nologin"),
            SHADOW_LOCKED,
        );
        let f = Fake::new(&p);
        cleanup(&p, &f);
        assert!(f.calls().is_empty());
        fs::create_dir_all(t.path().join("run/systemd/users")).unwrap();
        fs::write(p.logind_user(975), "").unwrap();
        cleanup(&p, &f);
        assert_eq!(
            f.calls(),
            ["/usr/bin/loginctl terminate-user telamon-setup"]
        );
    }

    #[test]
    fn unlock_undoes_the_lock_and_only_when_needed() {
        let (_t, p) = setup(
            &PASSWD.replace("/bin/sh", "/usr/sbin/nologin"),
            SHADOW_LOCKED,
        );
        let f = Fake::new(&p);
        unlock_setup_user(&p, &f);
        assert_eq!(
            f.calls(),
            [
                "/usr/bin/chage -E -1 telamon-setup",
                "/usr/sbin/usermod -s /bin/sh telamon-setup"
            ]
        );
        unlock_setup_user(&p, &f);
        assert_eq!(f.calls().len(), 2, "healthy: nothing more");
    }

    #[test]
    fn missing_user_is_fine() {
        let (_t, p) = setup("root:x:0:0::/root:/bin/bash\n", "root:!:1:::::::\n");
        let f = Fake::new(&p);
        cleanup(&p, &f);
        assert!(f.calls().is_empty());
    }

    #[test]
    fn failing_commands_do_not_stop_the_rest() {
        let (t, p) = setup(PASSWD, SHADOW_OPEN);
        fs::create_dir_all(p.setup_home()).unwrap();
        fs::write(p.setup_home().join("answers.json"), "{}").unwrap();
        let f = Fake::new(&p).failing("chage");
        cleanup(&p, &f);
        assert_eq!(f.calls().len(), 2, "usermod still ran");
        assert!(!t.path().join("run/telamon-setup/answers.json").exists());
    }

    #[test]
    fn terminate_waits_for_logind_to_let_go() {
        let (_t, p) = setup(PASSWD, SHADOW_LOCKED);
        fs::create_dir_all(p.logind_user(975).parent().unwrap()).unwrap();
        fs::write(p.logind_user(975), "").unwrap();
        let start = std::time::Instant::now();
        cleanup(&p, &Fake::new(&p));
        // the fake ends the user at once: no waiting the full LOGIND_WAIT
        assert!(start.elapsed() < LOGIND_WAIT);
        assert!(!p.logind_user(975).exists());
    }

    #[test]
    fn empty_dir_goes_on_past_an_entry_it_cannot_remove() {
        if rustix::process::getuid().is_root() {
            return; // root removes from a read-only directory too
        }
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        fs::create_dir_all(home.join("stuck")).unwrap();
        fs::write(home.join("stuck/inner"), "x").unwrap();
        fs::write(home.join("a"), "x").unwrap();
        fs::write(home.join("z"), "x").unwrap();
        fs::set_permissions(home.join("stuck"), fs::Permissions::from_mode(0o555)).unwrap();
        assert!(empty_dir(&home).is_err());
        fs::set_permissions(home.join("stuck"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!home.join("a").exists() && !home.join("z").exists());
    }

    #[test]
    fn empty_dir_does_not_follow_symlinks() {
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("victim");
        fs::create_dir(&victim).unwrap();
        fs::write(victim.join("keep"), "x").unwrap();
        let home = t.path().join("home");
        fs::create_dir_all(home.join("sub/deeper")).unwrap();
        fs::write(home.join("sub/deeper/f"), "x").unwrap();
        std::os::unix::fs::symlink(&victim, home.join("link")).unwrap();
        assert_eq!(empty_dir(&home).unwrap(), 2);
        assert!(victim.join("keep").exists());
        assert!(home.exists());
        assert_eq!(empty_dir(&t.path().join("none")).unwrap(), 0);
    }

    #[test]
    fn dropin_removal_is_quiet_when_absent() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::with_root(t.path());
        assert!(remove_dropin(&p));
        fs::create_dir_all(p.dropin().parent().unwrap()).unwrap();
        fs::write(p.dropin(), "x").unwrap();
        assert!(remove_dropin(&p));
        assert!(!p.dropin().exists());
    }
}

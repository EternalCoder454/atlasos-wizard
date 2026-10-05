//! Cleanup once setup is done (DESIGN.md, "Locking `atlas-setup`"): remove the
//! setup autologin, lock the setup user, end its session, empty its home.
//!
//! Every step first checks whether it is needed, so a finished machine's boot
//! does no writes, and every step may be repeated after a crash. The helper's
//! `Finish` does the same locking; both are idempotent.

use crate::cmd::{Runner, run_logged};
use crate::paths::{Paths, SETUP_USER};
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use wizard_core::accounts;

/// `chage` and `usermod` as installed on Fedora (`/usr/sbin` is `/usr/bin`).
const CHAGE: &str = "/usr/bin/chage";
const USERMOD: &str = "/usr/sbin/usermod";
const LOGINCTL: &str = "/usr/bin/loginctl";
const NOLOGIN: &str = "/usr/sbin/nologin";

/// Removes the setup autologin drop-in. True when it is gone afterwards.
pub fn remove_dropin(paths: &Paths) -> bool {
    let p = paths.dropin();
    match fs::remove_file(&p) {
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

/// Locks `atlas-setup` (expired, nologin shell) where it is not already, and
/// ends its logind user when it has one.
fn lock_setup_user(paths: &Paths, run: &dyn Runner) {
    let Some(passwd) = read_text(&paths.passwd()) else {
        return;
    };
    let Some(entry) = accounts::parse_passwd(&passwd)
        .into_iter()
        .find(|e| e.name == SETUP_USER)
    else {
        log::info!("{SETUP_USER} does not exist; nothing to lock");
        return;
    };
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86400) as i64);
    match read_text(&paths.shadow()).and_then(|s| expired(&s, SETUP_USER, today)) {
        Some(true) => {}
        Some(false) => {
            if run_logged(run, CHAGE, &["-E", "0", SETUP_USER]) {
                log::info!("{SETUP_USER}: account expired");
            }
        }
        None => log::warn!("{SETUP_USER} has no shadow entry; not expiring it"),
    }
    if accounts::login_shell(&entry.shell) && run_logged(run, USERMOD, &["-s", NOLOGIN, SETUP_USER])
    {
        log::info!("{SETUP_USER}: shell set to {NOLOGIN}");
    }
    if paths.logind_user(entry.uid).exists()
        && run_logged(run, LOGINCTL, &["terminate-user", SETUP_USER])
    {
        log::info!("{SETUP_USER}: session ended");
    }
}

/// Removes everything inside `dir`, never following a symlink; the directory
/// stays. Returns how many entries went.
pub fn empty_dir(dir: &Path) -> io::Result<usize> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut n = 0;
    for entry in rd {
        let entry = entry?;
        let p = entry.path();
        // `file_type` of a DirEntry does not follow symlinks.
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
        n += 1;
    }
    Ok(n)
}

/// All of the cleanup. Failures are logged and the rest still runs.
pub fn cleanup(paths: &Paths, run: &dyn Runner) {
    remove_dropin(paths);
    lock_setup_user(paths, run);
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

    const PASSWD: &str = "root:x:0:0::/root:/bin/bash\natlas-setup:x:975:975:AtlasOS Setup:/run/atlas-setup:/bin/sh\n";
    const SHADOW_OPEN: &str = "root:!:1:::::::\natlas-setup:!*:19000:0:99999:7:::\n";
    const SHADOW_LOCKED: &str = "root:!:1:::::::\natlas-setup:!*:19000:0:99999:7::0:\n";

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
        assert_eq!(expired(SHADOW_OPEN, "atlas-setup", 20000), Some(false));
        assert_eq!(expired(SHADOW_LOCKED, "atlas-setup", 20000), Some(true));
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
                "/usr/bin/chage -E 0 atlas-setup",
                "/usr/sbin/usermod -s /usr/sbin/nologin atlas-setup"
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
        assert_eq!(f.calls(), ["/usr/bin/loginctl terminate-user atlas-setup"]);
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
        assert!(!t.path().join("run/atlas-setup/answers.json").exists());
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

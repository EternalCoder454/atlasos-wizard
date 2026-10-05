//! Removing a half-made account (DESIGN.md: "deleted, made again"): only the
//! account the state names, only for its uid, and only while its home holds
//! nothing beyond what `/etc/skel` put there.

use crate::cmd::{Runner, run_logged};
use crate::paths::Paths;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use wizard_core::accounts::{self, UID_MAX, UID_MIN};
use wizard_core::state::{Account, Stage};
use wizard_core::validate;

const USERDEL: &str = "/usr/sbin/userdel";
const MAX_DEPTH: u32 = 16;

/// Why a half-made account was left alone.
#[derive(Debug, PartialEq, Eq)]
pub enum Kept {
    /// passwd has the name with another uid, or a system uid.
    NotOurs,
    /// The home holds files the user may have made.
    HomeHasData,
    /// `userdel` failed.
    UserdelFailed,
    /// passwd could not be read.
    Unreadable,
}

/// Every entry under `home` is also under `skel` with the same type, size
/// and (for a link) target.
fn same_as_skel(skel: &Path, home: &Path, depth: u32) -> io::Result<bool> {
    if depth > MAX_DEPTH {
        return Ok(false);
    }
    for entry in fs::read_dir(home)? {
        let entry = entry?;
        let theirs = entry.metadata()?; // does not follow a link
        let ours = match fs::symlink_metadata(skel.join(entry.file_name())) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let ft = theirs.file_type();
        if ft != ours.file_type() {
            return Ok(false);
        }
        let same = if ft.is_dir() {
            same_as_skel(&skel.join(entry.file_name()), &entry.path(), depth + 1)?
        } else if ft.is_symlink() {
            fs::read_link(entry.path())? == fs::read_link(skel.join(entry.file_name()))?
        } else if ft.is_file() {
            theirs.len() == ours.len()
        } else {
            false
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Deletes the account in `acct` (name, and uid unless it is 0 at stage
/// `creating`, which means "not known yet": the account was started but its
/// uid never recorded). With the uid unknown the state vouches for nothing
/// but the name, so the account must also look plainly half-made: no usable
/// password hash (as `useradd` leaves it), as the helper requires. A
/// pre-existing account of that name is never deleted on the name alone.
/// `Ok(())` also when there is nothing to delete.
///
/// # Errors
/// [`Kept`], with the reason it was left.
pub fn remove(paths: &Paths, run: &dyn Runner, acct: &Account) -> Result<(), Kept> {
    // The state is root's, but a name that is not one `useradd` would take
    // never reaches `userdel`.
    if validate::user_name(&acct.name).is_err() {
        log::error!("not deleting an account the state names oddly");
        return Err(Kept::NotOurs);
    }
    let uid_unknown = acct.uid == 0 && acct.stage == Stage::Creating;
    let passwd = match fs::read(paths.passwd()) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            log::error!("cannot read passwd: {e}");
            return Err(Kept::Unreadable);
        }
    };
    let Some(entry) = accounts::parse_passwd(&passwd)
        .into_iter()
        .find(|e| e.name == acct.name)
    else {
        return Ok(());
    };
    if !(UID_MIN..=UID_MAX).contains(&entry.uid) || (!uid_unknown && entry.uid != acct.uid) {
        log::error!(
            "not deleting {}: uid {} is not the account the setup made",
            acct.name,
            entry.uid
        );
        return Err(Kept::NotOurs);
    }
    if uid_unknown {
        let shadow = match fs::read(paths.shadow()) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                log::error!("cannot read shadow: {e}");
                return Err(Kept::Unreadable);
            }
        };
        if accounts::parse_shadow(&shadow)
            .get(&acct.name)
            .is_some_and(|h| accounts::usable_hash(h))
        {
            log::error!(
                "not deleting {}: its uid was never recorded and it has a password",
                acct.name
            );
            return Err(Kept::NotOurs);
        }
    }
    let home = Path::new(&entry.home);
    if home.is_absolute()
        && !home
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        let real = paths.under_root(&entry.home);
        match fs::symlink_metadata(&real) {
            Ok(m) => {
                let pristine = m.is_dir()
                    && m.uid() == entry.uid
                    && same_as_skel(&paths.skel(), &real, 0).unwrap_or(false);
                if !pristine {
                    log::error!(
                        "not deleting {}: its home holds more than /etc/skel",
                        acct.name
                    );
                    return Err(Kept::HomeHasData);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                log::error!("cannot look at the home of {}: {e}", acct.name);
                return Err(Kept::HomeHasData);
            }
        }
    } else {
        log::error!("not deleting {}: odd home path", acct.name);
        return Err(Kept::NotOurs);
    }
    if run_logged(run, USERDEL, &["-r", "--", &acct.name]) {
        log::info!("deleted the half-made account {}", acct.name);
        Ok(())
    } else {
        Err(Kept::UserdelFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    fn acct(uid: u32) -> Account {
        Account {
            name: "ada".into(),
            uid,
            stage: Stage::Created,
        }
    }

    /// Started, uid never recorded.
    fn creating() -> Account {
        Account {
            name: "ada".into(),
            uid: 0,
            stage: Stage::Creating,
        }
    }

    /// A root with skel (.bashrc), passwd with ada (uid 1000) and her home.
    fn root(home_extra: Option<&str>) -> (tempfile::TempDir, Paths) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::with_root(t.path());
        fs::create_dir_all(p.skel()).unwrap();
        fs::write(p.skel().join(".bashrc"), "# bashrc\n").unwrap();
        fs::create_dir_all(p.skel().join(".config")).unwrap();
        let home = t.path().join("home/ada");
        fs::create_dir_all(home.join(".config")).unwrap();
        fs::write(home.join(".bashrc"), "# bashrc\n").unwrap();
        if let Some(f) = home_extra {
            fs::write(home.join(f), "mine").unwrap();
        }
        let _ = std::os::unix::fs::chown(&home, Some(1000), Some(1000));
        fs::write(
            p.passwd(),
            "root:x:0:0::/root:/bin/bash\nada:x:1000:1000:Ada:/home/ada:/bin/bash\n",
        )
        .unwrap();
        fs::write(p.shadow(), "ada:!:1:::::::\n").unwrap();
        fs::write(p.group(), "wheel:x:10:ada\nada:x:1000:\n").unwrap();
        (t, p)
    }

    fn is_root() -> bool {
        // The home must be owned by the uid; only root can arrange that.
        rustix::process::getuid().is_root()
    }

    #[test]
    fn removes_a_pristine_home() {
        if !is_root() {
            return;
        }
        let (t, p) = root(None);
        let f = Fake::new(&p);
        remove(&p, &f, &acct(1000)).unwrap();
        assert_eq!(f.calls(), ["/usr/sbin/userdel -r -- ada"]);
        assert!(!t.path().join("home/ada").exists());
    }

    #[test]
    fn unknown_uid_zero_matches_by_name() {
        if !is_root() {
            return;
        }
        let (_t, p) = root(None);
        remove(&p, &Fake::new(&p), &creating()).unwrap();
    }

    #[test]
    fn unknown_uid_never_deletes_an_account_with_a_password() {
        // a name taken between the check and `useradd`, or an account the
        // user made by hand: it has a password, so it is not half-made
        let (t, p) = root(None);
        fs::write(p.shadow(), "ada:$y$j9T$abc$def:1:::::::\n").unwrap();
        let f = Fake::new(&p);
        assert_eq!(remove(&p, &f, &creating()), Err(Kept::NotOurs));
        assert!(f.calls().is_empty());
        assert!(t.path().join("home/ada").exists());
    }

    #[test]
    fn uid_zero_is_a_wildcard_only_while_creating() {
        let (_t, p) = root(None);
        let f = Fake::new(&p);
        assert_eq!(remove(&p, &f, &acct(0)), Err(Kept::NotOurs));
        assert!(f.calls().is_empty());
    }

    #[test]
    fn an_odd_name_in_the_state_never_reaches_userdel() {
        let (_t, p) = root(None);
        let f = Fake::new(&p);
        for name in ["-r", "--help", "Ada", "a:b", ""] {
            let a = Account {
                name: name.into(),
                ..creating()
            };
            assert_eq!(remove(&p, &f, &a), Err(Kept::NotOurs), "{name:?}");
        }
        assert!(f.calls().is_empty());
    }

    #[test]
    fn keeps_a_home_with_user_files() {
        if !is_root() {
            return;
        }
        let (t, p) = root(Some("thesis.odt"));
        let f = Fake::new(&p);
        assert_eq!(remove(&p, &f, &acct(1000)), Err(Kept::HomeHasData));
        assert!(f.calls().is_empty());
        assert!(t.path().join("home/ada/thesis.odt").exists());
    }

    #[test]
    fn keeps_another_uid() {
        let (_t, p) = root(None);
        let f = Fake::new(&p);
        assert_eq!(remove(&p, &f, &acct(1234)), Err(Kept::NotOurs));
        assert!(f.calls().is_empty());
    }

    #[test]
    fn never_touches_a_system_uid() {
        let (_t, p) = root(None);
        fs::write(p.passwd(), "ada:x:500:500::/home/ada:/bin/bash\n").unwrap();
        let f = Fake::new(&p);
        assert_eq!(remove(&p, &f, &creating()), Err(Kept::NotOurs));
        assert!(f.calls().is_empty());
    }

    #[test]
    fn absent_account_is_nothing_to_do() {
        let (_t, p) = root(None);
        fs::write(p.passwd(), "root:x:0:0::/root:/bin/bash\n").unwrap();
        let f = Fake::new(&p);
        remove(&p, &f, &acct(1000)).unwrap();
        assert!(f.calls().is_empty());
    }

    #[test]
    fn a_failing_userdel_is_reported() {
        if !is_root() {
            return;
        }
        let (_t, p) = root(None);
        let f = Fake::new(&p).failing("userdel");
        assert_eq!(remove(&p, &f, &acct(1000)), Err(Kept::UserdelFailed));
    }
}

//! Reading passwd, shadow and group from a root directory to find human
//! accounts and to verify the one the helper made.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// Lowest uid of a human account.
pub const UID_MIN: u32 = 1000;
/// Highest uid of a human account.
pub const UID_MAX: u32 = 60000;

/// A passwd entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Login name.
    pub name: String,
    /// Numeric uid.
    pub uid: u32,
    /// Primary group id.
    pub gid: u32,
    /// Home directory as written in passwd.
    pub home: String,
    /// Login shell as written in passwd.
    pub shell: String,
}

/// A human account: uid 1000..=60000, a login shell, a usable password hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Human {
    /// Login name.
    pub name: String,
    /// Numeric uid.
    pub uid: u32,
}

/// Why [`verify`] refused an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// No passwd entry with that name.
    NoPasswdEntry,
    /// The name exists with another uid.
    UidMismatch,
    /// No shadow entry, or its hash is empty or locked.
    NoUsableHash,
    /// Not a member of `wheel`.
    NotInWheel,
    /// The home path is empty or tries to leave the root.
    BadHomePath,
    /// The home directory does not exist.
    NoHome,
    /// The home directory is owned by someone else.
    HomeNotOwned,
    /// A file could not be read (kind of the I/O error).
    Io(io::ErrorKind),
}

impl VerifyError {
    /// Stable identifier for logs and the GUI.
    pub fn code(self) -> &'static str {
        match self {
            VerifyError::NoPasswdEntry => "verify-no-passwd-entry",
            VerifyError::UidMismatch => "verify-uid-mismatch",
            VerifyError::NoUsableHash => "verify-no-hash",
            VerifyError::NotInWheel => "verify-not-in-wheel",
            VerifyError::BadHomePath => "verify-bad-home-path",
            VerifyError::NoHome => "verify-no-home",
            VerifyError::HomeNotOwned => "verify-home-not-owned",
            VerifyError::Io(_) => "verify-io",
        }
    }
}

fn path(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}

/// Reads a file; a missing file is empty text.
fn read_or_empty(p: &Path) -> io::Result<String> {
    match fs::read(p) {
        Ok(b) => Ok(String::from_utf8_lossy(&b).into_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Parses passwd text; malformed lines are skipped.
pub fn parse_passwd(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 || f[0].is_empty() {
                return None;
            }
            Some(Entry {
                name: f[0].to_string(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                home: f[5].to_string(),
                shell: f[6].to_string(),
            })
        })
        .collect()
}

/// Parses shadow text into `name -> hash`.
pub fn parse_shadow(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split(':');
            let name = f.next()?;
            let hash = f.next()?;
            (!name.is_empty()).then(|| (name.to_string(), hash.to_string()))
        })
        .collect()
}

/// Parses group text into `(name, gid, members)`.
pub fn parse_group(text: &str) -> Vec<(String, u32, Vec<String>)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            let members = f[3]
                .split(',')
                .filter(|m| !m.is_empty())
                .map(String::from)
                .collect();
            Some((f[0].to_string(), f[2].parse().ok()?, members))
        })
        .collect()
}

/// True when a shadow hash can log in: not empty, not starting `!` or `*`.
pub fn usable_hash(hash: &str) -> bool {
    !hash.is_empty() && !hash.starts_with('!') && !hash.starts_with('*')
}

/// True when the shell lets a person log in: set, and not `nologin` or
/// `false` (any directory).
pub fn login_shell(shell: &str) -> bool {
    let base = shell.rsplit('/').next().unwrap_or("");
    !base.is_empty() && base != "nologin" && base != "false"
}

/// The human accounts under `root`: uid 1000..=60000, a login shell, and a
/// usable hash in shadow. A missing passwd or shadow means none.
///
/// # Errors
/// A read error on passwd or shadow (for example no permission): the caller
/// must not take that to mean "no accounts".
pub fn human_accounts(root: &Path) -> io::Result<Vec<Human>> {
    let passwd = parse_passwd(&read_or_empty(&path(root, "etc/passwd"))?);
    let shadow = parse_shadow(&read_or_empty(&path(root, "etc/shadow"))?);
    Ok(passwd
        .into_iter()
        .filter(|e| (UID_MIN..=UID_MAX).contains(&e.uid) && login_shell(&e.shell))
        .filter(|e| shadow.get(&e.name).is_some_and(|h| usable_hash(h)))
        .map(|e| Human {
            name: e.name,
            uid: e.uid,
        })
        .collect())
}

/// Checks the account the helper made: the passwd entry with that name and
/// uid, a usable shadow hash, `wheel` membership (supplementary or primary
/// group), and a home directory that exists and is owned by the uid.
///
/// # Errors
/// The first check that fails.
pub fn verify(root: &Path, name: &str, uid: u32) -> Result<(), VerifyError> {
    let io_err = |e: io::Error| VerifyError::Io(e.kind());
    let passwd = parse_passwd(&read_or_empty(&path(root, "etc/passwd")).map_err(io_err)?);
    let entry = passwd
        .iter()
        .find(|e| e.name == name)
        .ok_or(VerifyError::NoPasswdEntry)?;
    if entry.uid != uid {
        return Err(VerifyError::UidMismatch);
    }
    let shadow = parse_shadow(&read_or_empty(&path(root, "etc/shadow")).map_err(io_err)?);
    if !shadow.get(name).is_some_and(|h| usable_hash(h)) {
        return Err(VerifyError::NoUsableHash);
    }
    let groups = parse_group(&read_or_empty(&path(root, "etc/group")).map_err(io_err)?);
    let in_wheel = groups.iter().any(|(g, gid, members)| {
        g == "wheel" && (*gid == entry.gid || members.iter().any(|m| m == name))
    });
    if !in_wheel {
        return Err(VerifyError::NotInWheel);
    }
    let home = Path::new(&entry.home);
    if !home.is_absolute() || home.components().any(|c| c == Component::ParentDir) {
        return Err(VerifyError::BadHomePath);
    }
    let rel = home
        .strip_prefix("/")
        .map_err(|_| VerifyError::BadHomePath)?;
    let meta = match fs::symlink_metadata(root.join(rel)) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(VerifyError::NoHome),
        Err(e) => return Err(io_err(e)),
    };
    if !meta.is_dir() {
        return Err(VerifyError::NoHome);
    }
    if meta.uid() != uid {
        return Err(VerifyError::HomeNotOwned);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A fake root with the given file contents; homes are created for the
    /// names in `homes` (owned by the current user).
    fn root(passwd: &str, shadow: &str, group: &str, homes: &[&str]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc")).unwrap();
        fs::write(d.path().join("etc/passwd"), passwd).unwrap();
        fs::write(d.path().join("etc/shadow"), shadow).unwrap();
        fs::write(d.path().join("etc/group"), group).unwrap();
        for h in homes {
            fs::create_dir_all(d.path().join("home").join(h)).unwrap();
        }
        d
    }

    fn me() -> u32 {
        fs::metadata("/proc/self").unwrap().uid()
    }

    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
nobody:x:65534:65534:Kernel Overflow User:/:/sbin/nologin
telamon-setup:x:970:970:Telamon Setup:/run/telamon-setup:/bin/sh
ada:x:1000:1000:Ada:/home/ada:/bin/bash
nologin1:x:1001:1001::/home/nologin1:/usr/sbin/nologin
false1:x:1002:1002::/home/false1:/bin/false
locked:x:1003:1003::/home/locked:/bin/bash
star:x:1004:1004::/home/star:/bin/bash
empty:x:1005:1005::/home/empty:/bin/bash
nohash:x:1006:1006::/home/nohash:/bin/bash
big:x:60001:60001::/home/big:/bin/bash
edge:x:60000:60000::/home/edge:/bin/zsh
low:x:999:999::/home/low:/bin/bash
broken line
";
    const SHADOW: &str = "\
root:$y$x:19000:0:99999:7:::
ada:$y$j9T$abc:19000:0:99999:7:::
nologin1:$y$j9T$abc::::::
false1:$y$j9T$abc::::::
locked:!$y$j9T$abc::::::
star:*:19000::::::
empty::19000::::::
big:$y$j9T$abc::::::
edge:$y$j9T$abc::::::
low:$y$j9T$abc::::::
";

    #[test]
    fn human_accounts_filtering() {
        let d = root(PASSWD, SHADOW, "", &[]);
        let h = human_accounts(d.path()).unwrap();
        let names: Vec<_> = h.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["ada", "edge"]);
        assert_eq!(h[0].uid, 1000);
    }

    #[test]
    fn missing_files_mean_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(human_accounts(d.path()).unwrap(), vec![]);
        let d = root(PASSWD, SHADOW, "", &[]);
        fs::remove_file(d.path().join("etc/shadow")).unwrap();
        assert_eq!(human_accounts(d.path()).unwrap(), vec![]);
    }

    #[test]
    fn read_error_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc/passwd")).unwrap(); // a directory
        assert!(human_accounts(d.path()).is_err());
    }

    #[test]
    fn hash_and_shell_helpers() {
        assert!(usable_hash("$y$x") && usable_hash("x"));
        assert!(!usable_hash("") && !usable_hash("!") && !usable_hash("*") && !usable_hash("!!"));
        assert!(login_shell("/bin/bash") && login_shell("/usr/bin/fish"));
        assert!(
            !login_shell("")
                && !login_shell("/sbin/nologin")
                && !login_shell("/usr/bin/false")
                && !login_shell("nologin")
        );
    }

    fn good(uid: u32) -> tempfile::TempDir {
        root(
            &format!("ada:x:{uid}:{uid}::/home/ada:/bin/bash\n"),
            "ada:$y$j9T$abc:19000::::::\n",
            "wheel:x:10:ada\nada:x:1000:\n",
            &["ada"],
        )
    }

    #[test]
    fn verify_ok() {
        let uid = me();
        assert_eq!(verify(good(uid).path(), "ada", uid), Ok(()));
    }

    #[test]
    fn verify_each_failure() {
        let uid = me();
        let d = good(uid);
        assert_eq!(
            verify(d.path(), "bob", uid),
            Err(VerifyError::NoPasswdEntry)
        );
        assert_eq!(
            verify(d.path(), "ada", uid + 1),
            Err(VerifyError::UidMismatch)
        );

        let d = good(uid);
        fs::write(d.path().join("etc/shadow"), "ada:!:19000::::::\n").unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Err(VerifyError::NoUsableHash));
        fs::write(d.path().join("etc/shadow"), "").unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Err(VerifyError::NoUsableHash));

        let d = good(uid);
        fs::write(
            d.path().join("etc/group"),
            "wheel:x:10:bob\nada:x:1000:ada\n",
        )
        .unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Err(VerifyError::NotInWheel));

        let d = good(uid);
        fs::remove_dir(d.path().join("home/ada")).unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Err(VerifyError::NoHome));
        fs::write(d.path().join("home/ada"), "").unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Err(VerifyError::NoHome));
    }

    #[test]
    fn verify_wheel_by_primary_group() {
        let uid = me();
        let d = good(uid);
        fs::write(d.path().join("etc/group"), format!("wheel:x:{uid}:\n")).unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Ok(()));
    }

    #[test]
    fn verify_home_must_be_a_real_dir_owned_by_uid() {
        let uid = me();
        // Owned by someone else: ask for a uid that is not the owner's.
        let other = uid.wrapping_add(7);
        let d = root(
            &format!("ada:x:{other}:{other}::/home/ada:/bin/bash\n"),
            "ada:$y$x::::::\n",
            "wheel:x:10:ada\n",
            &["ada"],
        );
        assert_eq!(
            verify(d.path(), "ada", other),
            Err(VerifyError::HomeNotOwned)
        );
        // A symlinked home is not a directory for symlink_metadata.
        let d = good(uid);
        fs::remove_dir(d.path().join("home/ada")).unwrap();
        fs::create_dir(d.path().join("elsewhere")).unwrap();
        symlink(d.path().join("elsewhere"), d.path().join("home/ada")).unwrap();
        assert_eq!(verify(d.path(), "ada", uid), Err(VerifyError::NoHome));
    }

    #[test]
    fn verify_bad_home_paths() {
        let uid = me();
        for home in ["", "relative/x", "/home/../etc"] {
            let d = root(
                &format!("ada:x:{uid}:{uid}::{home}:/bin/bash\n"),
                "ada:$y$x::::::\n",
                "wheel:x:10:ada\n",
                &[],
            );
            assert_eq!(
                verify(d.path(), "ada", uid),
                Err(VerifyError::BadHomePath),
                "{home}"
            );
        }
    }

    #[test]
    fn codes() {
        assert_eq!(VerifyError::Io(io::ErrorKind::Other).code(), "verify-io");
        assert_eq!(VerifyError::NotInWheel.code(), "verify-not-in-wheel");
    }
}

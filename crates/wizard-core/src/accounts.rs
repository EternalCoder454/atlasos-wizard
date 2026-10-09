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
    /// The home directory can be written by its group or by everyone.
    HomeWritableByOthers,
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
            VerifyError::HomeWritableByOthers => "verify-home-writable",
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

/// The path of a passwd home (`/home/ada`) below `root`, refused when it is
/// empty, relative or climbs with `..`.
fn home_path(root: &Path, home: &str) -> Result<PathBuf, VerifyError> {
    let home = Path::new(home);
    if !home.is_absolute() || home.components().any(|c| c == Component::ParentDir) {
        return Err(VerifyError::BadHomePath);
    }
    let rel = home
        .strip_prefix("/")
        .map_err(|_| VerifyError::BadHomePath)?;
    Ok(root.join(rel))
}

/// The metadata of the home itself (a symlink is not followed), which must
/// be a directory.
fn home_meta(root: &Path, home: &str) -> Result<fs::Metadata, VerifyError> {
    let meta = match fs::symlink_metadata(home_path(root, home)?) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(VerifyError::NoHome),
        Err(e) => return Err(VerifyError::Io(e.kind())),
    };
    if !meta.is_dir() {
        return Err(VerifyError::NoHome);
    }
    Ok(meta)
}

/// Takes group and other access away from the home of the account `name`
/// (uid `uid`): `chmod go-rwx`, so it is at most 0700 whatever `HOME_MODE`
/// or `UMASK` the system gave. Runs between the account's creation and
/// [`verify`], before any session of the account can exist. Returns whether
/// the mode changed.
///
/// The home must be a real directory (never a symlink) owned by `uid`; the
/// change goes through a file descriptor checked to be that same directory.
///
/// # Errors
/// What [`verify`] would say about the passwd entry and the home, or the I/O
/// error of the change.
pub fn secure_home(root: &Path, name: &str, uid: u32) -> Result<bool, VerifyError> {
    use std::os::unix::fs::PermissionsExt;
    let io_err = |e: io::Error| VerifyError::Io(e.kind());
    let passwd = parse_passwd(&read_or_empty(&path(root, "etc/passwd")).map_err(io_err)?);
    let entry = passwd
        .iter()
        .find(|e| e.name == name)
        .ok_or(VerifyError::NoPasswdEntry)?;
    if entry.uid != uid {
        return Err(VerifyError::UidMismatch);
    }
    let before = home_meta(root, &entry.home)?;
    if before.uid() != uid {
        return Err(VerifyError::HomeNotOwned);
    }
    let dir = fs::File::open(home_path(root, &entry.home)?).map_err(io_err)?;
    let now = dir.metadata().map_err(io_err)?;
    if !now.is_dir() || now.dev() != before.dev() || now.ino() != before.ino() {
        // swapped between the two looks
        return Err(VerifyError::NoHome);
    }
    let mode = now.mode() & 0o7777;
    if mode & 0o077 == 0 {
        return Ok(false);
    }
    dir.set_permissions(fs::Permissions::from_mode(mode & !0o077))
        .map_err(io_err)?;
    Ok(true)
}

/// Checks the account the helper made: the passwd entry with that name and
/// uid, a usable shadow hash, `wheel` membership (supplementary or primary
/// group), and a home directory that exists, is owned by the uid and is not
/// writable by its group or by everyone.
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
    let meta = home_meta(root, &entry.home)?;
    if meta.uid() != uid {
        return Err(VerifyError::HomeNotOwned);
    }
    // Fedora's HOME_MODE is 0700 and `secure_home` makes it so; whatever
    // else the system gave, a home that the group or everyone can write to
    // lets another account swap files the new user will run.
    if meta.mode() & 0o022 != 0 {
        return Err(VerifyError::HomeWritableByOthers);
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
    fn verify_refuses_a_home_others_can_write() {
        use std::os::unix::fs::PermissionsExt;
        let uid = me();
        for (mode, ok) in [
            (0o700, true),
            (0o750, true),
            (0o755, true),
            (0o770, false),
            (0o775, false),
            (0o757, false),
            (0o777, false),
        ] {
            let d = good(uid);
            fs::set_permissions(d.path().join("home/ada"), fs::Permissions::from_mode(mode))
                .unwrap();
            let want = if ok {
                Ok(())
            } else {
                Err(VerifyError::HomeWritableByOthers)
            };
            assert_eq!(verify(d.path(), "ada", uid), want, "{mode:o}");
        }
        assert_eq!(
            VerifyError::HomeWritableByOthers.code(),
            "verify-home-writable"
        );
    }

    #[test]
    fn secure_home_takes_group_and_other_access_away() {
        use std::os::unix::fs::PermissionsExt;
        let uid = me();
        let mode_of = |d: &tempfile::TempDir| {
            fs::metadata(d.path().join("home/ada"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        for (before, after, changed) in [
            (0o755, 0o700, true),
            (0o777, 0o700, true),
            (0o750, 0o700, true),
            (0o705, 0o700, true),
            (0o700, 0o700, false),
            (0o500, 0o500, false),
            (0o2755, 0o2700, true),
        ] {
            let d = good(uid);
            fs::set_permissions(
                d.path().join("home/ada"),
                fs::Permissions::from_mode(before),
            )
            .unwrap();
            assert_eq!(secure_home(d.path(), "ada", uid), Ok(changed), "{before:o}");
            assert_eq!(mode_of(&d), after, "{before:o}");
        }
    }

    #[test]
    fn secure_home_refuses_what_is_not_the_accounts_own_directory() {
        use std::os::unix::fs::PermissionsExt;
        let uid = me();
        // no such account, another uid
        let d = good(uid);
        assert_eq!(
            secure_home(d.path(), "bob", uid),
            Err(VerifyError::NoPasswdEntry)
        );
        assert_eq!(
            secure_home(d.path(), "ada", uid + 1),
            Err(VerifyError::UidMismatch)
        );
        // a symlink as the home is never followed, whatever it points at
        let d = good(uid);
        let target = d.path().join("elsewhere");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir(d.path().join("home/ada")).unwrap();
        symlink(&target, d.path().join("home/ada")).unwrap();
        assert_eq!(secure_home(d.path(), "ada", uid), Err(VerifyError::NoHome));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755,
            "the link's target is left alone"
        );
        // a missing home, a file, an owner who is not the account
        let d = good(uid);
        fs::remove_dir(d.path().join("home/ada")).unwrap();
        assert_eq!(secure_home(d.path(), "ada", uid), Err(VerifyError::NoHome));
        let other = uid.wrapping_add(7);
        let d = root(
            &format!("ada:x:{other}:{other}::/home/ada:/bin/bash\n"),
            "ada:$y$x::::::\n",
            "wheel:x:10:ada\n",
            &["ada"],
        );
        fs::set_permissions(d.path().join("home/ada"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            secure_home(d.path(), "ada", other),
            Err(VerifyError::HomeNotOwned)
        );
        assert_eq!(
            fs::metadata(d.path().join("home/ada"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "a home of someone else is not touched"
        );
        // a home path that leaves the root
        let d = root(
            &format!("ada:x:{uid}:{uid}::/home/../etc:/bin/bash\n"),
            "ada:$y$x::::::\n",
            "wheel:x:10:ada\n",
            &[],
        );
        assert_eq!(
            secure_home(d.path(), "ada", uid),
            Err(VerifyError::BadHomePath)
        );
    }

    #[test]
    fn codes() {
        assert_eq!(VerifyError::Io(io::ErrorKind::Other).code(), "verify-io");
        assert_eq!(VerifyError::NotInWheel.code(), "verify-not-in-wheel");
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;

    fn line() -> impl Strategy<Value = String> {
        prop_oneof![
            2 => any::<String>(),
            3 => ("[a-z+@-][a-z0-9_-]{0,6}", prop::sample::select(vec![
                    "x", "!", "*", "", "$y$j9T$a$b", "!$y$j9T", "!!",
                ]), "[0-9a-z-]{0,12}", "[0-9a-z-]{0,12}", prop::sample::select(vec![
                    "", "/home/a", "/", "../x", "rel", "/home/a:b", "\u{202e}",
                ]), prop::sample::select(vec![
                    "/bin/bash", "/usr/sbin/nologin", "/bin/false", "", "nologin", "/",
                ]))
                .prop_map(|(n, p, a, b, h, sh)| format!("{n}:{p}:{a}:{b}::{h}:{sh}")),
        ]
    }

    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(line(), 0..12).prop_map(|v| v.join("\n"))
    }

    proptest! {
        /// The three readers never panic on any text, and what they return
        /// has a name (no `:`, not empty) and the numbers they promise.
        #[test]
        fn readers_never_panic_and_keep_their_shape(t in text(), raw in any::<String>()) {
            for text in [&t, &raw] {
                for e in parse_passwd(text) {
                    prop_assert!(!e.name.is_empty() && !e.name.contains(':'));
                    prop_assert!(!e.home.contains(':') && !e.shell.contains(':'));
                }
                for (name, hash) in parse_shadow(text) {
                    prop_assert!(!name.is_empty() && !name.contains(':'));
                    prop_assert!(!hash.contains(':'));
                }
                for (name, _, members) in parse_group(text) {
                    prop_assert!(!name.contains(':'));
                    prop_assert!(members.iter().all(|m| !m.is_empty() && !m.contains(',')));
                }
            }
        }

        /// A shell or a hash of any text is classified without panicking; a
        /// hash that starts with `!` or `*`, or is empty, can never log in,
        /// and a shell that ends in `nologin` or `false` is never a login
        /// shell, whatever precedes it.
        #[test]
        fn locked_means_locked(s in any::<String>(), dir in "[a-z/]{0,12}") {
            if s.is_empty() || s.starts_with('!') || s.starts_with('*') {
                prop_assert!(!usable_hash(&s));
            }
            let (nologin, falsy) = (format!("{}/nologin", dir), format!("{}/false", dir));
            prop_assert!(!login_shell(&nologin) && !login_shell(&falsy));
            prop_assert!(!login_shell(""));
        }
    }

    proptest! {
        /// `human_accounts` returns only uids 1000..=60000 with a login
        /// shell and a usable hash, whatever the files hold.
        #[test]
        fn humans_are_only_real_people(p in text(), s in text()) {
            static RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if !crate::budget::within(&RUNS, 100) {
                return Ok(());
            }
            let d = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(d.path().join("etc")).unwrap();
            std::fs::write(d.path().join("etc/passwd"), &p).unwrap();
            std::fs::write(d.path().join("etc/shadow"), &s).unwrap();
            let humans = human_accounts(d.path()).unwrap();
            let passwd = parse_passwd(&p);
            let shadow = parse_shadow(&s);
            for h in humans {
                prop_assert!((UID_MIN..=UID_MAX).contains(&h.uid));
                prop_assert!(passwd.iter().any(|e| e.name == h.name && e.uid == h.uid
                    && login_shell(&e.shell)));
                prop_assert!(shadow.get(&h.name).is_some_and(|x| usable_hash(x)));
            }
        }
    }

    /// The property above almost never draws a matching passwd and shadow; this
    /// is the same rule on a table of hits and near-misses.
    #[test]
    fn humans_are_listed_by_uid_shell_and_hash() {
        let hash = "$y$j9T$salt$hash";
        let passwd = "\
ada:x:1000:1000::/home/ada:/bin/bash
low:x:999:999::/home/low:/bin/bash
top:x:60000:60000::/home/top:/bin/zsh
over:x:60001:60001::/home/over:/bin/bash
svc:x:1001:1001::/home/svc:/usr/sbin/nologin
nohash:x:1002:1002::/home/nohash:/bin/bash
locked:x:1003:1003::/home/locked:/bin/bash
empty:x:1004:1004::/home/empty:/bin/bash
";
        let shadow = format!(
            "ada:{hash}:1::::::\nlow:{hash}:1::::::\ntop:{hash}:1::::::\nover:{hash}:1::::::\n\
svc:{hash}:1::::::\nlocked:!{hash}:1::::::\nempty::1::::::\n"
        );
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("etc")).unwrap();
        std::fs::write(d.path().join("etc/passwd"), passwd).unwrap();
        std::fs::write(d.path().join("etc/shadow"), shadow).unwrap();
        let mut names: Vec<String> = human_accounts(d.path())
            .unwrap()
            .into_iter()
            .map(|h| h.name)
            .collect();
        names.sort();
        assert_eq!(names, ["ada", "top"]);
    }

    #[test]
    fn nasty_files() {
        let huge = "ada:x:1000:1000::/home/ada:/bin/bash\n".repeat(300_000);
        assert_eq!(parse_passwd(&huge).len(), 300_000);
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("etc")).unwrap();
        // bytes that are not UTF-8 are read lossily, never as an error
        std::fs::write(
            d.path().join("etc/passwd"),
            b"\xff\xfe:x:1000:1000::/h:/bin/sh\n",
        )
        .unwrap();
        assert!(human_accounts(d.path()).unwrap().is_empty());
        // the same name twice: both are listed, and `verify` takes the first
        let two = "ada:x:1000:1000::/home/ada:/bin/sh\nada:x:0:0::/root:/bin/sh\n";
        assert_eq!(parse_passwd(two).len(), 2);
    }
}

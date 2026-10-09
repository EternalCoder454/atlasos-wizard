//! Every filesystem path and fixed name the helper uses, behind one value, so
//! tests can point the whole helper at a temporary root.
//!
//! In a release build the root is `/` and the setup user is `telamon-setup`; no
//! environment variable is read. Only a build with the `test-root` feature
//! (tests) reads `TELAMON_WIZARD_TEST_ROOT`, `TELAMON_WIZARD_TEST_SETUP_USER` and
//! `TELAMON_WIZARD_TEST_IDLE_MS`.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The setup user's name.
pub const SETUP_USER: &str = "telamon-setup";

/// True when this build can be redirected by `TELAMON_WIZARD_TEST_*`.
pub const TEST_ROOT_ENABLED: bool = cfg!(feature = "test-root");

/// Where the setup autologin drop-in lives (relative to the root).
pub const SETUP_AUTOLOGIN: &str = "etc/plasmalogin.conf.d/99-telamon-wizard.conf";
/// The new account's autologin drop-in (relative to the root).
pub const USER_AUTOLOGIN: &str = "etc/plasmalogin.conf.d/50-telamon-autologin.conf";
/// Atlas Wizard's setup autologin drop-in (relative to the root): removed
/// wherever it is found.
pub const LEGACY_SETUP_AUTOLOGIN: &str = "etc/plasmalogin.conf.d/99-atlas-wizard.conf";
/// The state file (relative to the root).
pub const STATE_FILE: &str = "var/lib/telamon-wizard/state.json";
/// The distribution's default `kdeglobals` (relative to the root).
pub const SYSTEM_KDEGLOBALS: &str = "etc/xdg/kdeglobals";
/// The high contrast colour scheme (relative to the root).
pub const HC_SCHEME_FILE: &str = "usr/share/color-schemes/AtlasOSHighContrast.colors";
/// The skeleton directory (relative to the root).
pub const SKEL: &str = "etc/skel";

/// Root directory and fixed names. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
    setup_user: String,
    idle: Duration,
}

/// Idle time before the helper exits.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

impl Paths {
    /// The real system: root `/`, the setup user `telamon-setup`.
    pub fn system() -> Paths {
        Paths {
            root: PathBuf::from("/"),
            setup_user: SETUP_USER.to_string(),
            idle: IDLE_TIMEOUT,
        }
    }

    /// A given root (tests); the setup user and idle time are the defaults.
    pub fn with_root(root: impl Into<PathBuf>) -> Paths {
        Paths {
            root: root.into(),
            ..Paths::system()
        }
    }

    /// The process's paths: [`Paths::system`], or, in a `test-root` build, as
    /// the `TELAMON_WIZARD_TEST_*` variables say.
    pub fn from_env() -> Paths {
        Paths::from_lookup(|k| std::env::var(k).ok())
    }

    /// [`Paths::from_env`] with the variable lookup passed in. Without the
    /// `test-root` feature `lookup` is never called.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Paths {
        let mut p = Paths::system();
        if TEST_ROOT_ENABLED {
            if let Some(r) =
                lookup("TELAMON_WIZARD_TEST_ROOT").filter(|r| Path::new(r).is_absolute())
            {
                p.root = PathBuf::from(r);
            }
            if let Some(u) = lookup("TELAMON_WIZARD_TEST_SETUP_USER").filter(|u| !u.is_empty()) {
                p.setup_user = u;
            }
            if let Some(ms) = lookup("TELAMON_WIZARD_TEST_IDLE_MS").and_then(|v| v.parse().ok()) {
                p.idle = Duration::from_millis(ms);
            }
        }
        p
    }

    /// The root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The setup user's name.
    pub fn setup_user(&self) -> &str {
        &self.setup_user
    }

    /// How long without a call before the helper exits.
    pub fn idle_timeout(&self) -> Duration {
        self.idle
    }

    /// `abs` (an absolute path such as `/usr/bin/chage`, or a path already
    /// relative) under the root.
    pub fn join(&self, abs: impl AsRef<Path>) -> PathBuf {
        let p = abs.as_ref();
        self.root.join(p.strip_prefix("/").unwrap_or(p))
    }

    /// The state file.
    pub fn state(&self) -> PathBuf {
        self.join(STATE_FILE)
    }

    /// `/etc/passwd`.
    pub fn passwd(&self) -> PathBuf {
        self.join("etc/passwd")
    }

    /// `/etc/group`.
    pub fn group(&self) -> PathBuf {
        self.join("etc/group")
    }

    /// The setup autologin drop-in.
    pub fn setup_autologin(&self) -> PathBuf {
        self.join(SETUP_AUTOLOGIN)
    }

    /// Atlas Wizard's setup autologin drop-in.
    pub fn legacy_setup_autologin(&self) -> PathBuf {
        self.join(LEGACY_SETUP_AUTOLOGIN)
    }

    /// The new account's autologin drop-in.
    pub fn user_autologin(&self) -> PathBuf {
        self.join(USER_AUTOLOGIN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_strips_the_leading_slash() {
        let p = Paths::with_root("/tmp/r");
        assert_eq!(p.join("/usr/bin/chage"), Path::new("/tmp/r/usr/bin/chage"));
        assert_eq!(p.join("etc/passwd"), Path::new("/tmp/r/etc/passwd"));
        assert_eq!(Paths::system().join("/etc/x"), Path::new("/etc/x"));
    }

    #[test]
    fn env_is_read_only_in_test_root_builds() {
        let p = Paths::from_lookup(|k| match k {
            "TELAMON_WIZARD_TEST_ROOT" => Some("/tmp/elsewhere".into()),
            "TELAMON_WIZARD_TEST_SETUP_USER" => Some("someone".into()),
            "TELAMON_WIZARD_TEST_IDLE_MS" => Some("250".into()),
            _ => None,
        });
        if TEST_ROOT_ENABLED {
            assert_eq!(p.root(), Path::new("/tmp/elsewhere"));
            assert_eq!(p.setup_user(), "someone");
            assert_eq!(p.idle_timeout(), Duration::from_millis(250));
        } else {
            assert_eq!(p.root(), Path::new("/"));
            assert_eq!(p.setup_user(), SETUP_USER);
            assert_eq!(p.idle_timeout(), IDLE_TIMEOUT);
        }
    }

    #[test]
    fn a_relative_test_root_is_ignored() {
        let p = Paths::from_lookup(|k| (k == "TELAMON_WIZARD_TEST_ROOT").then(|| "rel/dir".into()));
        assert_eq!(p.root(), Path::new("/"));
    }

    /// The release build must not contain the redirect: `test-root` is not a
    /// default feature, and only the crate's own dev-dependency turns it on.
    #[test]
    fn test_root_is_not_a_default_feature() {
        let manifest = include_str!("../Cargo.toml");
        let default = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("default"))
            .expect("a default feature line");
        assert!(!default.contains("test-root"), "{default}");
        let dev = manifest.split("[dev-dependencies]").nth(1).unwrap();
        let normal = manifest.split("[dev-dependencies]").next().unwrap();
        assert!(dev.contains("features = [\"test-root\"]"));
        assert_eq!(normal.matches("features = [\"test-root\"]").count(), 0);
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// A path with no `..` stays under the root, absolute or not, and
        /// the fixed names are all under it too.
        #[test]
        fn join_never_leaves_the_root(
            parts in prop::collection::vec(
                prop::sample::select(vec!["etc", "passwd", "", ".", "x y", "-a", "\u{202e}", "a\nb"]), 0..6),
            absolute in any::<bool>(),
        ) {
            let p = Paths::with_root("/tmp/r");
            let rel = parts.join("/");
            let arg = if absolute { format!("/{rel}") } else { rel };
            let joined = p.join(&arg);
            prop_assert!(joined.starts_with("/tmp/r"), "{joined:?}");
            prop_assert!(joined.components().all(|c| c != std::path::Component::ParentDir));
        }
    }

    #[test]
    fn every_fixed_path_is_under_the_root() {
        let p = Paths::with_root("/tmp/r");
        for f in [
            p.state(),
            p.passwd(),
            p.group(),
            p.setup_autologin(),
            p.legacy_setup_autologin(),
            p.user_autologin(),
        ] {
            assert!(f.starts_with("/tmp/r"), "{f:?}");
        }
        // and the release paths are the system's
        let s = Paths::system();
        assert_eq!(s.passwd(), Path::new("/etc/passwd"));
        assert_eq!(s.state(), Path::new("/var/lib/telamon-wizard/state.json"));
        assert_eq!(
            s.user_autologin(),
            Path::new("/etc/plasmalogin.conf.d/50-telamon-autologin.conf")
        );
    }
}

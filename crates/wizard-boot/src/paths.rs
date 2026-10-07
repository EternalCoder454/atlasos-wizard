//! Every path the program touches, under one root: `/` on a real system.
//! A release build can only ever use `/`; a build with the `test-root`
//! feature (tests only) takes the root from `TELAMON_WIZARD_TEST_ROOT`.

#[cfg(any(test, feature = "test-root"))]
use std::path::Path;
use std::path::PathBuf;
use wizard_core::state;

/// The setup user's name.
pub const SETUP_USER: &str = "telamon-setup";
/// The setup user of Atlas Wizard (0.1.x). A machine set up by it still has the
/// account (locked); a machine part way through setup may still have its
/// autologin. The cleanup deals with both.
pub const LEGACY_SETUP_USER: &str = "atlas-setup";

/// Paths under a root directory.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// The paths of the running system.
    pub fn system() -> Paths {
        #[cfg(feature = "test-root")]
        if let Some(root) = std::env::var_os("TELAMON_WIZARD_TEST_ROOT") {
            return Paths {
                root: PathBuf::from(root),
            };
        }
        Paths {
            root: PathBuf::from("/"),
        }
    }

    /// Paths under a given root (tests).
    #[cfg(any(test, feature = "test-root"))]
    pub fn with_root(root: &Path) -> Paths {
        Paths {
            root: root.to_path_buf(),
        }
    }

    /// The root directory.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// The setup autologin drop-in.
    pub fn dropin(&self) -> PathBuf {
        self.join("etc/plasmalogin.conf.d/99-telamon-wizard.conf")
    }

    /// Atlas Wizard's setup autologin, `99-atlas-wizard.conf`: it names a user
    /// and a session that no longer exist, so it is removed wherever it is
    /// found.
    pub fn legacy_dropin(&self) -> PathBuf {
        self.join("etc/plasmalogin.conf.d/99-atlas-wizard.conf")
    }

    /// `/var/lib/telamon-wizard/state.json`.
    pub fn state_file(&self) -> PathBuf {
        self.join(state::DEFAULT_PATH.trim_start_matches('/'))
    }

    /// `/etc/passwd`.
    pub fn passwd(&self) -> PathBuf {
        self.join("etc/passwd")
    }

    /// `/etc/shadow`.
    pub fn shadow(&self) -> PathBuf {
        self.join("etc/shadow")
    }

    /// `/etc/group`.
    pub fn group(&self) -> PathBuf {
        self.join("etc/group")
    }

    /// `/etc/skel`.
    pub fn skel(&self) -> PathBuf {
        self.join("etc/skel")
    }

    /// The setup user's home.
    pub fn setup_home(&self) -> PathBuf {
        self.join("run/telamon-setup")
    }

    /// logind's per-user state file, present while the user has a session.
    pub fn logind_user(&self, uid: u32) -> PathBuf {
        self.join(&format!("run/systemd/users/{uid}"))
    }

    /// The kernel command line.
    pub fn cmdline(&self) -> PathBuf {
        self.join("proc/cmdline")
    }

    /// A path inside the root from an absolute path as passwd writes it.
    pub fn under_root(&self, abs: &str) -> PathBuf {
        self.root.join(abs.trim_start_matches('/'))
    }

    /// Where the fake command runner logs (tests only).
    #[cfg(any(test, feature = "test-root"))]
    pub fn command_log(&self) -> PathBuf {
        self.join("commands.log")
    }
}

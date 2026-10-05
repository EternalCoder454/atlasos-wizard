//! What the helper talks to besides the filesystem: AccountsService, systemd,
//! the two lock commands and the child that writes the account's settings.
//! Each is a trait, so the logic in `core` is tested with fakes; the real
//! implementations are here.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use zbus::zvariant::OwnedObjectPath;

use crate::error::HelperError;
use crate::paths::Paths;

/// A boxed future, so the traits can be used as `dyn`.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A backend call that failed. `detail` never holds a password or a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    /// What went wrong, for the journal.
    pub detail: String,
}

impl BackendError {
    /// A new error.
    pub fn new(detail: impl Into<String>) -> Self {
        BackendError {
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

/// `org.freedesktop.Accounts`.
pub trait AccountsApi: Send + Sync {
    /// `CreateUser(name, full_name, 1)` (an administrator). Returns the new
    /// user's object path, `/org/freedesktop/Accounts/User<uid>`.
    fn create_user<'a>(
        &'a self,
        name: &'a str,
        full_name: &'a str,
    ) -> BoxFut<'a, Result<String, BackendError>>;

    /// `SetPassword(hash, "")` on the user's object. `hash` is a secret.
    fn set_password<'a>(
        &'a self,
        user_path: &'a str,
        hash: &'a str,
    ) -> BoxFut<'a, Result<(), BackendError>>;

    /// `DeleteUser(uid, true)`: the account and its home.
    fn delete_user(&self, uid: u32) -> BoxFut<'_, Result<(), BackendError>>;
}

/// `org.freedesktop.systemd1.Manager`, two calls on fixed units.
pub trait SystemdApi: Send + Sync {
    /// `RestartUnit(unit, "replace")`.
    fn restart_unit<'a>(&'a self, unit: &'a str) -> BoxFut<'a, Result<(), BackendError>>;
    /// `StartUnit(unit, "replace")`.
    fn start_unit<'a>(&'a self, unit: &'a str) -> BoxFut<'a, Result<(), BackendError>>;
}

/// Runs a fixed program with a fixed argv (never through a shell).
pub trait Runner: Send + Sync {
    /// Runs `program` (an absolute path such as `/usr/bin/chage`), waits, and
    /// returns its exit status: `Ok(())` for 0.
    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFut<'a, Result<(), BackendError>>;
}

/// What the child that writes the account's settings is given.
#[derive(Debug, Clone)]
pub struct UserSettingsRequest {
    /// The account's user name.
    pub name: String,
    /// Its uid.
    pub uid: u32,
    /// Its primary gid.
    pub gid: u32,
    /// Its home directory (already under the helper's root).
    pub home: PathBuf,
    /// The validated choices as JSON, sent on the child's stdin.
    pub json: Vec<u8>,
}

/// Writes the account's settings as that account.
pub trait SettingsApplier: Send + Sync {
    /// Runs the settings child for `req`.
    fn apply<'a>(&'a self, req: &'a UserSettingsRequest) -> BoxFut<'a, Result<(), HelperError>>;
}

/// Timeout of one AccountsService call (the whole `CreateAccount` has 120 s).
pub const ACCOUNTS_CALL_TIMEOUT: Duration = Duration::from_secs(110);
/// Timeout of one systemd call.
pub const SYSTEMD_CALL_TIMEOUT: Duration = Duration::from_secs(25);
/// Timeout of one lock command.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(20);
/// Timeout of the settings child.
pub const CHILD_TIMEOUT: Duration = Duration::from_secs(60);

/// Describes a zbus error for the journal; the remote message is left out
/// when `with_message` is false (calls that carry a secret).
fn describe(e: &zbus::Error, with_message: bool) -> String {
    match e {
        zbus::Error::MethodError(name, msg, _) => {
            if with_message {
                format!("{}: {}", name.as_str(), msg.as_deref().unwrap_or(""))
            } else {
                name.as_str().to_string()
            }
        }
        zbus::Error::InputOutput(_) => "connection error".to_string(),
        other if with_message => other.to_string(),
        _ => "bus error".to_string(),
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.Accounts",
    default_service = "org.freedesktop.Accounts",
    default_path = "/org/freedesktop/Accounts",
    gen_blocking = false
)]
trait AccountsManager {
    fn create_user(
        &self,
        name: &str,
        fullname: &str,
        account_type: i32,
    ) -> zbus::Result<OwnedObjectPath>;
    fn delete_user(&self, id: i64, remove_files: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Accounts.User",
    default_service = "org.freedesktop.Accounts",
    gen_blocking = false
)]
trait AccountsUser {
    fn set_password(&self, password: &str, hint: &str) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1",
    gen_blocking = false
)]
trait SystemdManager {
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
}

/// AccountsService and systemd over the system bus.
#[derive(Clone)]
pub struct BusBackend {
    conn: zbus::Connection,
}

impl BusBackend {
    /// Uses `conn` (the helper's own connection).
    pub fn new(conn: zbus::Connection) -> Self {
        BusBackend { conn }
    }
}

impl AccountsApi for BusBackend {
    fn create_user<'a>(
        &'a self,
        name: &'a str,
        full_name: &'a str,
    ) -> BoxFut<'a, Result<String, BackendError>> {
        Box::pin(async move {
            let proxy = AccountsManagerProxy::builder(&self.conn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(|e| BackendError::new(describe(&e, true)))?;
            let path =
                tokio::time::timeout(ACCOUNTS_CALL_TIMEOUT, proxy.create_user(name, full_name, 1))
                    .await
                    .map_err(|_| BackendError::new("AccountsService CreateUser timed out"))?
                    .map_err(|e| BackendError::new(describe(&e, true)))?;
            Ok(path.as_str().to_string())
        })
    }

    fn set_password<'a>(
        &'a self,
        user_path: &'a str,
        hash: &'a str,
    ) -> BoxFut<'a, Result<(), BackendError>> {
        Box::pin(async move {
            // the call carries the hash: its errors are logged without text
            let quiet = |e: zbus::Error| BackendError::new(describe(&e, false));
            let proxy = AccountsUserProxy::builder(&self.conn)
                .path(user_path.to_string())
                .map_err(quiet)?
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(quiet)?;
            tokio::time::timeout(ACCOUNTS_CALL_TIMEOUT, proxy.set_password(hash, ""))
                .await
                .map_err(|_| BackendError::new("AccountsService SetPassword timed out"))?
                .map_err(quiet)
        })
    }

    fn delete_user(&self, uid: u32) -> BoxFut<'_, Result<(), BackendError>> {
        Box::pin(async move {
            let proxy = AccountsManagerProxy::builder(&self.conn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(|e| BackendError::new(describe(&e, true)))?;
            tokio::time::timeout(
                ACCOUNTS_CALL_TIMEOUT,
                proxy.delete_user(i64::from(uid), true),
            )
            .await
            .map_err(|_| BackendError::new("AccountsService DeleteUser timed out"))?
            .map_err(|e| BackendError::new(describe(&e, true)))
        })
    }
}

impl SystemdApi for BusBackend {
    fn restart_unit<'a>(&'a self, unit: &'a str) -> BoxFut<'a, Result<(), BackendError>> {
        Box::pin(async move {
            let proxy = SystemdManagerProxy::builder(&self.conn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(|e| BackendError::new(describe(&e, true)))?;
            tokio::time::timeout(SYSTEMD_CALL_TIMEOUT, proxy.restart_unit(unit, "replace"))
                .await
                .map_err(|_| BackendError::new("systemd RestartUnit timed out"))?
                .map(|_| ())
                .map_err(|e| BackendError::new(describe(&e, true)))
        })
    }

    fn start_unit<'a>(&'a self, unit: &'a str) -> BoxFut<'a, Result<(), BackendError>> {
        Box::pin(async move {
            let proxy = SystemdManagerProxy::builder(&self.conn)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
                .map_err(|e| BackendError::new(describe(&e, true)))?;
            tokio::time::timeout(SYSTEMD_CALL_TIMEOUT, proxy.start_unit(unit, "replace"))
                .await
                .map_err(|_| BackendError::new("systemd StartUnit timed out"))?
                .map(|_| ())
                .map_err(|e| BackendError::new(describe(&e, true)))
        })
    }
}

/// Runs the programs for real (under the helper's root in tests).
pub struct SystemRunner {
    paths: Paths,
}

impl SystemRunner {
    /// Programs are looked up under `paths`' root.
    pub fn new(paths: Paths) -> Self {
        SystemRunner { paths }
    }
}

impl Runner for SystemRunner {
    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFut<'a, Result<(), BackendError>> {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(self.paths.join(program));
            cmd.args(args)
                .env_clear()
                .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let status = tokio::time::timeout(RUN_TIMEOUT, async {
                let mut child = cmd.spawn()?;
                child.wait().await
            })
            .await
            .map_err(|_| BackendError::new(format!("{program} timed out")))?
            .map_err(|e| BackendError::new(format!("{program}: {}", e.kind())))?;
            if status.success() {
                Ok(())
            } else {
                Err(BackendError::new(format!("{program} exited with {status}")))
            }
        })
    }
}

/// Runs `/proc/self/exe apply-user-settings` as the account.
pub struct ChildApplier {
    paths: Paths,
}

impl ChildApplier {
    /// The child inherits `paths`' root only in `test-root` builds.
    pub fn new(paths: Paths) -> Self {
        ChildApplier { paths }
    }
}

/// The child's whole environment: nothing but these.
pub fn child_env(req: &UserSettingsRequest, _paths: &Paths) -> Vec<(String, String)> {
    let home = req.home.to_string_lossy().into_owned();
    #[allow(unused_mut)]
    let mut env = vec![
        ("HOME".to_string(), home.clone()),
        ("USER".to_string(), req.name.clone()),
        ("LOGNAME".to_string(), req.name.clone()),
        ("PATH".to_string(), "/usr/bin:/bin".to_string()),
        ("XDG_CONFIG_HOME".to_string(), format!("{home}/.config")),
        ("XDG_DATA_HOME".to_string(), format!("{home}/.local/share")),
        ("XDG_STATE_HOME".to_string(), format!("{home}/.local/state")),
        ("QT_QPA_PLATFORM".to_string(), "offscreen".to_string()),
    ];
    #[cfg(feature = "test-root")]
    env.push((
        "ATLAS_WIZARD_TEST_ROOT".to_string(),
        _paths.root().to_string_lossy().into_owned(),
    ));
    env
}

impl SettingsApplier for ChildApplier {
    fn apply<'a>(&'a self, req: &'a UserSettingsRequest) -> BoxFut<'a, Result<(), HelperError>> {
        Box::pin(async move {
            let fail = |code: &str, text: &str| HelperError::failed(code, text);
            let mut cmd = tokio::process::Command::new("/proc/self/exe");
            // std drops the supplementary groups when it changes uid from root
            cmd.arg("apply-user-settings")
                .env_clear()
                .envs(child_env(req, &self.paths))
                .current_dir(&req.home)
                .uid(req.uid)
                .gid(req.gid)
                .process_group(0)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .kill_on_drop(true);
            let mut child = cmd.spawn().map_err(|e| {
                log::error!("cannot start the settings child: {}", e.kind());
                fail("settings-spawn", "Could not write the account's settings.")
            })?;
            let pgid = child
                .id()
                .and_then(|p| rustix::process::Pid::from_raw(p as i32));
            let mut stdin = child.stdin.take();
            let json = &req.json;
            let run = async {
                if let Some(mut s) = stdin.take() {
                    // a few hundred bytes: fits the pipe, never blocks
                    let _ = s.write_all(json).await;
                    drop(s);
                }
                child.wait().await
            };
            match tokio::time::timeout(CHILD_TIMEOUT, run).await {
                Ok(Ok(status)) if status.success() => Ok(()),
                Ok(Ok(status)) => {
                    log::error!("the settings child exited with {status}");
                    Err(fail(
                        "settings-failed",
                        "Could not write the account's settings.",
                    ))
                }
                Ok(Err(e)) => {
                    log::error!("waiting for the settings child: {}", e.kind());
                    Err(fail(
                        "settings-failed",
                        "Could not write the account's settings.",
                    ))
                }
                Err(_) => {
                    log::error!("the settings child ran over {CHILD_TIMEOUT:?}; killing it");
                    if let Some(pgid) = pgid {
                        let _ = rustix::process::kill_process_group(
                            pgid,
                            rustix::process::Signal::KILL,
                        );
                    }
                    let _ = child.kill().await;
                    Err(fail(
                        "settings-timeout",
                        "Writing the account's settings took too long.",
                    ))
                }
            }
        })
    }
}

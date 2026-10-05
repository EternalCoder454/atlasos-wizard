//! Tests of `core` with fake AccountsService, systemd, runner and settings
//! child, on a temporary root.

use super::*;
use crate::backends::{BackendError, BoxFut};
use crate::error::Kind;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use wizard_core::choices::Value;

const PW: &str = "violet-Plum-Orbit-4711";

fn me() -> u32 {
    fs::metadata("/proc/self").unwrap().uid()
}

/// The uid the fakes hand out: ours when that is a human uid, else 1000 (the
/// home is then chown-ed, which needs root, as in the dev container).
fn account_uid() -> u32 {
    if me() >= 1000 { me() } else { 1000 }
}

/// Fake AccountsService working on the temp root's passwd, shadow and group.
struct FakeAccounts {
    root: PathBuf,
    state_path: PathBuf,
    log: StdMutex<Vec<String>>,
    fail_set_password: StdMutex<bool>,
    hang_create: bool,
    delay: Duration,
    uid: u32,
}

impl FakeAccounts {
    fn log(&self, s: impl Into<String>) {
        self.log.lock().unwrap().push(s.into());
    }

    fn stage_now(&self) -> String {
        match fs::read_to_string(&self.state_path) {
            Ok(t) => serde_json::from_str::<serde_json::Value>(&t).unwrap()["account"]["stage"]
                .as_str()
                .unwrap_or("-")
                .to_string(),
            Err(_) => "no-state".into(),
        }
    }

    fn edit(&self, rel: &str, keep: impl Fn(&str) -> bool, add: Option<String>) {
        let p = self.root.join(rel);
        let old = fs::read_to_string(&p).unwrap_or_default();
        let mut out: Vec<&str> = old.lines().filter(|l| keep(l)).collect();
        let add = add.unwrap_or_default();
        if !add.is_empty() {
            out.push(&add);
        }
        let mut s = out.join("\n");
        s.push('\n');
        fs::write(p, s).unwrap();
    }

    fn name_of(&self, uid: u32) -> String {
        let passwd = fs::read_to_string(self.root.join("etc/passwd")).unwrap();
        passwd
            .lines()
            .find_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                (f[2] == uid.to_string()).then(|| f[0].to_string())
            })
            .unwrap()
    }
}

impl AccountsApi for FakeAccounts {
    fn create_user<'a>(
        &'a self,
        name: &'a str,
        full_name: &'a str,
    ) -> BoxFut<'a, Result<String, BackendError>> {
        Box::pin(async move {
            self.log(format!("create_user {name} (state {})", self.stage_now()));
            if self.hang_create {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(self.delay).await;
            let uid = self.uid;
            self.edit(
                "etc/passwd",
                |_| true,
                Some(format!(
                    "{name}:x:{uid}:{uid}:{full_name}:/home/{name}:/bin/bash"
                )),
            );
            self.edit(
                "etc/shadow",
                |_| true,
                Some(format!("{name}:!:19000::::::")),
            );
            self.edit(
                "etc/group",
                |l| !l.starts_with("wheel:"),
                Some(format!("wheel:x:10:{name}")),
            );
            let home = self.root.join("home").join(name);
            fs::create_dir_all(&home).unwrap();
            fs::write(home.join(".bashrc"), "").unwrap();
            let _ = std::os::unix::fs::chown(&home, Some(uid), Some(uid));
            Ok(format!("/org/freedesktop/Accounts/User{uid}"))
        })
    }

    fn set_password<'a>(
        &'a self,
        user_path: &'a str,
        hash: &'a str,
    ) -> BoxFut<'a, Result<(), BackendError>> {
        Box::pin(async move {
            self.log(format!(
                "set_password {user_path} (state {})",
                self.stage_now()
            ));
            if std::mem::take(&mut *self.fail_set_password.lock().unwrap()) {
                return Err(BackendError::new("boom"));
            }
            let name = self.name_of(uid_from_path(user_path).unwrap());
            let prefix = format!("{name}:");
            self.edit(
                "etc/shadow",
                |l| !l.starts_with(&prefix),
                Some(format!("{name}:{hash}:19000::::::")),
            );
            Ok(())
        })
    }

    fn delete_user(&self, uid: u32) -> BoxFut<'_, Result<(), BackendError>> {
        Box::pin(async move {
            self.log(format!("delete_user {uid}"));
            let name = self.name_of(uid);
            let prefix = format!("{name}:");
            self.edit("etc/passwd", |l| !l.starts_with(&prefix), None);
            self.edit("etc/shadow", |l| !l.starts_with(&prefix), None);
            let _ = fs::remove_dir_all(self.root.join("home").join(&name));
            Ok(())
        })
    }
}

#[derive(Default)]
struct FakeSystemd {
    log: StdMutex<Vec<String>>,
}

impl SystemdApi for FakeSystemd {
    fn restart_unit<'a>(&'a self, unit: &'a str) -> BoxFut<'a, Result<(), BackendError>> {
        self.log.lock().unwrap().push(format!("restart {unit}"));
        Box::pin(async { Ok(()) })
    }
    fn start_unit<'a>(&'a self, unit: &'a str) -> BoxFut<'a, Result<(), BackendError>> {
        self.log.lock().unwrap().push(format!("start {unit}"));
        Box::pin(async { Ok(()) })
    }
}

/// Logs the commands and, like the real ones, edits the temp root's shadow
/// (`chage -E 0`) and passwd (`usermod -s`) unless told to do nothing.
#[derive(Default)]
struct FakeRunner {
    log: StdMutex<Vec<String>>,
    fail: bool,
    root: PathBuf,
    /// Runs that do nothing at all, from the start (they still succeed).
    ignore_first: StdMutex<usize>,
}

impl FakeRunner {
    fn rewrite(&self, rel: &str, user: &str, f: impl Fn(&mut Vec<String>)) {
        let p = self.root.join(rel);
        let text = fs::read_to_string(&p).unwrap();
        let out: Vec<String> = text
            .lines()
            .map(|l| {
                let mut fields: Vec<String> = l.split(':').map(String::from).collect();
                if fields[0] == user {
                    f(&mut fields);
                }
                fields.join(":")
            })
            .collect();
        fs::write(p, out.join("\n") + "\n").unwrap();
    }
}

impl Runner for FakeRunner {
    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFut<'a, Result<(), BackendError>> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{program} {}", args.join(" ")));
        let fail = self.fail;
        let ignored = {
            let mut n = self.ignore_first.lock().unwrap();
            let ignore = *n > 0;
            *n = n.saturating_sub(1);
            ignore
        };
        if !fail && !ignored {
            let user = args.last().copied().unwrap_or("");
            match (program, args.first().copied()) {
                ("/usr/bin/chage", Some("-E")) => {
                    let day = args[1].to_string();
                    self.rewrite("etc/shadow", user, |f| f[7] = day.clone());
                }
                ("/usr/sbin/usermod", Some("-s")) => {
                    let shell = args[1].to_string();
                    self.rewrite("etc/passwd", user, |f| f[6] = shell.clone());
                }
                _ => {}
            }
        }
        Box::pin(async move {
            if fail {
                Err(BackendError::new("no"))
            } else {
                Ok(())
            }
        })
    }
}

#[derive(Default)]
struct FakeApplier {
    reqs: StdMutex<Vec<UserSettingsRequest>>,
    fail: bool,
}

impl SettingsApplier for FakeApplier {
    fn apply<'a>(&'a self, req: &'a UserSettingsRequest) -> BoxFut<'a, Result<(), HelperError>> {
        self.reqs.lock().unwrap().push(req.clone());
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(HelperError::failed("settings-failed", "no"))
            } else {
                Ok(())
            }
        })
    }
}

struct Rig {
    dir: tempfile::TempDir,
    core: Arc<Core>,
    accounts: Arc<FakeAccounts>,
    systemd: Arc<FakeSystemd>,
    runner: Arc<FakeRunner>,
    applier: Arc<FakeApplier>,
}

#[derive(Default)]
struct Opts {
    hang_create: bool,
    delay_ms: u64,
    runner_fails: bool,
    runner_ignores: usize,
    create_uid: Option<u32>,
    applier_fails: bool,
    limit: Option<Duration>,
}

fn rig_with(o: Opts) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    for d in [
        "etc/skel",
        "etc/atlasos",
        "etc/plasmalogin.conf.d",
        "var/lib/atlas-wizard",
        "home",
    ] {
        fs::create_dir_all(r.join(d)).unwrap();
    }
    fs::write(r.join("etc/skel/.bashrc"), "").unwrap();
    fs::write(
        r.join("etc/passwd"),
        "root:x:0:0:root:/root:/bin/bash\natlas-setup:x:970:970::/run/atlas-setup:/bin/sh\n",
    )
    .unwrap();
    fs::write(
        r.join("etc/shadow"),
        "root:!:19000::::::\natlas-setup:!*:19000::::::\n",
    )
    .unwrap();
    fs::write(
        r.join("etc/group"),
        "root:x:0:\nwheel:x:10:\natlas-setup:x:970:\n",
    )
    .unwrap();
    fs::write(
        r.join("etc/plasmalogin.conf.d/99-atlas-wizard.conf"),
        "[Autologin]\nUser=atlas-setup\n",
    )
    .unwrap();
    let paths = Paths::with_root(r);
    let accounts = Arc::new(FakeAccounts {
        root: r.to_path_buf(),
        state_path: paths.state(),
        log: StdMutex::default(),
        fail_set_password: StdMutex::new(false),
        hang_create: o.hang_create,
        delay: Duration::from_millis(o.delay_ms),
        uid: o.create_uid.unwrap_or_else(account_uid),
    });
    let systemd = Arc::new(FakeSystemd::default());
    let runner = Arc::new(FakeRunner {
        fail: o.runner_fails,
        root: r.to_path_buf(),
        ignore_first: StdMutex::new(o.runner_ignores),
        ..Default::default()
    });
    let applier = Arc::new(FakeApplier {
        fail: o.applier_fails,
        ..Default::default()
    });
    let mut core = Core::new(
        paths,
        accounts.clone(),
        systemd.clone(),
        runner.clone(),
        applier.clone(),
    );
    if let Some(l) = o.limit {
        core = core.with_create_timeout(l);
    }
    Rig {
        dir,
        core: Arc::new(core),
        accounts,
        systemd,
        runner,
        applier,
    }
}

fn rig() -> Rig {
    rig_with(Opts::default())
}

impl Rig {
    async fn create(&self, name: &str, autologin: bool) -> Result<u32, HelperError> {
        self.core
            .create_account(
                name.into(),
                "Ada Lovelace".into(),
                Zeroizing::new(PW.as_bytes().to_vec()),
                autologin,
            )
            .await
    }

    fn state(&self) -> State {
        state::load(&self.core.paths().state()).unwrap().state
    }

    fn calls(&self) -> Vec<String> {
        self.accounts.log.lock().unwrap().clone()
    }

    fn save_state(&self, account: Option<Account>) {
        let st = State {
            account,
            ..State::default()
        };
        st.save(&self.core.paths().state()).unwrap();
    }
}

fn code(r: Result<impl std::fmt::Debug, HelperError>) -> (Kind, String) {
    let e = r.unwrap_err();
    (e.kind, e.code)
}

fn choices(pairs: &[(&str, Value)]) -> ChoiceMap {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[tokio::test]
async fn create_writes_the_stages_around_each_step() {
    let r = rig();
    let uid = r.create("ada", true).await.unwrap();
    assert_eq!(uid, account_uid());
    assert_eq!(
        r.calls(),
        vec![
            "create_user ada (state creating)".to_string(),
            format!("set_password /org/freedesktop/Accounts/User{uid} (state created)"),
        ]
    );
    let st = r.state();
    let acc = st.account.unwrap();
    assert_eq!(
        (acc.name.as_str(), acc.uid, acc.stage),
        ("ada", uid, Stage::Verified)
    );
    assert_eq!(st.extra["autologin"], serde_json::Value::Bool(true));
    // the hash is in shadow and checks against the password
    let shadow = fs::read_to_string(r.dir.path().join("etc/shadow")).unwrap();
    let hash = shadow
        .lines()
        .find_map(|l| l.strip_prefix("ada:"))
        .unwrap()
        .split(':')
        .next()
        .unwrap();
    assert!(password::verify(PW.as_bytes(), hash));
    // neither the password nor the hash is in the state
    let text = fs::read_to_string(r.core.paths().state()).unwrap();
    assert!(!text.contains(PW) && !text.contains(hash), "{text}");
}

#[tokio::test]
async fn bad_input_is_refused_before_accountsservice() {
    let r = rig();
    let call = |n: &str, f: &str, p: &str| {
        r.core.create_account(
            n.into(),
            f.into(),
            Zeroizing::new(p.as_bytes().to_vec()),
            false,
        )
    };
    assert_eq!(code(call("Bad Name", "Ada", PW).await).0, Kind::Invalid);
    assert_eq!(code(call("root", "Ada", PW).await).0, Kind::Invalid);
    assert_eq!(code(call("ada", "A:da", PW).await).0, Kind::Invalid);
    assert_eq!(code(call("ada", "Ada", "short").await).0, Kind::Invalid);
    assert_eq!(
        code(call("ada", "Ada", "ada-ada-ada-ada").await).0,
        Kind::Invalid
    );
    assert_eq!(
        code(call("ada", "Ada", "password1234").await).0,
        Kind::Invalid
    );
    assert!(r.calls().is_empty());
    assert!(!r.core.paths().state().exists());
}

#[tokio::test]
async fn done_setup_is_refused_for_every_method() {
    let r = rig();
    markers::write_missing(r.dir.path(), SystemTime::now()).unwrap();
    assert_eq!(
        code(r.create("ada", false).await),
        (Kind::SetupDone, "setup-done".into())
    );
    assert_eq!(
        code(r.core.finish(ChoiceMap::new()).await).0,
        Kind::SetupDone
    );
    assert_eq!(code(r.core.give_up().await).0, Kind::SetupDone);
    assert!(r.calls().is_empty());
}

#[tokio::test]
async fn one_account_per_first_run() {
    let r = rig();
    r.create("ada", false).await.unwrap();
    assert_eq!(
        code(r.create("bob", false).await),
        (Kind::Invalid, "account-exists".into())
    );
    assert_eq!(r.calls().len(), 2);
}

#[tokio::test]
async fn a_half_made_account_is_deleted_and_made_again() {
    let r = rig();
    *r.accounts.fail_set_password.lock().unwrap() = true;
    let (k, c) = code(r.create("ada", false).await);
    assert_eq!(
        (k, c.as_str()),
        (Kind::AccountsService, "accounts-password-failed")
    );
    assert_eq!(r.state().account.unwrap().stage, Stage::Created);
    let uid = r.create("ada", false).await.unwrap();
    let calls = r.calls();
    assert!(calls.contains(&format!("delete_user {uid}")), "{calls:?}");
    assert_eq!(r.state().account.unwrap().stage, Stage::Verified);
}

#[tokio::test]
async fn a_creating_stage_has_no_uid_and_is_found_by_name() {
    let r = rig();
    let uid = account_uid();
    fs::write(
        r.dir.path().join("etc/passwd"),
        format!(
            "atlas-setup:x:970:970::/run/atlas-setup:/bin/sh\nada:x:{uid}:{uid}::/home/ada:/bin/bash\n"
        ),
    )
    .unwrap();
    fs::write(r.dir.path().join("etc/shadow"), "ada:!:1::::::\n").unwrap();
    fs::create_dir_all(r.dir.path().join("home/ada")).unwrap();
    r.save_state(Some(Account {
        name: "ada".into(),
        uid: 0,
        stage: Stage::Creating,
    }));
    r.create("ada", false).await.unwrap();
    assert!(r.calls().contains(&format!("delete_user {uid}")));
}

#[tokio::test]
async fn a_half_made_account_with_files_in_its_home_is_never_deleted() {
    let r = rig();
    let uid = account_uid();
    *r.accounts.fail_set_password.lock().unwrap() = true;
    r.create("ada", false).await.unwrap_err();
    fs::write(r.dir.path().join("home/ada/precious.txt"), "mine").unwrap();
    let (k, c) = code(r.create("ada", false).await);
    assert_eq!((k, c.as_str()), (Kind::Failed, "half-made-has-files"));
    assert!(!r.calls().contains(&format!("delete_user {uid}")));
    assert!(r.dir.path().join("home/ada/precious.txt").exists());
}

#[tokio::test]
async fn only_the_uid_the_state_names_is_deleted() {
    let r = rig();
    let uid = account_uid();
    // passwd says another uid than the state
    fs::write(
        r.dir.path().join("etc/passwd"),
        format!("ada:x:{}:1::/home/ada:/bin/bash\n", uid + 1),
    )
    .unwrap();
    r.save_state(Some(Account {
        name: "ada".into(),
        uid,
        stage: Stage::Created,
    }));
    assert_eq!(code(r.create("ada", false).await).1, "half-made-mismatch");
    // a system uid is never deleted, whatever the state says
    fs::write(
        r.dir.path().join("etc/passwd"),
        "ada:x:5:5::/home/ada:/bin/bash\n",
    )
    .unwrap();
    r.save_state(Some(Account {
        name: "ada".into(),
        uid: 5,
        stage: Stage::Created,
    }));
    assert_eq!(code(r.create("ada", false).await).1, "half-made-mismatch");
    assert!(r.calls().is_empty());
}

#[tokio::test]
async fn a_state_account_missing_from_passwd_is_just_cleared() {
    let r = rig();
    r.save_state(Some(Account {
        name: "ghost".into(),
        uid: 1001,
        stage: Stage::PasswordSet,
    }));
    r.create("ada", false).await.unwrap();
    assert!(!r.calls().iter().any(|c| c.starts_with("delete_user")));
}

#[test]
fn home_check() {
    let r = rig();
    let p = r.core.paths();
    assert!(home_holds_only_skel(p, "/home/nobody"), "missing home");
    fs::create_dir_all(r.dir.path().join("home/a")).unwrap();
    assert!(home_holds_only_skel(p, "/home/a"), "empty");
    fs::write(r.dir.path().join("home/a/.bashrc"), "").unwrap();
    assert!(home_holds_only_skel(p, "/home/a"), "skel only");
    fs::write(r.dir.path().join("home/a/x"), "").unwrap();
    assert!(!home_holds_only_skel(p, "/home/a"), "extra file");
    assert!(!home_holds_only_skel(p, "/home/../etc"));
    assert!(!home_holds_only_skel(p, "home/a"));
    std::os::unix::fs::symlink("/etc", r.dir.path().join("home/l")).unwrap();
    assert!(!home_holds_only_skel(p, "/home/l"), "a symlink");
}

#[tokio::test]
async fn concurrent_calls_run_one_at_a_time() {
    let r = rig_with(Opts {
        delay_ms: 80,
        ..Opts::default()
    });
    let (a, b) = tokio::join!(r.create("ada", false), r.create("bob", false));
    assert_eq!(
        [a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(),
        1,
        "{a:?} {b:?}"
    );
    let loser = if a.is_err() { a } else { b };
    assert_eq!(code(loser).1, "account-exists");
    assert_eq!(
        r.calls()
            .iter()
            .filter(|c| c.starts_with("create_user"))
            .count(),
        1
    );
}

#[tokio::test]
async fn create_gives_up_after_its_time_limit() {
    let r = rig_with(Opts {
        hang_create: true,
        limit: Some(Duration::from_millis(300)),
        ..Opts::default()
    });
    assert_eq!(
        code(r.create("ada", false).await),
        (Kind::Failed, "timeout".into())
    );
    // the state names the half-made account, so the next call can resolve it
    assert_eq!(r.state().account.unwrap().stage, Stage::Creating);
}

#[tokio::test]
async fn finish_does_every_step_and_is_then_closed() {
    let r = rig();
    let uid = r.create("ada", true).await.unwrap();
    let c = choices(&[
        ("look", Value::Str("dark".into())),
        ("text_scale", Value::F64(1.25)),
    ]);
    r.core.finish(c).await.unwrap();

    let reqs = r.applier.reqs.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        (reqs[0].name.as_str(), reqs[0].uid, reqs[0].gid),
        ("ada", uid, uid)
    );
    assert_eq!(reqs[0].home, r.dir.path().join("home/ada"));
    let json = String::from_utf8(reqs[0].json.clone()).unwrap();
    assert!(
        json.contains("\"look\":\"dark\"") && json.contains("1.25"),
        "{json}"
    );

    let drop_in = fs::read_to_string(
        r.dir
            .path()
            .join("etc/plasmalogin.conf.d/50-atlas-autologin.conf"),
    )
    .unwrap();
    assert_eq!(drop_in, "[Autologin]\nUser=ada\nSession=plasma\n");
    assert!(markers::present(r.dir.path()).both());
    assert!(
        !r.dir
            .path()
            .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf")
            .exists()
    );
    assert_eq!(
        *r.runner.log.lock().unwrap(),
        vec![
            "/usr/bin/chage -E 0 atlas-setup".to_string(),
            "/usr/sbin/usermod -s /usr/sbin/nologin atlas-setup".to_string(),
        ]
    );
    assert_eq!(r.state().finish.as_deref(), Some(FINISH_DONE));

    // a repeat is refused, EndSetup now works
    assert_eq!(
        code(r.core.finish(ChoiceMap::new()).await).0,
        Kind::SetupDone
    );
    r.core.end_setup().await.unwrap();
    assert_eq!(
        *r.systemd.log.lock().unwrap(),
        vec!["restart display-manager.service".to_string()]
    );
    assert_eq!(code(r.create("bob", false).await).0, Kind::SetupDone);
}

#[tokio::test]
async fn finish_without_autologin_writes_no_drop_in() {
    let r = rig();
    r.create("ada", false).await.unwrap();
    r.core.finish(ChoiceMap::new()).await.unwrap();
    assert!(
        !r.dir
            .path()
            .join("etc/plasmalogin.conf.d/50-atlas-autologin.conf")
            .exists()
    );
}

#[tokio::test]
async fn finish_refuses_bad_choices_and_a_missing_account() {
    let r = rig();
    assert_eq!(code(r.core.finish(ChoiceMap::new()).await).1, "no-account");
    r.create("ada", false).await.unwrap();
    let bad = choices(&[("surprise", Value::Bool(true))]);
    assert_eq!(
        code(r.core.finish(bad).await),
        (Kind::Invalid, "choices-unknown-key".into())
    );
    assert!(r.applier.reqs.lock().unwrap().is_empty());
    assert!(!markers::is_done(r.dir.path()));
}

#[tokio::test]
async fn a_failed_settings_child_leaves_setup_open_and_resumable() {
    let r = rig_with(Opts {
        applier_fails: true,
        ..Opts::default()
    });
    r.create("ada", true).await.unwrap();
    assert_eq!(
        code(r.core.finish(ChoiceMap::new()).await).1,
        "settings-failed"
    );
    assert!(!markers::is_done(r.dir.path()));
    assert_eq!(r.state().finish.as_deref(), Some(FINISH_SETTINGS));
    assert!(
        r.dir
            .path()
            .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf")
            .exists()
    );
    assert_eq!(code(r.core.end_setup().await).1, "not-finished");
}

#[tokio::test]
async fn finish_resumes_after_the_markers_with_only_the_clean_up() {
    let r = rig();
    r.create("ada", false).await.unwrap();
    // a crash right after the markers
    let mut st = r.state();
    st.finish = Some(FINISH_MARKERS.into());
    st.save(&r.core.paths().state()).unwrap();
    markers::write_missing(r.dir.path(), SystemTime::now()).unwrap();
    r.core.finish(ChoiceMap::new()).await.unwrap();
    assert!(
        r.applier.reqs.lock().unwrap().is_empty(),
        "settings are not written twice"
    );
    assert!(
        !r.dir
            .path()
            .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf")
            .exists()
    );
    assert_eq!(r.runner.log.lock().unwrap().len(), 2);
    assert_eq!(r.state().finish.as_deref(), Some(FINISH_DONE));
}

#[tokio::test]
async fn failing_lock_commands_do_not_fail_finish() {
    let r = rig_with(Opts {
        runner_fails: true,
        ..Opts::default()
    });
    r.create("ada", false).await.unwrap();
    r.core.finish(ChoiceMap::new()).await.unwrap();
    assert!(markers::present(r.dir.path()).both());
}

#[tokio::test]
async fn give_up_records_removes_the_autologin_and_starts_the_fallback() {
    let r = rig();
    r.core.give_up().await.unwrap();
    assert!(r.state().gave_up);
    assert!(
        !r.dir
            .path()
            .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf")
            .exists()
    );
    assert_eq!(
        *r.systemd.log.lock().unwrap(),
        vec!["start atlas-wizard-fallback.service".to_string()]
    );
    // again: still fine
    r.core.give_up().await.unwrap();
}

#[tokio::test]
async fn end_setup_needs_a_finished_setup() {
    let r = rig();
    assert_eq!(code(r.core.end_setup().await).1, "not-finished");
    assert!(r.systemd.log.lock().unwrap().is_empty());
}

// ----- S fixes: clean-up check, half-made safety, uid range, GiveUp -----------

async fn finished(r: &Rig) {
    r.create("ada", true).await.unwrap();
    r.core.finish(ChoiceMap::new()).await.unwrap();
}

#[tokio::test]
async fn end_setup_checks_the_clean_up_and_restarts_when_it_holds() {
    let r = rig();
    finished(&r).await;
    assert!(r.core.cleanup_gaps().is_empty());
    r.core.end_setup().await.unwrap();
    assert_eq!(r.systemd.log.lock().unwrap().len(), 1);
    // no extra lock runs when nothing was missing
    assert_eq!(r.runner.log.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn end_setup_redoes_a_missing_clean_up_once() {
    let r = rig();
    finished(&r).await;
    // the autologin came back and the account was un-expired behind our back
    fs::write(
        r.dir
            .path()
            .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf"),
        "[Autologin]\nUser=atlas-setup\n",
    )
    .unwrap();
    r.runner
        .rewrite("etc/shadow", "atlas-setup", |f| f[7] = String::new());
    r.core.end_setup().await.unwrap();
    assert!(
        !r.dir
            .path()
            .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf")
            .exists()
    );
    assert_eq!(r.runner.log.lock().unwrap().len(), 4, "the lock ran again");
    assert_eq!(r.systemd.log.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn end_setup_does_not_restart_when_the_clean_up_cannot_be_completed() {
    let r = rig();
    finished(&r).await;
    // the shell is wrong and the lock commands do nothing
    r.runner
        .rewrite("etc/passwd", "atlas-setup", |f| f[6] = "/bin/sh".into());
    *r.runner.ignore_first.lock().unwrap() = 2;
    let (k, c) = code(r.core.end_setup().await);
    assert_eq!((k, c.as_str()), (Kind::Failed, "cleanup-incomplete"));
    assert!(r.systemd.log.lock().unwrap().is_empty());
    assert_eq!(r.runner.log.lock().unwrap().len(), 4, "redone exactly once");
}

#[test]
fn the_expire_field_must_be_a_day_that_has_passed() {
    let r = rig();
    let shell = |f: &mut Vec<String>| f[6] = "/usr/sbin/nologin".into();
    r.runner.rewrite("etc/passwd", "atlas-setup", shell);
    let gaps = |expire: &str| {
        let e = expire.to_string();
        r.runner
            .rewrite("etc/shadow", "atlas-setup", move |f| f[7] = e.clone());
        fs::remove_file(
            r.dir
                .path()
                .join("etc/plasmalogin.conf.d/99-atlas-wizard.conf"),
        )
        .ok();
        r.core.cleanup_gaps()
    };
    assert!(gaps("0").is_empty());
    assert!(gaps("1").is_empty());
    assert_eq!(gaps(""), vec!["account-not-expired"]);
    assert_eq!(gaps("-1"), vec!["account-not-expired"]);
    assert_eq!(
        gaps("99999"),
        vec!["account-not-expired"],
        "a day in the future"
    );
}

#[tokio::test]
async fn a_creating_stage_account_with_a_real_hash_is_not_deleted() {
    let r = rig();
    let uid = account_uid();
    fs::write(
        r.dir.path().join("etc/passwd"),
        format!("ada:x:{uid}:{uid}::/home/ada:/bin/bash\n"),
    )
    .unwrap();
    fs::write(
        r.dir.path().join("etc/shadow"),
        "ada:$y$j9T$salt$hash:19000::::::\n",
    )
    .unwrap();
    fs::create_dir_all(r.dir.path().join("home/ada")).unwrap();
    r.save_state(Some(Account {
        name: "ada".into(),
        uid: 0,
        stage: Stage::Creating,
    }));
    let (k, c) = code(r.create("ada", false).await);
    assert_eq!((k, c.as_str()), (Kind::Failed, "half-made-account-unclear"));
    assert!(r.calls().is_empty(), "{:?}", r.calls());
    assert!(r.dir.path().join("home/ada").exists());
}

#[tokio::test]
async fn a_creating_stage_account_in_the_system_uid_range_is_not_deleted() {
    let r = rig();
    fs::write(
        r.dir.path().join("etc/passwd"),
        "ada:x:5:5::/home/ada:/bin/bash\n",
    )
    .unwrap();
    fs::write(r.dir.path().join("etc/shadow"), "ada:!:19000::::::\n").unwrap();
    r.save_state(Some(Account {
        name: "ada".into(),
        uid: 0,
        stage: Stage::Creating,
    }));
    assert_eq!(
        code(r.create("ada", false).await).1,
        "half-made-account-unclear"
    );
    assert!(r.calls().is_empty());
}

#[tokio::test]
async fn a_uid_outside_the_human_range_is_refused_and_left_half_made() {
    let r = rig_with(Opts {
        create_uid: Some(5),
        ..Opts::default()
    });
    let (k, c) = code(r.create("ada", false).await);
    assert_eq!((k, c.as_str()), (Kind::AccountsService, "accounts-bad-uid"));
    let acc = r.state().account.unwrap();
    assert_eq!((acc.stage, acc.uid), (Stage::Creating, 0));
    assert!(!r.calls().iter().any(|c| c.starts_with("set_password")));
    // next time the system-range account is not deleted, it is refused
    assert_eq!(
        code(r.create("ada", false).await).1,
        "half-made-account-unclear"
    );
    assert!(!r.calls().iter().any(|c| c.starts_with("delete_user")));
}

#[tokio::test]
async fn give_up_with_an_account_or_a_finish_under_way_starts_the_fallback() {
    // the fallback finishes without asking; refusing would strand the machine
    let r = rig();
    r.create("ada", false).await.unwrap();
    r.core.give_up().await.unwrap();
    assert!(r.state().gave_up);
    assert_eq!(
        *r.systemd.log.lock().unwrap(),
        vec!["start atlas-wizard-fallback.service".to_string()]
    );

    let r = rig();
    let st = State {
        finish: Some(FINISH_SETTINGS.into()),
        ..State::default()
    };
    st.save(&r.core.paths().state()).unwrap();
    r.core.give_up().await.unwrap();
    assert_eq!(
        *r.systemd.log.lock().unwrap(),
        vec!["start atlas-wizard-fallback.service".to_string()]
    );

    // a half-made account is still fine to give up on
    let r = rig();
    r.save_state(Some(Account {
        name: "ada".into(),
        uid: 1000,
        stage: Stage::Created,
    }));
    r.core.give_up().await.unwrap();
}

#[tokio::test]
async fn end_setup_finishes_the_tail_when_the_markers_exist_but_finish_is_markers() {
    let r = rig();
    finished(&r).await;
    // as if the helper stopped right after writing the markers
    let mut st = r.state();
    st.finish = Some(FINISH_MARKERS.into());
    st.save(&r.core.paths().state()).unwrap();
    // the session's GiveUp answers setup-done, then EndSetup must work
    assert_eq!(code(r.core.give_up().await).0, Kind::SetupDone);
    let before = r.runner.log.lock().unwrap().len();
    r.core.end_setup().await.unwrap();
    assert_eq!(r.state().finish.as_deref(), Some(FINISH_DONE));
    assert_eq!(r.runner.log.lock().unwrap().len(), before + 2, "locked");
    assert!(r.core.cleanup_gaps().is_empty());
    assert_eq!(
        *r.systemd.log.lock().unwrap(),
        vec!["restart display-manager.service".to_string()]
    );
}

#[tokio::test]
async fn create_account_is_refused_after_giving_up() {
    let r = rig();
    r.core.give_up().await.unwrap();
    let (k, c) = code(r.create("ada", false).await);
    assert_eq!((k, c.as_str()), (Kind::Invalid, "gave-up"));
    assert!(r.state().account.is_none());
}

#[tokio::test]
async fn finish_is_refused_after_giving_up_unless_resuming_the_markers() {
    let r = rig();
    r.create("ada", false).await.unwrap();
    r.core.give_up().await.unwrap();
    let (k, c) = code(r.core.finish(ChoiceMap::new()).await);
    assert_eq!((k, c.as_str()), (Kind::Invalid, "gave-up"));
    assert!(r.applier.reqs.lock().unwrap().is_empty());

    // markers written, tail missing: Finish may resume despite gave_up
    markers::write_missing(r.dir.path(), SystemTime::now()).unwrap();
    let mut st = r.state();
    st.finish = Some(FINISH_MARKERS.into());
    st.save(&r.core.paths().state()).unwrap();
    r.core.finish(ChoiceMap::new()).await.unwrap();
    assert_eq!(r.state().finish.as_deref(), Some(FINISH_DONE));
}

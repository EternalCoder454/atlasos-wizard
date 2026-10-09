//! The helper end to end, in a throwaway world: a private dbus-daemon used as
//! the system bus, python-dbusmock's polkitd, a mock AccountsService and a mock
//! systemd (tests/helper/mocks), the helper binary built with `test-root`, and
//! a temporary root holding passwd, shadow, group, skel and fake tools.
//!
//! Nothing here touches the real system: every path is under the temp root and
//! every bus is private. The tests skip (with a message) when dbus-daemon or
//! python-dbusmock is missing; set TELAMON_WIZARD_REQUIRE_BUS_TESTS=1 to make
//! that a failure instead.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use zbus::zvariant::{Dict, Value};

const BUS_NAME: &str = "net.eterneon.telamon.WizardHelper";
const OBJECT: &str = "/net/eterneon/telamon/WizardHelper";
const IFACE: &str = "net.eterneon.telamon.WizardHelper1";
const ERR: &str = "net.eterneon.telamon.Error";
const PW: &str = "violet-Plum-Orbit-4711";

fn me() -> u32 {
    fs::metadata("/proc/self").unwrap().uid()
}

/// The uid the mock AccountsService gives the new account.
fn account_uid() -> u32 {
    if me() >= 1000 { me() } else { 1000 }
}

fn have(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tools_available() -> bool {
    have(Command::new("dbus-daemon").arg("--version"))
        && have(Command::new("python3").args(["-c", "import dbusmock"]))
}

struct Opts {
    allowed: Vec<&'static str>,
    setup_uid: u32,
    idle_ms: u64,
    /// `Some(true)`: the test bus enforces the shipped bus policy with the
    /// current user standing in for `telamon-setup`; `Some(false)`: with
    /// somebody else standing in (and root's rule moved off us), so sending
    /// is denied. `None`: everything may send.
    shipped_policy: Option<bool>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            allowed: vec![
                "net.eterneon.telamon.wizard.create-account",
                "net.eterneon.telamon.wizard.finish",
                "net.eterneon.telamon.wizard.fallback",
            ],
            setup_uid: me(),
            idle_ms: 120_000,
            shipped_policy: None,
        }
    }
}

struct World {
    dir: tempfile::TempDir,
    root: PathBuf,
    procs: Vec<Child>,
    helper: Option<Child>,
    helper_log: PathBuf,
    conn: zbus::Connection,
}

impl Drop for World {
    fn drop(&mut self) {
        for p in self.helper.iter_mut().chain(self.procs.iter_mut()) {
            let _ = p.kill();
            let _ = p.wait();
        }
    }
}

/// The shipped bus policy's `<policy>` blocks, with the user names swapped for
/// ones the test bus can tell apart: `telamon-setup` becomes this user (or
/// `nobody`) and `root` becomes `nobody`, so root running the tests gets no
/// special pass.
fn shipped_policy_rules(allow_me: Option<bool>) -> String {
    let Some(allow_me) = allow_me else {
        return String::new();
    };
    let me = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
    let me = me.trim();
    assert!(
        !me.is_empty() && me != "nobody",
        "cannot tell this user from nobody"
    );
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/dbus-1/system.d/net.eterneon.telamon.WizardHelper.conf"),
    )
    .unwrap();
    let body = text
        .split_once("<busconfig>")
        .and_then(|(_, rest)| rest.rsplit_once("</busconfig>"))
        .expect("a busconfig")
        .0;
    assert!(body.contains("user=\"telamon-setup\"") && body.contains("user=\"root\""));
    assert!(
        !body.contains("context=\"default\""),
        "the default must stay deny"
    );
    body.replace("user=\"root\"", "user=\"nobody\"").replace(
        "user=\"telamon-setup\"",
        &format!("user=\"{}\"", if allow_me { me } else { "nobody" }),
    )
}

fn chmod(p: &Path, mode: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

fn write_tool(root: &Path, log: &Path, rel: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let script = format!(
        "#!/bin/sh\n\
         printf 'TOOL %s|%s|uid=%s|gid=%s|groups=%s|home=%s|env=%s\\n' \
         \"$(basename \"$0\")\" \"$*\" \"$(id -u)\" \"$(id -g)\" \"$(id -G)\" \"$HOME\" \
         \"$(env | grep -v '^_=' | grep -v '^PWD=' | grep -v '^SHLVL=' | sort | tr '\\n' ' ')\" >> '{}'\n",
        log.display()
    );
    // like the real commands, the lock tools change the temp root's files
    let effect = match rel.rsplit('/').next() {
        Some("chage") => format!(
            "[ \"$1\" = -E ] && sed -i \"s/^\\(telamon-setup:\\([^:]*:\\)\\{{6\\}}\\)[^:]*/\\1$2/\" '{}'\n",
            root.join("etc/shadow").display()
        ),
        Some("usermod") => format!(
            "[ \"$1\" = -s ] && sed -i \"s|^\\(telamon-setup:\\([^:]*:\\)\\{{5\\}}\\)[^:]*|\\1$2|\" '{}'\n",
            root.join("etc/passwd").display()
        ),
        _ => String::new(),
    };
    fs::write(&p, script + &effect).unwrap();
    chmod(&p, 0o755);
}

fn fixture(root: &Path, setup_uid: u32) -> PathBuf {
    for d in [
        "etc/skel",
        "etc/telamon",
        "etc/atlasos",
        "etc/xdg",
        "etc/plasmalogin.conf.d",
        "var/lib/telamon-wizard",
        "usr/bin",
        "usr/sbin",
        "usr/share/color-schemes",
        "home",
        "log",
    ] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(root.join("etc/skel/.bashrc"), "# skel\n").unwrap();
    fs::write(
        root.join("etc/passwd"),
        format!(
            "root:x:0:0:root:/root:/bin/bash\ntelamon-setup:x:{setup_uid}:970:Telamon Setup:/run/telamon-setup:/bin/sh\n"
        ),
    )
    .unwrap();
    fs::write(
        root.join("etc/shadow"),
        "root:!:19000::::::\ntelamon-setup:!*:19000::::::\n",
    )
    .unwrap();
    fs::write(
        root.join("etc/group"),
        "root:x:0:\nwheel:x:10:\ntelamon-setup:x:970:\n",
    )
    .unwrap();
    fs::write(
        root.join("etc/plasmalogin.conf.d/99-telamon-wizard.conf"),
        "[Autologin]\nUser=telamon-setup\nSession=telamon-wizard\nRelogin=true\n",
    )
    .unwrap();
    fs::write(
        root.join("etc/xdg/kdeglobals"),
        "[General]\nfont=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
         menuFont=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
         toolBarFont=Noto Sans,9,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
         smallestReadableFont=Noto Sans,8,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
         fixed=Hack,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
         [WM]\nactiveFont=Noto Sans,10,-1,5,700,0,0,0,0,0,0,0,0,0,0,1\n",
    )
    .unwrap();
    fs::write(
        root.join("usr/share/color-schemes/AtlasOSHighContrast.colors"),
        "",
    )
    .unwrap();
    let log = root.join("log/tools.log");
    fs::write(&log, "").unwrap();
    for t in [
        "usr/bin/chage",
        "usr/sbin/usermod",
        "usr/bin/plasma-apply-lookandfeel",
        "usr/bin/plasma-apply-colorscheme",
    ] {
        write_tool(root, &log, t);
    }
    // the settings child runs as another uid: let it walk and log
    fn open_up(dir: &Path) {
        chmod(dir, 0o755);
        for e in fs::read_dir(dir).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                open_up(&e.path());
            }
        }
    }
    open_up(root);
    chmod(&root.join("log"), 0o777);
    chmod(&log, 0o666);
    log
}

fn spawn_logged(cmd: &mut Command, out: &Path) -> Child {
    let f = fs::File::create(out).unwrap();
    cmd.stdout(f.try_clone().unwrap())
        .stderr(f)
        .stdin(Stdio::null())
        .spawn()
        .unwrap()
}

async fn wait_for<F: std::future::Future<Output = bool>>(what: &str, mut f: impl FnMut() -> F) {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn name_owned(conn: &zbus::Connection, name: &str) -> bool {
    let Ok(proxy) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(name.to_string()) else {
        return false;
    };
    proxy.name_has_owner(name).await.unwrap_or(false)
}

impl World {
    async fn new(o: Opts) -> Option<World> {
        if !tools_available() {
            assert!(
                std::env::var_os("TELAMON_WIZARD_REQUIRE_BUS_TESTS").is_none(),
                "dbus-daemon or python-dbusmock is missing"
            );
            eprintln!("SKIP: dbus-daemon or python-dbusmock is not installed");
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        chmod(dir.path(), 0o755);
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        fixture(&root, o.setup_uid);
        let bus_dir = dir.path().join("bus");
        fs::create_dir(&bus_dir).unwrap();
        let sock = bus_dir.join("bus.sock");
        let conf = bus_dir.join("bus.conf");
        fs::write(
            &conf,
            format!(
                "<busconfig><type>session</type><keep_umask/>\
                 <listen>unix:path={}</listen><auth>EXTERNAL</auth>\
                 <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
                 <allow eavesdrop=\"true\"/><allow own=\"*\"/>{}</policy>{}</busconfig>",
                sock.display(),
                // the default of the real system bus for our name is deny
                if o.shipped_policy.is_some() {
                    "<deny send_destination=\"net.eterneon.telamon.WizardHelper\"/>"
                } else {
                    ""
                },
                shipped_policy_rules(o.shipped_policy),
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", conf.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut addr = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut addr)
            .unwrap();
        let addr = addr.trim().to_string();
        assert!(addr.starts_with("unix:"), "dbus-daemon said {addr:?}");

        let mocks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/helper/mocks");
        let params = format!(
            "{{\"root\": {:?}, \"first_uid\": {}}}",
            root.to_string_lossy(),
            account_uid()
        );
        let mock = |args: Vec<String>, log: &str| {
            spawn_logged(
                Command::new("python3")
                    .arg("-m")
                    .arg("dbusmock")
                    .args(args)
                    .env("DBUS_SYSTEM_BUS_ADDRESS", &addr)
                    .env("DBUS_SESSION_BUS_ADDRESS", &addr),
                &bus_dir.join(log),
            )
        };
        let procs = vec![
            daemon,
            mock(vec!["-t".into(), "polkitd".into()], "polkitd.log"),
            mock(
                vec![
                    "-t".into(),
                    mocks.join("accounts.py").to_string_lossy().into_owned(),
                    "-p".into(),
                    params,
                ],
                "accounts.log",
            ),
            mock(
                vec![
                    "-t".into(),
                    mocks.join("systemd1.py").to_string_lossy().into_owned(),
                ],
                "systemd1.log",
            ),
        ];

        let conn = zbus::connection::Builder::address(addr.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut w = World {
            dir,
            root,
            procs,
            helper: None,
            helper_log: bus_dir.join("helper.log"),
            conn,
        };
        for n in [
            "org.freedesktop.PolicyKit1",
            "org.freedesktop.Accounts",
            "org.freedesktop.systemd1",
        ] {
            let c = w.conn.clone();
            wait_for(n, move || {
                let c = c.clone();
                async move { name_owned(&c, n).await }
            })
            .await;
        }
        w.set_allowed(&o.allowed).await;

        let helper = spawn_logged(
            Command::new(env!("CARGO_BIN_EXE_telamon-wizard-helper"))
                .env_clear()
                .env("DBUS_SYSTEM_BUS_ADDRESS", &addr)
                .env("TELAMON_WIZARD_TEST_ROOT", &w.root)
                .env("TELAMON_WIZARD_TEST_IDLE_MS", o.idle_ms.to_string()),
            &w.helper_log,
        );
        w.helper = Some(helper);
        let c = w.conn.clone();
        wait_for("the helper's name", move || {
            let c = c.clone();
            async move { name_owned(&c, BUS_NAME).await }
        })
        .await;
        Some(w)
    }

    async fn mock_call<B>(
        &self,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(dest), path, Some(iface), method, body)
            .await
    }

    async fn set_allowed(&self, actions: &[&str]) {
        let actions: Vec<String> = actions.iter().map(ToString::to_string).collect();
        self.mock_call(
            "org.freedesktop.PolicyKit1",
            "/org/freedesktop/PolicyKit1/Authority",
            "org.freedesktop.DBus.Mock",
            "SetAllowed",
            &(actions,),
        )
        .await
        .unwrap();
    }

    async fn log_of(&self, dest: &str, path: &str) -> Vec<String> {
        self.mock_call(dest, path, "org.freedesktop.DBus.Mock", "GetLog", &())
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap()
    }

    async fn accounts_log(&self) -> Vec<String> {
        self.log_of("org.freedesktop.Accounts", "/org/freedesktop/Accounts")
            .await
    }

    async fn systemd_log(&self) -> Vec<String> {
        self.log_of("org.freedesktop.systemd1", "/org/freedesktop/systemd1")
            .await
    }

    async fn accounts_ctl<B>(&self, method: &str, body: &B)
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.mock_call(
            "org.freedesktop.Accounts",
            "/org/freedesktop/Accounts",
            "org.freedesktop.DBus.Mock",
            method,
            body,
        )
        .await
        .unwrap();
    }

    async fn call<B>(&self, method: &str, body: &B) -> Result<zbus::Message, (String, String)>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(BUS_NAME), OBJECT, Some(IFACE), method, body)
            .await
            .map_err(|e| match e {
                zbus::Error::MethodError(name, desc, _) => {
                    (name.to_string(), desc.unwrap_or_default())
                }
                other => panic!("unexpected error from {method}: {other}"),
            })
    }

    async fn create(&self, name: &str, autologin: bool) -> Result<u32, (String, String)> {
        self.create_with(name, PW.as_bytes().to_vec(), autologin)
            .await
    }

    async fn create_with(
        &self,
        name: &str,
        pw: Vec<u8>,
        autologin: bool,
    ) -> Result<u32, (String, String)> {
        let m = self
            .call(
                "CreateAccount",
                &(name.to_string(), "Ada Lovelace".to_string(), pw, autologin),
            )
            .await?;
        Ok(m.body().deserialize::<u32>().unwrap())
    }

    async fn finish(&self, c: HashMap<&str, Value<'_>>) -> Result<(), (String, String)> {
        self.call("Finish", &(c,)).await.map(|_| ())
    }

    async fn simple(&self, method: &str) -> Result<(), (String, String)> {
        self.call(method, &()).await.map(|_| ())
    }

    fn state(&self) -> serde_json::Value {
        let t = fs::read_to_string(self.root.join("var/lib/telamon-wizard/state.json")).unwrap();
        serde_json::from_str(&t).unwrap()
    }

    fn tools_log(&self) -> String {
        fs::read_to_string(self.root.join("log/tools.log")).unwrap()
    }

    /// Every file of the world (root, bus logs) as bytes, for secret scans.
    fn all_files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(d: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            for e in fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                let t = e.file_type().unwrap();
                if t.is_dir() {
                    walk(&p, out);
                } else if t.is_file() {
                    out.push((p.clone(), fs::read(&p).unwrap_or_default()));
                }
            }
        }
        let mut out = Vec::new();
        walk(self.dir.path(), &mut out);
        out
    }
}

fn err_name(e: &(String, String), short: &str) -> bool {
    e.0 == format!("{ERR}.{short}")
}

fn has_code(e: &(String, String), code: &str) -> bool {
    e.1.starts_with(&format!("{code}: "))
}

fn all_choices() -> HashMap<&'static str, Value<'static>> {
    let mut kb: HashMap<&str, Value<'_>> = HashMap::new();
    kb.insert("layout", Value::from("de"));
    kb.insert("variant", Value::from("nodeadkeys"));
    let mut c: HashMap<&str, Value<'_>> = HashMap::new();
    c.insert("look", Value::from("dark"));
    c.insert("accent", Value::from("#E93A9A"));
    c.insert("text_scale", Value::from(1.25f64));
    c.insert("high_contrast", Value::from(true));
    c.insert("screen_reader", Value::from(true));
    c.insert("crash_reports", Value::from(true));
    c.insert("keyboard", Value::Dict(Dict::from(kb)));
    c
}

#[tokio::test]
async fn full_flow_create_finish_end() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    let uid = w.create("ada", true).await.unwrap();
    assert_eq!(uid, account_uid());

    // the account as the system has it
    let passwd = fs::read_to_string(w.root.join("etc/passwd")).unwrap();
    assert!(passwd.contains(&format!(
        "ada:x:{uid}:{uid}:Ada Lovelace:/home/ada:/bin/bash"
    )));
    let shadow = fs::read_to_string(w.root.join("etc/shadow")).unwrap();
    let hash = shadow
        .lines()
        .find_map(|l| l.strip_prefix("ada:"))
        .unwrap()
        .split(':')
        .next()
        .unwrap()
        .to_string();
    assert!(wizard_core::password::verify(PW.as_bytes(), &hash));
    assert!(hash.starts_with("$y$"), "yescrypt: {hash}");
    let st = w.state();
    assert_eq!(st["account"]["name"], "ada");
    assert_eq!(st["account"]["uid"], uid);
    assert_eq!(st["account"]["stage"], "verified");
    assert_eq!(st["autologin"], true);
    assert_eq!(
        w.accounts_log().await,
        vec![
            "CreateUser ada 1".to_string(),
            format!("SetPassword /org/freedesktop/Accounts/User{uid}"),
        ]
    );

    // not finished yet
    let e = w.simple("EndSetup").await.unwrap_err();
    assert!(
        err_name(&e, "Failed") && has_code(&e, "not-finished"),
        "{e:?}"
    );

    w.finish(all_choices()).await.unwrap();

    // settings, written by the child as the account
    let home = w.root.join("home/ada");
    let kde = fs::read_to_string(home.join(".config/kdeglobals")).unwrap();
    assert!(kde.contains("font=Noto Sans,12.5,-1,5,400"), "{kde}");
    assert!(kde.contains("activeFont=Noto Sans,12.5,-1,5,700"), "{kde}");
    assert!(kde.contains("ColorScheme=AtlasOSHighContrast"), "{kde}");
    let acc = fs::read_to_string(home.join(".config/kaccessrc")).unwrap();
    assert!(acc.contains("[ScreenReader]\nEnabled=true"), "{acc}");
    let kxkb = fs::read_to_string(home.join(".config/kxkbrc")).unwrap();
    assert!(
        kxkb.contains("LayoutList=de")
            && kxkb.contains("VariantList=nodeadkeys")
            && kxkb.contains("Use=true"),
        "{kxkb}"
    );
    let crash = fs::read_to_string(home.join(".config/telamon/crash-reporting.toml")).unwrap();
    assert!(crash.contains("enabled = true"), "{crash}");
    // Atlas.Ui 1.x apps read the choice from where they always did.
    assert_eq!(
        fs::read_to_string(home.join(".config/atlas/crash-reporting.toml")).unwrap(),
        crash
    );
    for f in [
        ".config/kdeglobals",
        ".config/kaccessrc",
        ".config/kxkbrc",
        ".config/telamon/crash-reporting.toml",
        ".config/atlas/crash-reporting.toml",
    ] {
        assert_eq!(
            fs::metadata(home.join(f)).unwrap().uid(),
            uid,
            "{f} is owned by the account"
        );
    }

    // the tools: fixed argv, run as the account with nothing else in the env
    let log = w.tools_log();
    let tool = |name: &str| -> String {
        log.lines()
            .find(|l| l.starts_with(&format!("TOOL {name}|")))
            .unwrap_or_else(|| panic!("{name} was not run:\n{log}"))
            .to_string()
    };
    let l = tool("plasma-apply-lookandfeel");
    assert!(l.contains("|--apply org.telamon.dark.desktop|"), "{l}");
    let a = tool("plasma-apply-colorscheme");
    assert!(a.contains("|--accent-color #E93A9A|"), "{a}");
    for line in [&l, &a] {
        assert!(
            line.contains(&format!("uid={uid}|gid={uid}|groups={uid}|")),
            "{line}"
        );
        assert!(
            line.contains(&format!("home={}|", home.display())),
            "{line}"
        );
        let env = line.split("|env=").nth(1).unwrap();
        let keys: Vec<&str> = env
            .split_whitespace()
            .map(|kv| kv.split('=').next().unwrap())
            .collect();
        let mut keys = keys;
        keys.sort_unstable();
        let mut want = vec![
            "TELAMON_WIZARD_TEST_ROOT",
            "HOME",
            "LOGNAME",
            "PATH",
            "QT_QPA_PLATFORM",
            "USER",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
        ];
        want.sort_unstable();
        assert_eq!(keys, want, "the child's whole environment: {env}");
        assert!(
            env.contains("PATH=/usr/bin:/bin ") && env.contains("QT_QPA_PLATFORM=offscreen "),
            "{env}"
        );
        assert!(
            env.contains(&format!("XDG_CONFIG_HOME={}/.config ", home.display())),
            "{env}"
        );
    }
    // and the lock commands, as root
    let c = tool("chage");
    assert!(c.contains("|-E 0 telamon-setup|uid=0|"), "{c}");
    let u = tool("usermod");
    assert!(
        u.contains("|-s /usr/sbin/nologin telamon-setup|uid=0|"),
        "{u}"
    );

    // drop-ins and markers
    let dropin = fs::read_to_string(
        w.root
            .join("etc/plasmalogin.conf.d/50-telamon-autologin.conf"),
    )
    .unwrap();
    assert_eq!(dropin, "[Autologin]\nUser=ada\nSession=plasma\n");
    assert!(
        !w.root
            .join("etc/plasmalogin.conf.d/99-telamon-wizard.conf")
            .exists()
    );
    assert!(w.root.join("etc/telamon/setup-done").exists());
    assert!(w.root.join("etc/atlasos/setup-done").exists());
    assert!(w.root.join("etc/plasma-setup-done").exists());
    assert_eq!(w.state()["finish"], "done");

    // setup is over: every method refuses, except EndSetup, which ends it
    let e = w.create("bob", false).await.unwrap_err();
    assert!(
        err_name(&e, "SetupDone") && has_code(&e, "setup-done"),
        "{e:?}"
    );
    let e = w.finish(HashMap::new()).await.unwrap_err();
    assert!(err_name(&e, "SetupDone"), "{e:?}");
    let e = w.simple("GiveUp").await.unwrap_err();
    assert!(err_name(&e, "SetupDone"), "{e:?}");
    w.simple("EndSetup").await.unwrap();
    assert_eq!(
        w.systemd_log().await,
        vec!["RestartUnit display-manager.service replace".to_string()]
    );

    // the password and the hash are nowhere but shadow
    let helper_log = fs::read_to_string(&w.helper_log).unwrap();
    assert!(helper_log.contains("CreateAccount: done"), "{helper_log}");
    for (p, bytes) in w.all_files() {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(PW), "the password is in {}", p.display());
        if !p.ends_with("etc/shadow") {
            assert!(!text.contains(&hash), "the hash is in {}", p.display());
        }
    }
}

#[tokio::test]
async fn without_polkit_nothing_is_allowed() {
    let Some(w) = World::new(Opts {
        allowed: vec![],
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    let e = w.create("ada", false).await.unwrap_err();
    assert!(
        err_name(&e, "NotAuthorized") && has_code(&e, "polkit-denied"),
        "{e:?}"
    );
    for m in ["EndSetup", "GiveUp"] {
        let e = w.simple(m).await.unwrap_err();
        assert!(err_name(&e, "NotAuthorized"), "{m}: {e:?}");
    }
    let e = w.finish(HashMap::new()).await.unwrap_err();
    assert!(err_name(&e, "NotAuthorized"), "{e:?}");
    assert!(w.accounts_log().await.is_empty());
    assert!(w.systemd_log().await.is_empty());
    assert!(!w.root.join("var/lib/telamon-wizard/state.json").exists());
    assert!(
        w.root
            .join("etc/plasmalogin.conf.d/99-telamon-wizard.conf")
            .exists()
    );
}

#[tokio::test]
async fn only_the_action_polkit_allows_is_allowed() {
    let Some(w) = World::new(Opts {
        allowed: vec!["net.eterneon.telamon.wizard.fallback"],
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    let e = w.create("ada", false).await.unwrap_err();
    assert!(err_name(&e, "NotAuthorized"), "{e:?}");
    w.simple("GiveUp").await.unwrap();
}

#[tokio::test]
async fn a_caller_that_is_not_the_setup_user_is_refused() {
    let Some(w) = World::new(Opts {
        setup_uid: me() + 1,
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    let e = w.create("ada", false).await.unwrap_err();
    assert!(
        err_name(&e, "NotAuthorized") && has_code(&e, "not-setup-user"),
        "{e:?}"
    );
    let e = w.simple("GiveUp").await.unwrap_err();
    assert!(err_name(&e, "NotAuthorized"), "{e:?}");
    assert!(w.accounts_log().await.is_empty());
    assert!(w.systemd_log().await.is_empty());
}

#[tokio::test]
async fn bad_input_never_reaches_accountsservice() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    for (name, pw) in [
        ("Bad Name", PW.as_bytes().to_vec()),
        ("root", PW.as_bytes().to_vec()),
        ("systemd-x", PW.as_bytes().to_vec()),
        ("ada", b"short".to_vec()),
        ("ada", b"ada-ada-ada-ada".to_vec()),
        ("ada", b"password1234".to_vec()),
        (
            "ada",
            vec![0xff, 0xfe, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47],
        ),
        ("ada", vec![b'a'; 600]),
    ] {
        let e = w.create_with(name, pw, false).await.unwrap_err();
        assert!(err_name(&e, "Invalid"), "{name}: {e:?}");
        assert!(!e.1.contains(PW));
    }
    let e = w
        .call(
            "CreateAccount",
            &(
                "ada".to_string(),
                "A:da".to_string(),
                PW.as_bytes().to_vec(),
                false,
            ),
        )
        .await
        .unwrap_err();
    assert!(err_name(&e, "Invalid"), "{e:?}");
    assert!(w.accounts_log().await.is_empty());
    assert!(!w.root.join("var/lib/telamon-wizard/state.json").exists());

    // Finish: nothing to finish, then bad choices
    let e = w.finish(HashMap::new()).await.unwrap_err();
    assert!(has_code(&e, "no-account"), "{e:?}");
    w.create("ada", false).await.unwrap();
    for (k, v) in [
        ("surprise", Value::from(true)),
        ("look", Value::from("neon")),
        ("accent", Value::from("#123456")),
        ("text_scale", Value::from(3.0f64)),
        ("crash_reports", Value::from("yes")),
    ] {
        let mut c = HashMap::new();
        c.insert(k, v);
        let e = w.finish(c).await.unwrap_err();
        assert!(err_name(&e, "Invalid"), "{k}: {e:?}");
    }
    assert!(!w.root.join("etc/telamon/setup-done").exists());
    assert!(!w.root.join("etc/atlasos/setup-done").exists());
    assert!(
        !w.tools_log().contains("TOOL"),
        "no tool ran for refused choices"
    );
}

/// Hostile arguments from the setup user itself (a compromised GUI): huge
/// strings and arrays, a flood of `Finish` keys, variants inside variants.
/// Each is refused with an error, nothing reaches AccountsService or the
/// tools, the helper stays up, and an ordinary call works afterwards.
#[tokio::test]
async fn hostile_arguments_are_refused_and_the_helper_survives() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    let mb = |n: usize| "a".repeat(n * 1024 * 1024);
    // CreateAccount: a 20 MB password, a 20 MB name, a 20 MB full name
    for (name, full, pw) in [
        (
            "ada".to_string(),
            "Ada".to_string(),
            vec![b'x'; 20 * 1024 * 1024],
        ),
        (mb(20), "Ada".to_string(), PW.as_bytes().to_vec()),
        ("ada".to_string(), mb(20), PW.as_bytes().to_vec()),
        (
            "ada\nroot".to_string(),
            "Ada".to_string(),
            PW.as_bytes().to_vec(),
        ),
        ("ada".to_string(), "Ada".to_string(), Vec::new()),
    ] {
        let e = w
            .call("CreateAccount", &(name, full, pw, false))
            .await
            .unwrap_err();
        assert!(err_name(&e, "Invalid"), "{e:?}");
        assert!(e.1.len() < 200, "the answer does not echo the input");
    }
    // Finish: before any account, the argument is still looked at after the
    // account check; so make the account first
    w.create("ada", false).await.unwrap();
    let calls_before = w.accounts_log().await;

    // 300 keys, a 4 MB key, a 20 MB value, a variant 30 deep, a map in a map in a map
    let many: HashMap<String, Value<'_>> = (0..300)
        .map(|i| (format!("k{i}"), Value::from(true)))
        .collect();
    let e = w.call("Finish", &(many,)).await.unwrap_err();
    assert!(
        err_name(&e, "Invalid") && has_code(&e, "choices-bad-value"),
        "{e:?}"
    );

    let mut huge_key: HashMap<String, Value<'_>> = HashMap::new();
    huge_key.insert(mb(4), Value::from(true));
    let e = w.call("Finish", &(huge_key,)).await.unwrap_err();
    assert!(err_name(&e, "Invalid"), "{e:?}");

    let mut huge_value: HashMap<&str, Value<'_>> = HashMap::new();
    huge_value.insert("look", Value::from(mb(20)));
    let e = w.finish(huge_value).await.unwrap_err();
    assert!(err_name(&e, "Invalid"), "{e:?}");

    let mut nested = Value::from("dark");
    for _ in 0..30 {
        nested = Value::Value(Box::new(nested));
    }
    let mut deep: HashMap<&str, Value<'_>> = HashMap::new();
    deep.insert("look", nested);
    let e = w.finish(deep).await.unwrap_err();
    assert!(
        err_name(&e, "Invalid") && has_code(&e, "choices-bad-value"),
        "{e:?}"
    );

    let mut inner: HashMap<&str, Value<'_>> = HashMap::new();
    inner.insert("layout", Value::from("us"));
    let mut mid: HashMap<&str, Value<'_>> = HashMap::new();
    mid.insert("layout", Value::Dict(Dict::from(inner)));
    let mut maps: HashMap<&str, Value<'_>> = HashMap::new();
    maps.insert("keyboard", Value::Dict(Dict::from(mid)));
    let e = w.finish(maps).await.unwrap_err();
    assert!(err_name(&e, "Invalid"), "{e:?}");

    // nothing of that got further than the helper's own checks
    assert_eq!(w.accounts_log().await, calls_before);
    assert!(!w.tools_log().contains("TOOL"), "no tool ran");
    assert!(!w.root.join("etc/telamon/setup-done").exists());
    assert!(w.helper.is_some() && name_owned(&w.conn, BUS_NAME).await);
    // and a normal Finish still works
    w.finish(all_choices()).await.unwrap();
    assert!(w.root.join("etc/telamon/setup-done").exists());
}

#[tokio::test]
async fn a_half_made_account_is_cleaned_up_and_made_again() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    w.accounts_ctl("Fail", &("SetPassword",)).await;
    let e = w.create("ada", false).await.unwrap_err();
    assert!(
        err_name(&e, "AccountsService") && has_code(&e, "accounts-password-failed"),
        "{e:?}"
    );
    assert_eq!(w.state()["account"]["stage"], "created");
    // the account exists, half-made
    assert!(
        fs::read_to_string(w.root.join("etc/passwd"))
            .unwrap()
            .contains("ada:x:")
    );

    let uid = account_uid(); // the half-made one
    w.create("ada", false).await.unwrap();
    assert_eq!(
        w.accounts_log().await,
        vec![
            "CreateUser ada 1".to_string(),
            format!("SetPassword /org/freedesktop/Accounts/User{uid}"),
            format!("DeleteUser {uid} true"),
            "CreateUser ada 1".to_string(),
            format!("SetPassword /org/freedesktop/Accounts/User{}", uid + 1),
        ]
    );
    assert_eq!(w.state()["account"]["stage"], "verified");
}

#[tokio::test]
async fn files_in_a_half_made_home_stop_the_clean_up() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    w.accounts_ctl("Fail", &("SetPassword",)).await;
    w.create("ada", false).await.unwrap_err();
    fs::write(w.root.join("home/ada/precious.txt"), "mine").unwrap();
    let e = w.create("ada", false).await.unwrap_err();
    assert!(
        err_name(&e, "Failed") && has_code(&e, "half-made-has-files"),
        "{e:?}"
    );
    assert!(w.root.join("home/ada/precious.txt").exists());
    assert!(
        !w.accounts_log()
            .await
            .iter()
            .any(|l| l.starts_with("DeleteUser"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_calls_are_serialised() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    w.accounts_ctl("SetDelay", &(0.6f64,)).await;
    let (a, b) = tokio::join!(w.create("ada", false), w.create("bob", false));
    let (ok, bad): (Vec<_>, Vec<_>) = [a, b].into_iter().partition(Result::is_ok);
    assert_eq!((ok.len(), bad.len()), (1, 1));
    let e = bad.into_iter().next().unwrap().unwrap_err();
    assert!(
        err_name(&e, "Invalid") && has_code(&e, "account-exists"),
        "{e:?}"
    );
    let creates = w
        .accounts_log()
        .await
        .iter()
        .filter(|l| l.starts_with("CreateUser"))
        .count();
    assert_eq!(
        creates, 1,
        "the second call waited and saw the first one's account"
    );
}

#[tokio::test]
async fn give_up_hands_over_to_the_fallback() {
    let Some(w) = World::new(Opts::default()).await else {
        return;
    };
    w.simple("GiveUp").await.unwrap();
    assert_eq!(w.state()["gave_up"], true);
    assert!(
        !w.root
            .join("etc/plasmalogin.conf.d/99-telamon-wizard.conf")
            .exists()
    );
    assert_eq!(
        w.systemd_log().await,
        vec!["StartUnit telamon-wizard-fallback.service replace".to_string()]
    );
}

#[tokio::test]
async fn the_helper_exits_when_idle_and_releases_its_name() {
    let Some(mut w) = World::new(Opts {
        idle_ms: 800,
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    w.simple("GiveUp").await.unwrap();
    let start = Instant::now();
    let mut helper = w.helper.take().unwrap();
    let status = loop {
        if let Some(s) = helper.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the helper did not exit when idle"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(status.success(), "{status}");
    assert!(!name_owned(&w.conn, BUS_NAME).await);
}

#[tokio::test]
async fn the_shipped_bus_policy_lets_only_the_setup_user_send() {
    // allowed: the user standing in for telamon-setup reaches the helper
    let Some(w) = World::new(Opts {
        shipped_policy: Some(true),
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    w.simple("GiveUp").await.unwrap();
    drop(w);

    // anyone else: the bus refuses before the helper sees the call
    let Some(w) = World::new(Opts {
        shipped_policy: Some(false),
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    let e = w.simple("GiveUp").await.unwrap_err();
    assert!(
        e.0.ends_with("AccessDenied"),
        "the bus should refuse the call: {e:?}"
    );
    assert!(w.systemd_log().await.is_empty());
    assert!(!w.root.join("var/lib/telamon-wizard/state.json").exists());
}

#[tokio::test]
async fn a_refused_caller_does_not_keep_the_helper_alive() {
    let Some(mut w) = World::new(Opts {
        setup_uid: me() + 1,
        idle_ms: 2000,
        ..Opts::default()
    })
    .await
    else {
        return;
    };
    let start = Instant::now();
    // refused calls, the last one at about half the idle time before the exit
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let e = w.simple("GiveUp").await.unwrap_err();
        assert!(
            err_name(&e, "NotAuthorized") && has_code(&e, "not-setup-user"),
            "{e:?}"
        );
    }
    let mut helper = w.helper.take().unwrap();
    let status = loop {
        if let Some(s) = helper.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_millis(2900),
            "a refused call reset the idle timer"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(status.success(), "{status}");
    // and the helper never asked polkit or did any work for them
    assert!(w.accounts_log().await.is_empty());
}

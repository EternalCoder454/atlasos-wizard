//! Static checks of what the package installs (data/) and of where the
//! programs read their environment: they parse the shipped files and assert
//! the security properties, so a change that loosens one fails CI. They run
//! nothing; the real behaviour is the VM's (docs/DESIGN.md, "The helper").

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A systemd unit file: `section -> key -> every value, in order`.
struct Unit(BTreeMap<String, BTreeMap<String, Vec<String>>>);

impl Unit {
    fn load(rel: &str) -> Unit {
        let mut sections: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
        let mut cur = String::new();
        for line in read(rel).lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
                continue;
            }
            if let Some(s) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                cur = s.to_string();
                sections.entry(cur.clone()).or_default();
            } else if let Some((k, v)) = l.split_once('=') {
                sections
                    .entry(cur.clone())
                    .or_default()
                    .entry(k.trim().to_string())
                    .or_default()
                    .push(v.trim().to_string());
            } else {
                panic!("{rel}: a line that is not key=value: {l:?}");
            }
        }
        Unit(sections)
    }

    fn all(&self, key: &str) -> Vec<&str> {
        self.0
            .values()
            .filter_map(|s| s.get(key))
            .flatten()
            .map(String::as_str)
            .collect()
    }

    fn one(&self, key: &str) -> Option<&str> {
        let v = self.all(key);
        assert!(v.len() <= 1, "{key} is set more than once");
        v.first().copied()
    }

    fn has_section(&self, s: &str) -> bool {
        self.0.contains_key(s)
    }
}

/// The keys every unit of ours sets, and the value each must have.
const COMMON: &[(&str, &str)] = &[
    ("PrivateTmp", "yes"),
    ("ProtectKernelTunables", "yes"),
    ("ProtectKernelModules", "yes"),
    ("ProtectKernelLogs", "yes"),
    ("ProtectControlGroups", "yes"),
    ("ProtectClock", "yes"),
    ("ProtectHostname", "yes"),
    ("LockPersonality", "yes"),
    ("RestrictRealtime", "yes"),
    ("RestrictSUIDSGID", "yes"),
    ("SystemCallArchitectures", "native"),
    ("RestrictAddressFamilies", "AF_UNIX"),
    ("ProtectProc", "invisible"),
    // a password or a hash must never land in a core file
    ("LimitCORE", "0"),
    ("UMask", "0022"),
];

const UNITS: [&str; 3] = [
    "data/systemd/telamon-wizard-boot.service",
    "data/systemd/telamon-wizard-fallback.service",
    "data/systemd/telamon-wizard-helper.service",
];

#[test]
fn every_unit_sets_the_common_sandbox_keys() {
    for u in UNITS {
        let unit = Unit::load(u);
        for (k, v) in COMMON {
            assert_eq!(unit.one(k), Some(*v), "{u}: {k}");
        }
        // anything that undoes the sandbox, or widens what root holds
        for k in [
            "AmbientCapabilities",
            "SecureBits",
            "Environment",
            "EnvironmentFile",
            "PassEnvironment",
            "User",
            "DynamicUser",
            "PermissionsStartOnly",
        ] {
            assert!(unit.one(k).is_none(), "{u}: {k} is set");
        }
        // NoNewPrivileges is left out on purpose (SELinux transitions of
        // chage, usermod, useradd: DESIGN.md); if it is ever set, it is "yes"
        assert!(
            matches!(unit.one("NoNewPrivileges"), None | Some("yes")),
            "{u}"
        );
    }
}

#[test]
fn the_boot_unit_is_a_tight_oneshot() {
    let u = Unit::load(UNITS[0]);
    assert_eq!(u.one("Type"), Some("oneshot"));
    assert_eq!(
        u.one("ExecStart"),
        Some("/usr/libexec/telamon-wizard-boot prepare")
    );
    for (k, v) in [
        ("ProtectSystem", "strict"),
        ("ProtectHome", "yes"),
        ("PrivateDevices", "yes"),
        ("PrivateNetwork", "yes"),
        ("RestrictNamespaces", "yes"),
        ("MemoryDenyWriteExecute", "yes"),
    ] {
        assert_eq!(u.one(k), Some(v), "{k}");
    }
    // ordered before the login screen, off the live image
    let before = u.one("Before").unwrap();
    assert!(before.contains("display-manager.service") && before.contains("plasmalogin.service"));
    assert_eq!(u.one("ConditionKernelCommandLine"), Some("!rd.live.image"));
    // the only places it may write
    let rw = u.one("ReadWritePaths").unwrap();
    let paths: Vec<&str> = rw
        .split_whitespace()
        .map(|p| p.trim_start_matches('-'))
        .collect();
    assert_eq!(
        paths,
        ["/etc", "/run/telamon-setup", "/var/lib/atlas-wizard"]
    );
    assert_eq!(u.one("StateDirectory"), Some("telamon-wizard"));
    assert_eq!(u.one("StateDirectoryMode"), Some("0755"));
}

#[test]
fn the_fallback_unit_owns_the_console_and_is_never_started_at_boot() {
    let u = Unit::load(UNITS[1]);
    assert_eq!(
        u.one("ExecStart"),
        Some("/usr/libexec/telamon-wizard-boot fallback")
    );
    for (k, v) in [
        ("ProtectSystem", "strict"),
        ("PrivateNetwork", "yes"),
        ("RestrictNamespaces", "yes"),
        ("MemoryDenyWriteExecute", "yes"),
        ("StandardInput", "tty"),
        ("TTYPath", "/dev/tty1"),
        ("TTYReset", "yes"),
        ("TTYVHangup", "yes"),
        // diagnostics go to the journal, never onto the console a person
        // is typing a password at
        ("StandardError", "journal"),
    ] {
        assert_eq!(u.one(k), Some(v), "{k}");
    }
    let conflicts = u.one("Conflicts").unwrap();
    assert!(
        conflicts.contains("display-manager.service") && conflicts.contains("getty@tty1.service")
    );
    // not enabled by anything: `prepare` or the helper's GiveUp starts it
    assert!(!u.has_section("Install"));
    // writable: /etc, /var (state, home on an image system), and that is it
    let rw = u.one("ReadWritePaths").unwrap();
    for p in rw.split_whitespace() {
        let p = p.trim_start_matches('-');
        assert!(
            ["/etc", "/var", "/home", "/run/telamon-setup"].contains(&p),
            "{p}"
        );
    }
}

#[test]
fn the_helper_unit_is_a_bus_service_with_a_short_capability_list() {
    let u = Unit::load(UNITS[2]);
    assert_eq!(u.one("Type"), Some("dbus"));
    assert_eq!(u.one("BusName"), Some("net.eterneon.telamon.WizardHelper"));
    assert_eq!(
        u.one("ExecStart"),
        Some("/usr/libexec/telamon-wizard-helper")
    );
    assert_eq!(u.one("ProcSubset"), Some("pid"));
    assert!(matches!(
        u.one("ProtectSystem"),
        Some("yes" | "full" | "strict")
    ));
    let caps = u.one("CapabilityBoundingSet").expect("a capability list");
    let allowed = [
        "CAP_CHOWN",
        "CAP_DAC_OVERRIDE",
        "CAP_FOWNER",
        "CAP_SETUID",
        "CAP_SETGID",
        "CAP_KILL",
    ];
    for c in caps.split_whitespace() {
        assert!(
            allowed.contains(&c),
            "{c} is not one the helper's work needs"
        );
    }
    assert!(caps.split_whitespace().count() >= 5);
    // the bus name is released and the process ends by itself: no Restart
    assert_eq!(u.one("Restart"), None);
}

#[test]
fn only_root_and_the_setup_user_may_send_to_the_helper() {
    let text = read("data/dbus-1/system.d/net.eterneon.telamon.WizardHelper.conf");
    assert!(
        !text.contains("context="),
        "no default or mandatory context"
    );
    assert!(!text.contains("group=") && !text.contains("at_console"));
    assert!(!text.contains("eavesdrop") && !text.contains("<deny"));
    // every <policy ...> block, with its rules
    let mut policies = Vec::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("<policy ") {
        let head_end = rest[i..].find('>').unwrap() + i;
        let head = &rest[i..head_end];
        let close = rest[head_end..].find("</policy>").unwrap() + head_end;
        policies.push((head.to_string(), rest[head_end..close].to_string()));
        rest = &rest[close..];
    }
    let users: Vec<&str> = policies.iter().map(|(h, _)| h.as_str()).collect();
    assert_eq!(
        users,
        ["<policy user=\"telamon-setup\"", "<policy user=\"root\""]
    );
    for (head, body) in &policies {
        let setup = head.contains("telamon-setup");
        for rule in body.split("<allow ").skip(1) {
            let rule = rule.split("/>").next().unwrap();
            if rule.contains("own=") {
                assert!(!setup, "the setup user may not own the name");
                assert!(rule.contains("own=\"net.eterneon.telamon.WizardHelper\""));
            } else {
                assert!(
                    rule.contains("send_destination=\"net.eterneon.telamon.WizardHelper\""),
                    "{rule}"
                );
                if setup {
                    // an interface is always named for the setup user
                    assert!(rule.contains("send_interface="), "{rule}");
                }
            }
        }
    }
    let setup_body = &policies[0].1;
    for iface in setup_body.split("send_interface=\"").skip(1) {
        let iface = iface.split('"').next().unwrap();
        assert!(
            [
                "net.eterneon.telamon.WizardHelper1",
                "org.freedesktop.DBus.Introspectable",
                "org.freedesktop.DBus.Properties",
                "org.freedesktop.DBus.Peer",
            ]
            .contains(&iface),
            "{iface}"
        );
    }
}

#[test]
fn every_polkit_action_defaults_to_no() {
    let text = read("data/polkit-1/actions/net.eterneon.telamon.wizard.policy");
    let ids: Vec<&str> = text
        .split("<action id=\"")
        .skip(1)
        .map(|a| a.split('"').next().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "net.eterneon.telamon.wizard.create-account",
            "net.eterneon.telamon.wizard.finish",
            "net.eterneon.telamon.wizard.fallback",
        ]
    );
    for tag in ["allow_any", "allow_inactive", "allow_active"] {
        let values: Vec<&str> = text
            .split(&format!("<{tag}>"))
            .skip(1)
            .map(|a| a.split('<').next().unwrap())
            .collect();
        assert_eq!(values, ["no"; 3], "{tag}");
    }
    assert!(
        !text.contains("<annotate"),
        "no implied or GUI-authenticated action"
    );
}

#[test]
fn the_polkit_rule_grants_a_fixed_list_to_the_setup_user_only() {
    let js = read("data/polkit-1/rules.d/50-telamon-wizard.rules");
    // one grant, one refusal, nothing interactive
    assert_eq!(js.matches("polkit.Result.YES").count(), 1);
    assert_eq!(js.matches("polkit.Result.NO").count(), 1);
    assert!(!js.contains("AUTH") && !js.contains("polkit.Result.NOT_HANDLED"));
    assert!(js.contains("subject.user == \"telamon-setup\" && subject.local && subject.active"));
    // the marker check: absolute `test`, a failure to run or a marker is "no"
    assert_eq!(js.matches("polkit.spawn(").count(), 1);
    assert!(js.contains("polkit.spawn([\"/usr/bin/test\", \"-d\""));
    assert!(js.contains("catch (e) {\n            return false;"));
    for m in [
        "\"/etc/telamon/setup-done\"",
        "\"/etc/atlasos/setup-done\"",
        "\"/etc/plasma-setup-done\"",
    ] {
        assert!(js.contains(m), "{m}");
    }
    // the granted list: ours and the stock actions the pages use, no others
    let start = js.find("var granted = [").unwrap();
    let end = js[start..].find("];").unwrap() + start;
    let granted: Vec<&str> = js[start..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .flat_map(|l| l.split('"').skip(1).step_by(2))
        .collect();
    assert_eq!(
        granted,
        [
            "net.eterneon.telamon.wizard.create-account",
            "net.eterneon.telamon.wizard.finish",
            "net.eterneon.telamon.wizard.fallback",
            "org.freedesktop.locale1.set-locale",
            "org.freedesktop.locale1.set-keyboard",
            "org.freedesktop.timedate1.set-timezone",
            "org.freedesktop.hostname1.set-static-hostname",
            "org.freedesktop.hostname1.set-hostname",
            "org.freedesktop.NetworkManager.settings.modify.system",
            "org.freedesktop.NetworkManager.network-control",
        ]
    );
    // "finish" alone skips the marker check, and only that action
    assert_eq!(js.matches("action.id ==").count(), 1);
    assert!(js.contains(
        "action.id == \"net.eterneon.telamon.wizard.finish\" || telamonWizardSetupOpen()"
    ));
}

#[test]
fn the_setup_user_and_its_directories_are_as_documented() {
    let sysusers: Vec<String> = read("data/sysusers.d/telamon-wizard.conf")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    // a system user (dynamic id), a login shell for the session, the home on
    // /run, no extra groups; sysusers locks the password itself
    assert_eq!(
        sysusers,
        ["u telamon-setup - \"Telamon Setup\" /run/telamon-setup /bin/sh"]
    );
    let tmpfiles: Vec<Vec<String>> = read("data/tmpfiles.d/telamon-wizard.conf")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().map(String::from).collect())
        .collect();
    let rows: BTreeMap<&str, (&str, &str, &str, &str)> = tmpfiles
        .iter()
        .map(|f| {
            (
                f[1].as_str(),
                (f[0].as_str(), f[2].as_str(), f[3].as_str(), f[4].as_str()),
            )
        })
        .collect();
    assert_eq!(rows.len(), tmpfiles.len(), "a path twice");
    assert_eq!(
        rows["/run/telamon-setup"],
        ("d", "0750", "telamon-setup", "telamon-setup")
    );
    assert_eq!(
        rows["/var/lib/telamon-wizard"],
        ("d", "0755", "root", "root")
    );
    assert_eq!(rows["/etc/telamon"], ("d", "0755", "root", "root"));
    assert_eq!(rows["/etc/atlasos"], ("d", "0755", "root", "root"));
    assert_eq!(rows.len(), 4);
    for (p, (kind, mode, user, _)) in &rows {
        // `d` adjusts an existing directory too; nothing links, writes or is writable by others
        assert_eq!(*kind, "d", "{p}");
        let m = u32::from_str_radix(mode, 8).unwrap();
        assert_eq!(m & 0o022, 0, "{p} is writable by group or others");
        if !p.starts_with("/run") {
            assert_eq!(*user, "root", "{p}");
        }
    }
}

#[test]
fn the_session_is_only_for_the_autologin() {
    let d = read("data/wayland-sessions/telamon-wizard.desktop");
    assert!(d.contains("\nNoDisplay=true\n"));
    assert!(d.contains("\nExec=/usr/libexec/telamon-wizard-session\n"));
    // `Hidden=true` would disable the entry for autologin too, so it is not used
    assert!(!d.lines().any(|l| l.trim() == "Hidden=true"));
    let app = read("apps/telamon-wizard/data/net.eterneon.telamon.wizard.desktop");
    assert!(app.contains("\nNoDisplay=true\n") && app.contains("\nTerminal=false\n"));
}

#[test]
fn the_session_script_runs_fixed_programs_on_its_own_files() {
    let sh = read("data/libexec/telamon-wizard-session");
    assert!(sh.starts_with("#!/bin/sh\n"));
    let code: String = sh
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    for banned in [
        "eval ", "source ", "sudo", "pkexec", "curl", "wget", "chmod", "/tmp/", "bash",
    ] {
        assert!(!code.contains(banned), "the session script uses {banned:?}");
    }
    // a password or a core file of the wizard must not end up on disk: the
    // wizard holds the typed password in memory
    let ulimit = sh.find("ulimit -c 0").expect("core dumps are switched off");
    let kwin = sh.find("/usr/bin/kwin_wayland").unwrap();
    assert!(ulimit < kwin, "before KWin and the wizard start");
    // the helper is addressed by its fixed name, and only these two calls
    let calls: Vec<&str> = sh
        .split("net.eterneon.telamon.WizardHelper1 ")
        .skip(1)
        .map(|r| r.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(calls, ["GiveUp", "EndSetup"]);
    // its counters live in the setup user's own directory
    assert!(sh.contains("dir=/run/telamon-setup\n"));
    for line in sh
        .lines()
        .filter(|l| l.contains("$counter") || l.contains("$tries_file"))
    {
        assert!(!line.contains("rm "), "{line}");
    }
}

/// The program sources that read the environment, and how many times. A new
/// read in code that runs as root (or in the settings child) is a decision
/// to review, so it must be added here with the review.
const ENV_READS: &[(&str, usize)] = &[
    // test-root builds only (helper: behind a const that is false without the
    // feature; boot: behind `#[cfg(feature = "test-root")]`)
    ("crates/wizard-helper/src/paths.rs", 1),
    ("crates/wizard-boot/src/paths.rs", 1),
    ("crates/wizard-boot/src/main.rs", 2),
    // the settings child: its own HOME, set by the helper
    ("crates/wizard-helper/src/apply.rs", 1),
    // the GUI (runs as the setup user or the new user, never as root)
    ("apps/telamon-wizard/src/backend.rs", 2),
    ("apps/telamon-wizard/src/system.rs", 2),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The source of `file` before its first `#[cfg(test)]` module.
fn production_part(text: &str) -> &str {
    text.split("\n#[cfg(test)]").next().unwrap()
}

/// Every `log::...!(...)` statement of `text`, from its first line to the
/// `);` that ends it, with the line number it starts on.
fn log_statements(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, String)> = None;
    for (n, line) in text.lines().enumerate() {
        let l = line.trim_start();
        if cur.is_none() && l.starts_with("log::") {
            cur = Some((n + 1, String::new()));
        }
        if let Some((_, acc)) = cur.as_mut() {
            acc.push_str(l);
            acc.push(' ');
            if l.ends_with(");") || l.ends_with("),") {
                out.push(cur.take().unwrap());
            }
        }
    }
    out
}

#[test]
fn the_environment_is_read_in_known_places_only() {
    let mut files = Vec::new();
    for d in ["crates", "apps"] {
        rust_files(&repo().join(d), &mut files);
    }
    let mut got: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        let rel = f
            .strip_prefix(repo())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if rel.contains("/target/") || rel.contains("/tests/") || rel.ends_with("_tests.rs") {
            continue;
        }
        let text = fs::read_to_string(&f).unwrap();
        let n: usize = production_part(&text)
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(|l| {
                ["env::var(", "env::var_os(", "env::vars", "getenv"]
                    .iter()
                    .map(|pat| l.matches(pat).count())
                    .sum::<usize>()
            })
            .sum();
        if n > 0 {
            got.insert(rel, n);
        }
    }
    let want: BTreeMap<String, usize> = ENV_READS
        .iter()
        .map(|(f, n)| ((*f).to_string(), *n))
        .collect();
    assert_eq!(
        got, want,
        "the places that read the environment changed: review the new read, then update ENV_READS"
    );
}

#[test]
fn test_hooks_exist_only_behind_the_test_root_feature() {
    // every file that names a TELAMON_WIZARD_TEST_ variable outside a test
    // module must be gated by the feature (or the const that follows it)
    let mut files = Vec::new();
    rust_files(&repo().join("crates"), &mut files);
    for f in files {
        let rel = f
            .strip_prefix(repo())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if rel.contains("/tests/") || rel.ends_with("_tests.rs") || rel.ends_with("fake.rs") {
            continue;
        }
        let text = fs::read_to_string(&f).unwrap();
        let prod = production_part(&text);
        let code: String = prod
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        if code.contains("TELAMON_WIZARD_TEST") {
            assert!(
                code.contains("feature = \"test-root\"") || code.contains("TEST_ROOT_ENABLED"),
                "{rel} names a test hook without the test-root gate"
            );
        }
    }
    // and no crate turns the feature on for a normal build
    for m in [
        "crates/wizard-boot/Cargo.toml",
        "crates/wizard-helper/Cargo.toml",
    ] {
        let t = read(m);
        let normal = t.split("[dev-dependencies]").next().unwrap();
        assert!(!normal.contains("features = [\"test-root\"]"), "{m}");
        assert!(
            !t.lines()
                .any(|l| l.trim_start().starts_with("default") && l.contains("test-root")),
            "{m}"
        );
        assert!(
            t.contains("test-root = []"),
            "{m} defines the feature without any dependency"
        );
    }
    for l in [
        "crates/wizard-boot/src/lib.rs",
        "crates/wizard-helper/src/lib.rs",
    ] {
        assert!(
            read(l).contains("compile_error!(\"the test-root feature is for test builds only"),
            "{l}: a release build with the feature must not compile"
        );
    }
}

#[test]
fn programs_that_hold_a_password_turn_core_dumps_off_before_they_work() {
    // the helper: before it reads its arguments, for the service and the child
    let helper = read("crates/wizard-helper/src/main.rs");
    let dump = helper.find("set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)");
    let dispatch = helper.find("match args");
    assert!(dump.is_some() && dispatch.is_some() && dump < dispatch);
    assert!(
        helper.contains("return ExitCode::FAILURE"),
        "it refuses to run without"
    );
    // the fallback: before the terminal is read
    let boot = read("crates/wizard-boot/src/main.rs");
    let fb = boot.find("\"fallback\" =>").unwrap();
    let dump = boot[fb..].find("DumpableBehavior::NotDumpable").unwrap();
    let tty = boot[fb..].find("Tty::new()").unwrap();
    assert!(dump < tty);
    // and the session that holds the typed password in the GUI sets the limit
    // (checked in the session script test)
}

#[test]
fn the_helper_never_logs_a_user_supplied_string() {
    // the helper's log lines carry codes, counts, uids and fixed text: a name
    // or full name from the caller must not reach the journal (it is the one
    // log a local reader can see). `acc.name` in the half-made clean-up comes
    // from the state file, which only root writes.
    let mut seen = 0;
    for f in [
        "crates/wizard-helper/src/service.rs",
        "crates/wizard-helper/src/core.rs",
    ] {
        let text = read(f);
        for (n, l) in log_statements(production_part(&text)) {
            seen += 1;
            for var in [
                "full_name",
                "password",
                "hash",
                "{name}",
                "{full",
                "choices",
            ] {
                assert!(!l.contains(var), "{f}:{n}: {l}");
            }
        }
    }
    assert!(
        seen > 30,
        "found only {seen} log statements: the scan is broken"
    );
}

#[test]
fn the_gui_logs_no_password_ssid_or_full_name() {
    // what the journal of the setup session may hold: the user name and
    // codes. `ssid`, `full`, `password`, `psk` in a log statement would put
    // them there.
    let mut seen = 0;
    for f in [
        "apps/telamon-wizard/src/backend.rs",
        "apps/telamon-wizard/src/system.rs",
        "apps/telamon-wizard/src/answers.rs",
    ] {
        let text = read(f);
        for (n, l) in log_statements(production_part(&text)) {
            seen += 1;
            for var in ["ssid", "full", "password", "{pw", "psk", "wifi_pw"] {
                assert!(!l.contains(var), "{f}:{n}: {l}");
            }
        }
    }
    assert!(
        seen > 8,
        "found only {seen} log statements: the scan is broken"
    );
}

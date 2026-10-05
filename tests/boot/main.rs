//! Runs the real `atlas-wizard-boot` (built with the `test-root` feature, see
//! the crate's Cargo.toml) against temporary roots: one test per row of
//! DESIGN.md's decision table, and the cases a power cut leaves behind.
//! Commands are never run: the binary's fake runner records them in
//! `<root>/commands.log` and edits passwd, shadow and group under the root.

mod fallback;
mod prepare;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const BIN: &str = env!("CARGO_BIN_EXE_atlas-wizard-boot");

pub const DROPIN: &str = "[Autologin]\nUser=atlas-setup\nSession=atlas-wizard\nRelogin=true\n";

/// A temporary root with the files of a fresh, un-set-up machine.
pub struct Sys {
    pub dir: tempfile::TempDir,
}

pub fn is_root() -> bool {
    rustix::process::getuid().is_root()
}

impl Default for Sys {
    fn default() -> Self {
        Sys::new()
    }
}

impl Sys {
    pub fn new() -> Sys {
        let dir = tempfile::tempdir().unwrap();
        let s = Sys { dir };
        s.write(
            "etc/passwd",
            "root:x:0:0:root:/root:/bin/bash\natlas-setup:x:975:975:AtlasOS Setup:/run/atlas-setup:/bin/sh\n",
        );
        s.write(
            "etc/shadow",
            "root:!:19000::::::\natlas-setup:!*:19000:0:99999:7:::\n",
        );
        s.write("etc/group", "root:x:0:\nwheel:x:10:\natlas-setup:x:975:\n");
        s.write("proc/cmdline", "BOOT_IMAGE=/vmlinuz root=/dev/sda2 quiet\n");
        s
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    pub fn write(&self, rel: &str, text: &str) {
        let p = self.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap_or_default()
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    pub fn cmdline(&self, extra: &str) {
        self.write("proc/cmdline", &format!("quiet {extra}\n"));
    }

    pub fn state(&self, json: &str) {
        self.write("var/lib/atlas-wizard/state.json", json);
    }

    pub fn state_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.read("var/lib/atlas-wizard/state.json")).unwrap_or_default()
    }

    /// A human account with a usable hash and a home.
    pub fn add_human(&self, name: &str, uid: u32) {
        let mut p = self.read("etc/passwd");
        p.push_str(&format!(
            "{name}:x:{uid}:{uid}:{name}:/home/{name}:/bin/bash\n"
        ));
        self.write("etc/passwd", &p);
        let mut s = self.read("etc/shadow");
        s.push_str(&format!("{name}:$y$j9T$salt$hash:19000:0:99999:7:::\n"));
        self.write("etc/shadow", &s);
        let g = self
            .read("etc/group")
            .replace("wheel:x:10:", &format!("wheel:x:10:{name}"));
        self.write("etc/group", &g);
        fs::create_dir_all(self.path(&format!("home/{name}"))).unwrap();
        let _ = std::os::unix::fs::chown(self.path(&format!("home/{name}")), Some(uid), Some(uid));
    }

    /// Both done markers, as the wizard writes them.
    pub fn markers(&self) {
        self.write("etc/atlasos/setup-done", "[Setup]\nVersion=1\n");
        self.write("etc/plasma-setup-done", "done\n");
    }

    pub fn lock_atlas_setup(&self) {
        self.write(
            "etc/passwd",
            &self
                .read("etc/passwd")
                .replace(":/bin/sh\n", ":/usr/sbin/nologin\n"),
        );
        self.write(
            "etc/shadow",
            &self.read("etc/shadow").replace(
                "atlas-setup:!*:19000:0:99999:7:::",
                "atlas-setup:!*:19000:0:99999:7::0:",
            ),
        );
    }

    pub fn commands(&self) -> Vec<String> {
        self.read("commands.log")
            .lines()
            .map(String::from)
            .collect()
    }

    pub fn command(&self, sub: &str) -> Command {
        let mut c = Command::new(BIN);
        c.arg(sub)
            .env("ATLAS_WIZARD_TEST_ROOT", self.dir.path())
            .env_remove("ATLAS_WIZARD_TEST_FAIL");
        c
    }

    pub fn prepare(&self) -> Output {
        let out = self.command("prepare").output().unwrap();
        assert!(
            out.status.success(),
            "prepare must always exit 0: {:?}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    pub fn dropin(&self) -> Option<String> {
        let p = self.path("etc/plasmalogin.conf.d/99-atlas-wizard.conf");
        p.exists().then(|| fs::read_to_string(p).unwrap())
    }

    pub fn marker_state(&self) -> (bool, bool) {
        (
            self.exists("etc/atlasos/setup-done"),
            self.exists("etc/plasma-setup-done"),
        )
    }

    pub fn files_under(&self, rel: &str) -> Vec<String> {
        let dir: &Path = &self.path(rel);
        let mut v: Vec<String> = fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

pub const LOCK_CMDS: [&str; 2] = [
    "/usr/bin/chage -E 0 atlas-setup",
    "/usr/sbin/usermod -s /usr/sbin/nologin atlas-setup",
];
pub const FALLBACK_CMD: &str = "/usr/bin/systemctl start --no-block atlas-wizard-fallback.service";
pub const DM_CMD: &str = "/usr/bin/systemctl start --no-block display-manager.service";

#[test]
fn wrong_arguments_exit_2_and_touch_nothing() {
    let s = Sys::new();
    for args in [vec![], vec!["bogus"], vec!["prepare", "extra"]] {
        let out = Command::new(BIN)
            .args(&args)
            .env("ATLAS_WIZARD_TEST_ROOT", s.dir.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
    assert!(s.commands().is_empty());
    assert!(!s.exists("var"));
}

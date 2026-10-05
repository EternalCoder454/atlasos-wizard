//! The command runner of the tests: records every call and, for the account
//! programs, does what they would do to the passwd, shadow and group files
//! under the test root. Compiled only for tests and the `test-root` feature.

use crate::cmd::{CmdError, Runner};
use crate::paths::Paths;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// A recording, file-editing stand-in for the real programs.
pub struct Fake {
    paths: Paths,
    calls: Mutex<Vec<String>>,
    failing: Vec<String>,
}

fn base(prog: &str) -> &str {
    prog.rsplit('/').next().unwrap_or(prog)
}

impl Fake {
    /// A fake that edits files under `paths`' root.
    pub fn new(paths: &Paths) -> Fake {
        Fake {
            paths: paths.clone(),
            calls: Mutex::new(Vec::new()),
            failing: Vec::new(),
        }
    }

    /// Makes every call of the program with this base name fail (after
    /// being recorded, without effect).
    #[must_use]
    pub fn failing(mut self, prog: &str) -> Fake {
        self.failing.push(prog.to_string());
        self
    }

    /// Makes the programs named in `list` (comma separated) fail.
    #[must_use]
    pub fn failing_list(mut self, list: &str) -> Fake {
        self.failing
            .extend(list.split(',').filter(|s| !s.is_empty()).map(String::from));
        self
    }

    /// The calls so far, one `prog arg arg` string each.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn lines(&self, file: PathBuf) -> Vec<String> {
        fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn put(&self, file: PathBuf, lines: &[String]) {
        let mut text = lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        let _ = fs::create_dir_all(file.parent().unwrap_or(self.paths.root()));
        let _ = fs::write(file, text);
    }

    fn useradd(&self, args: &[&str]) {
        let Some(name) = args.last() else { return };
        let full = args
            .iter()
            .position(|a| *a == "-c")
            .and_then(|i| args.get(i + 1))
            .copied()
            .unwrap_or("");
        let mut passwd = self.lines(self.paths.passwd());
        let uid = passwd
            .iter()
            .filter_map(|l| l.split(':').nth(2)?.parse::<u32>().ok())
            .filter(|u| *u < 60000)
            .max()
            .map_or(1000, |m| (m + 1).max(1000));
        passwd.push(format!(
            "{name}:x:{uid}:{uid}:{full}:/home/{name}:/bin/bash"
        ));
        self.put(self.paths.passwd(), &passwd);
        let mut shadow = self.lines(self.paths.shadow());
        shadow.push(format!("{name}:!:19000:0:99999:7:::"));
        self.put(self.paths.shadow(), &shadow);
        let mut group = self.lines(self.paths.group());
        group.push(format!("{name}:x:{uid}:"));
        if let Some(w) = group.iter_mut().find(|l| l.starts_with("wheel:")) {
            if w.ends_with(':') {
                w.push_str(name);
            } else {
                w.push_str(&format!(",{name}"));
            }
        } else {
            group.push(format!("wheel:x:10:{name}"));
        }
        self.put(self.paths.group(), &group);
        let home = self.paths.under_root(&format!("/home/{name}"));
        let _ = fs::create_dir_all(&home);
        // Ownership only works as root (the container); the tests that
        // verify the home run there.
        let _ = std::os::unix::fs::chown(&home, Some(uid), Some(uid));
    }

    fn chpasswd(&self, stdin: Option<&[u8]>) {
        let Some(text) = stdin.and_then(|s| std::str::from_utf8(s).ok()) else {
            return;
        };
        let _ = fs::write(self.paths.root().join("chpasswd.stdin"), text);
        let Some((user, hash)) = text.trim_end().split_once(':') else {
            return;
        };
        let mut shadow = self.lines(self.paths.shadow());
        for l in &mut shadow {
            if l.split(':').next() == Some(user) {
                let mut f: Vec<&str> = l.split(':').collect();
                f[1] = hash;
                *l = f.join(":");
            }
        }
        self.put(self.paths.shadow(), &shadow);
    }

    fn userdel(&self, args: &[&str]) {
        let Some(name) = args.last() else { return };
        let mut home = None;
        let mut passwd = self.lines(self.paths.passwd());
        passwd.retain(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let hit = f.first() == Some(name);
            if hit {
                home = f.get(5).map(|h| h.to_string());
            }
            !hit
        });
        self.put(self.paths.passwd(), &passwd);
        for (file, member) in [(self.paths.shadow(), false), (self.paths.group(), true)] {
            let mut lines = self.lines(file.clone());
            lines.retain(|l| l.split(':').next() != Some(name));
            if member {
                for l in &mut lines {
                    let f: Vec<&str> = l.split(':').collect();
                    if f.len() == 4 && f[3].split(',').any(|m| m == *name) {
                        let kept: Vec<&str> = f[3].split(',').filter(|m| m != name).collect();
                        *l = format!("{}:{}:{}:{}", f[0], f[1], f[2], kept.join(","));
                    }
                }
            }
            self.put(file, &lines);
        }
        if args.contains(&"-r")
            && let Some(h) = home
        {
            let _ = fs::remove_dir_all(self.paths.under_root(&h));
        }
    }

    fn chage(&self, args: &[&str]) {
        let (Some(name), true) = (args.last(), args.contains(&"0")) else {
            return;
        };
        let mut shadow = self.lines(self.paths.shadow());
        for l in &mut shadow {
            if l.split(':').next() == Some(name) {
                let mut f: Vec<String> = l.split(':').map(String::from).collect();
                while f.len() < 9 {
                    f.push(String::new());
                }
                f[7] = "0".into();
                *l = f.join(":");
            }
        }
        self.put(self.paths.shadow(), &shadow);
    }

    fn usermod(&self, args: &[&str]) {
        let (Some(name), Some(shell)) = (args.last(), args.get(1)) else {
            return;
        };
        let mut passwd = self.lines(self.paths.passwd());
        for l in &mut passwd {
            if l.split(':').next() == Some(name) {
                let mut f: Vec<&str> = l.split(':').collect();
                if f.len() >= 7 {
                    f[6] = shell;
                }
                *l = f.join(":");
            }
        }
        self.put(self.paths.passwd(), &passwd);
    }
}

impl Runner for Fake {
    fn run(&self, prog: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<(), CmdError> {
        let line = std::iter::once(prog)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        if let Ok(mut c) = self.calls.lock() {
            c.push(line.clone());
        }
        let _ = fs::create_dir_all(self.paths.root());
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.command_log())
        {
            let _ = writeln!(f, "{line}");
        }
        let name = base(prog);
        if self.failing.iter().any(|f| f == name) {
            return Err(CmdError::Exit(Some(1), "fake failure".into()));
        }
        match name {
            "useradd" => self.useradd(args),
            "chpasswd" => self.chpasswd(stdin),
            "userdel" => self.userdel(args),
            "chage" => self.chage(args),
            "usermod" => self.usermod(args),
            _ => {}
        }
        Ok(())
    }
}

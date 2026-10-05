//! `/var/lib/atlas-wizard/state.json` (DESIGN.md, State and recovery):
//! root-owned, mode 0644, no secrets, written atomically.

use crate::fsutil::write_atomic;
use crate::timefmt::unix_secs;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where the state lives.
pub const DEFAULT_PATH: &str = "/var/lib/atlas-wizard/state.json";

/// `finish` value: Finish has reached the step that writes the done markers.
pub const FINISH_MARKERS: &str = "markers";

/// The state file mode.
pub const MODE: u32 = 0o644;

/// Largest state file read; a bigger one is treated as unparsable.
const MAX_BYTES: u64 = 1024 * 1024;

fn format_one() -> u32 {
    1
}

/// How far the account got. Written before and after each helper step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// AccountsService `CreateUser` is running.
    Creating,
    /// The user exists.
    Created,
    /// The password hash is set.
    PasswordSet,
    /// passwd, shadow, wheel and home all checked.
    Verified,
}

impl Stage {
    /// True for the stages that leave a half-made account.
    pub fn is_half_made(self) -> bool {
        self != Stage::Verified
    }
}

/// The account the wizard made (or started to make).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// User name.
    pub name: String,
    /// Numeric uid.
    pub uid: u32,
    /// How far creation got.
    pub stage: Stage,
}

/// The state file. Unknown fields are kept through a load/save round trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// File format; a missing field means 1, a higher one is read as 1.
    #[serde(default = "format_one")]
    pub format: u32,
    /// Boots that started the wizard.
    #[serde(default)]
    pub boots: u32,
    /// The session failed too often and the helper's `GiveUp` ran.
    #[serde(default)]
    pub gave_up: bool,
    /// The account, once creation started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
    /// How far Finish got (see [`FINISH_MARKERS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    /// Fields this version does not know.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for State {
    fn default() -> Self {
        State {
            format: 1,
            boots: 0,
            gave_up: false,
            account: None,
            finish: None,
            extra: Map::new(),
        }
    }
}

/// Something worth a journal warning that [`load`] recovered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The file could not be parsed; it was moved aside and the default used.
    Unparsable {
        /// What the parser said.
        error: String,
        /// Where the bad file went; `None` when the rename failed too.
        moved_to: Option<PathBuf>,
    },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Warning::Unparsable { error, moved_to } = self;
        match moved_to {
            Some(p) => write!(
                f,
                "state file unparsable ({error}); moved to {}",
                p.display()
            ),
            None => write!(
                f,
                "state file unparsable ({error}); could not move it aside"
            ),
        }
    }
}

/// A loaded state and the warning, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The state (the default when the file was missing or bad).
    pub state: State,
    /// Set when the file was bad.
    pub warning: Option<Warning>,
}

/// Loads the state. A missing file gives the default; an unparsable one is
/// renamed to `state.json.bad-<unix time>` and gives the default plus a
/// [`Warning`].
///
/// # Errors
/// Only a real read error (permissions, I/O), where guessing would be unsafe.
pub fn load(path: &Path) -> io::Result<Loaded> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Loaded {
                state: State::default(),
                warning: None,
            });
        }
        Err(e) => return Err(e),
    };
    let mut buf = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut buf)?;
    let parsed = if buf.len() as u64 > MAX_BYTES {
        Err("file too large".to_string())
    } else {
        serde_json::from_slice::<State>(&buf).map_err(|e| e.to_string())
    };
    match parsed {
        Ok(state) => Ok(Loaded {
            state,
            warning: None,
        }),
        Err(error) => {
            let moved_to = move_aside(path);
            Ok(Loaded {
                state: State::default(),
                warning: Some(Warning::Unparsable { error, moved_to }),
            })
        }
    }
}

fn move_aside(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let secs = unix_secs(SystemTime::now());
    for n in 0..100u32 {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{n}")
        };
        let dest = path.with_file_name(format!("{name}.bad-{secs}{suffix}"));
        if dest.exists() {
            continue;
        }
        return fs::rename(path, &dest).ok().map(|()| dest);
    }
    None
}

impl State {
    /// Writes the state atomically (temp file in the same directory, fsync,
    /// rename, directory fsync) with mode 0644.
    ///
    /// # Errors
    /// Any I/O error; the old file is left as it was and no temp remains.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut json = serde_json::to_vec(self).map_err(io::Error::other)?;
        json.push(b'\n');
        write_atomic(path, &json, MODE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn missing_is_default() {
        let d = dir();
        let l = load(&d.path().join("state.json")).unwrap();
        assert_eq!(l.state, State::default());
        assert_eq!(l.warning, None);
    }

    #[test]
    fn design_example_parses() {
        let d = dir();
        let p = d.path().join("state.json");
        fs::write(
            &p,
            r#"{"format": 1, "boots": 2, "gave_up": false,
 "account": {"name": "ada", "uid": 1000, "stage": "verified"},
 "finish": "markers"}"#,
        )
        .unwrap();
        let s = load(&p).unwrap().state;
        assert_eq!(s.boots, 2);
        assert_eq!(
            s.account,
            Some(Account {
                name: "ada".into(),
                uid: 1000,
                stage: Stage::Verified
            })
        );
        assert_eq!(s.finish.as_deref(), Some(FINISH_MARKERS));
        assert!(s.extra.is_empty());
    }

    #[test]
    fn all_stage_names() {
        for (n, st) in [
            ("creating", Stage::Creating),
            ("created", Stage::Created),
            ("password-set", Stage::PasswordSet),
            ("verified", Stage::Verified),
        ] {
            let j = format!(r#"{{"name":"a","uid":1,"stage":"{n}"}}"#);
            assert_eq!(serde_json::from_str::<Account>(&j).unwrap().stage, st);
        }
        assert!(Stage::Created.is_half_made() && !Stage::Verified.is_half_made());
    }

    #[test]
    fn empty_object_and_missing_fields() {
        let s: State = serde_json::from_str("{}").unwrap();
        assert_eq!(s, State::default());
    }

    #[test]
    fn higher_format_and_unknown_fields_round_trip() {
        let d = dir();
        let p = d.path().join("state.json");
        fs::write(
            &p,
            r#"{"format":7,"boots":1,"future":{"a":[1,2]},"note":"x"}"#,
        )
        .unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.warning, None);
        assert_eq!(l.state.format, 7);
        l.state.save(&p).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        assert_eq!(v["format"], 7);
        assert_eq!(v["future"]["a"][1], 2);
        assert_eq!(v["note"], "x");
        assert_eq!(v["boots"], 1);
    }

    #[test]
    fn save_load_round_trip_with_mode() {
        let d = dir();
        let p = d.path().join("var/lib/state.json");
        let s = State {
            boots: 3,
            gave_up: true,
            account: Some(Account {
                name: "ada".into(),
                uid: 1001,
                stage: Stage::PasswordSet,
            }),
            finish: Some("markers".into()),
            ..State::default()
        };
        s.save(&p).unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(load(&p).unwrap().state, s);
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains(r#""stage":"password-set""#), "{text}");
    }

    #[test]
    fn unparsable_is_moved_aside() {
        for bad in [
            "not json",
            "[1,2]",
            r#"{"boots":"many"}"#,
            r#"{"account":{"stage":"weird","name":"a","uid":1}}"#,
            "",
        ] {
            let d = dir();
            let p = d.path().join("state.json");
            fs::write(&p, bad).unwrap();
            let l = load(&p).unwrap();
            assert_eq!(l.state, State::default(), "{bad}");
            let Some(Warning::Unparsable {
                moved_to: Some(to), ..
            }) = l.warning.clone()
            else {
                panic!("{bad}: {:?}", l.warning);
            };
            assert!(!p.exists());
            assert!(
                to.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("state.json.bad-")
            );
            assert_eq!(fs::read_to_string(&to).unwrap(), bad);
            assert!(l.warning.unwrap().to_string().contains("moved to"));
        }
    }

    #[test]
    fn repeated_bad_files_do_not_overwrite_each_other() {
        let d = dir();
        let p = d.path().join("state.json");
        let mut moved = Vec::new();
        for i in 0..3 {
            fs::write(&p, format!("bad{i}")).unwrap();
            if let Some(Warning::Unparsable {
                moved_to: Some(t), ..
            }) = load(&p).unwrap().warning
            {
                moved.push(t);
            }
        }
        moved.sort();
        moved.dedup();
        assert_eq!(moved.len(), 3);
    }

    #[test]
    fn oversized_is_unparsable() {
        let d = dir();
        let p = d.path().join("state.json");
        fs::write(&p, vec![b' '; 1_100_000]).unwrap();
        assert!(load(&p).unwrap().warning.is_some());
    }

    #[test]
    fn read_error_is_an_error() {
        let d = dir();
        // A directory opens but cannot be read.
        assert!(load(d.path()).is_err());
    }

    #[test]
    fn failed_save_leaves_old_file_and_no_temp() {
        let d = dir();
        let p = d.path().join("state.json");
        State::default().save(&p).unwrap();
        let before = fs::read(&p).unwrap();
        // Replace the destination by a directory so the rename fails.
        let q = d.path().join("dest");
        fs::create_dir(&q).unwrap();
        assert!(State::default().save(&q).is_err());
        assert_eq!(fs::read(&p).unwrap(), before);
        let tmp: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(tmp.is_empty());
    }

    #[test]
    fn state_holds_no_password_field() {
        let j = serde_json::to_string(&State::default()).unwrap();
        assert!(!j.contains("password"));
    }
}

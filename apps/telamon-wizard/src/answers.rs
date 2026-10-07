//! The answers so far, kept in `/run/telamon-setup/answers.json` so a crashed
//! wizard comes back on the same page. There is no field for a password, so
//! none can reach the file. Written atomically, mode 0600.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use wizard_core::choices::{ACCENTS, Value};
use wizard_core::fsutil::write_atomic;

pub const DEFAULT_PATH: &str = "/run/telamon-setup/answers.json";
/// The file is ours but never trusted: read at most this much.
const MAX_BYTES: u64 = 64 << 10;

/// Everything the pages collect, but the password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Answers {
    /// The page the wizard was on (its id).
    pub step: String,
    pub language: String,
    pub keyboard_layout: String,
    pub keyboard_variant: String,
    pub timezone: String,
    pub hostname: String,
    pub wifi_done: bool,
    pub full_name: String,
    pub user_name: String,
    /// True once the user edited the user name by hand.
    pub user_name_edited: bool,
    pub autologin: bool,
    /// `light` or `dark`.
    pub look: String,
    pub accent: String,
    pub text_scale: f64,
    pub high_contrast: bool,
    pub screen_reader: bool,
    pub crash_reports: bool,
}

impl Default for Answers {
    fn default() -> Self {
        Answers {
            step: String::new(),
            language: String::new(),
            keyboard_layout: String::new(),
            keyboard_variant: String::new(),
            timezone: String::new(),
            hostname: String::new(),
            wifi_done: false,
            full_name: String::new(),
            user_name: String::new(),
            user_name_edited: false,
            autologin: false,
            look: "light".into(),
            accent: ACCENTS[0].into(),
            text_scale: 1.0,
            high_contrast: false,
            screen_reader: false,
            crash_reports: false,
        }
    }
}

impl Answers {
    /// Reads the file; anything missing, too big or unparsable gives the
    /// defaults (logged), never an error: the wizard must still start.
    pub fn load(path: &Path) -> Answers {
        let mut text = String::new();
        let read =
            std::fs::File::open(path).and_then(|f| f.take(MAX_BYTES).read_to_string(&mut text));
        match read {
            Ok(_) => Answers::from_json(&text).unwrap_or_else(|e| {
                log::warn!("{}: {e}; starting with the defaults", path.display());
                Answers::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Answers::default(),
            Err(e) => {
                log::warn!("{}: {e}; starting with the defaults", path.display());
                Answers::default()
            }
        }
    }

    /// Parses and clamps: values the helper would refuse are replaced.
    pub fn from_json(text: &str) -> Result<Answers, serde_json::Error> {
        let mut a: Answers = serde_json::from_str(text)?;
        if a.look != "light" && a.look != "dark" {
            a.look = "light".into();
        }
        if !ACCENTS.iter().any(|c| c.eq_ignore_ascii_case(&a.accent)) {
            a.accent = ACCENTS[0].into();
        }
        if ![1.0, 1.25, 1.5].contains(&a.text_scale) {
            a.text_scale = 1.0;
        }
        Ok(a)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Atomic write (temp file, fsync, rename, directory fsync).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, self.to_json().as_bytes(), 0o600)
    }

    /// The `Finish` choices (DESIGN.md's keys, exactly).
    pub fn choices(&self) -> BTreeMap<String, Value> {
        let accent = ACCENTS
            .iter()
            .find(|c| c.eq_ignore_ascii_case(&self.accent))
            .unwrap_or(&ACCENTS[0]);
        let mut m = BTreeMap::new();
        m.insert("look".to_string(), Value::Str(self.look.clone()));
        m.insert("accent".to_string(), Value::Str((*accent).to_string()));
        m.insert("text_scale".to_string(), Value::F64(self.text_scale));
        m.insert("high_contrast".to_string(), Value::Bool(self.high_contrast));
        m.insert("screen_reader".to_string(), Value::Bool(self.screen_reader));
        m.insert("crash_reports".to_string(), Value::Bool(self.crash_reports));
        if !self.keyboard_layout.is_empty() {
            let kb = BTreeMap::from([
                (
                    "layout".to_string(),
                    Value::Str(self.keyboard_layout.clone()),
                ),
                (
                    "variant".to_string(),
                    Value::Str(self.keyboard_variant.clone()),
                ),
            ]);
            m.insert("keyboard".to_string(), Value::Map(kb));
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wizard_core::choices::validate;

    #[test]
    fn choices_are_accepted_by_the_helper_rules() {
        let a = Answers {
            keyboard_layout: "us".into(),
            keyboard_variant: "dvorak".into(),
            look: "dark".into(),
            text_scale: 1.25,
            ..Answers::default()
        };
        let c = validate(a.choices()).unwrap();
        assert_eq!(c.keyboard.unwrap().variant, "dvorak");
        assert!(validate(Answers::default().choices()).is_ok());
    }

    #[test]
    fn odd_values_are_clamped_and_save_round_trips() {
        let a = Answers::from_json(r##"{"look":"x","accent":"#000000","textScale":9,"stuff":1}"##)
            .unwrap();
        assert_eq!(
            (a.look.as_str(), a.accent.as_str(), a.text_scale),
            ("light", ACCENTS[0], 1.0)
        );
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("run/answers.json");
        let b = Answers {
            step: "account".into(),
            full_name: "Ada L".into(),
            ..a
        };
        b.save(&p).unwrap();
        assert_eq!(Answers::load(&p), b);
        assert!(
            !std::fs::read_to_string(&p)
                .unwrap()
                .to_lowercase()
                .contains("password")
        );
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(Answers::load(&p), Answers::default());
        assert_eq!(Answers::load(&dir.path().join("none")), Answers::default());
    }
}

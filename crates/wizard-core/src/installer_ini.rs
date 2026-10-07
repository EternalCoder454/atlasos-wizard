//! `/etc/telamon/installer.ini` (or `/etc/atlasos/installer.ini`), written by
//! the installer (DESIGN.md, Pages). Anything wrong with the file means "every page shows".

use crate::choices::{xkb_layout, xkb_variant};
use crate::ini::Ini;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Where the installer leaves its answers.
pub const DEFAULT_PATH: &str = "/etc/telamon/installer.ini";

/// Where Atlas Installer left them (until Telamon Installer 0.x; it writes
/// both for one release). Read when [`DEFAULT_PATH`] does not exist.
pub const LEGACY_PATH: &str = "/etc/atlasos/installer.ini";

/// Largest file read; anything bigger is treated as garbage.
const MAX_BYTES: u64 = 64 * 1024;

/// What the installer already asked. Default: nothing answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallerAnswers {
    /// The `Version` key, when present and a number.
    pub version: Option<u32>,
    /// Locale such as `en_US.UTF-8`.
    pub language: Option<String>,
    /// XKB layout.
    pub keyboard_layout: Option<String>,
    /// XKB variant.
    pub keyboard_variant: Option<String>,
    /// The installer connected to a network (`Network=true`).
    pub network: bool,
}

impl InstallerAnswers {
    /// True when the Language page can be skipped.
    pub fn skip_language(&self) -> bool {
        self.language.is_some()
    }

    /// True when the Keyboard page can be skipped.
    pub fn skip_keyboard(&self) -> bool {
        self.keyboard_layout.is_some()
    }

    /// True when the Wi-Fi page may be skipped; the caller also checks that
    /// NetworkManager reports full connectivity.
    pub fn wifi_may_skip(&self) -> bool {
        self.network
    }
}

/// Why the answers are the defaults (for the caller's log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// There is no file.
    Missing,
    /// The file could not be read (the I/O error kind, as text).
    Unreadable(String),
    /// Larger than 64 KiB.
    TooLarge,
    /// Not valid UTF-8.
    NotText,
    /// No `[Installer]` group.
    NoInstallerGroup,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Missing => f.write_str("installer.ini is missing"),
            Reason::Unreadable(e) => write!(f, "installer.ini is unreadable: {e}"),
            Reason::TooLarge => f.write_str("installer.ini is too large"),
            Reason::NotText => f.write_str("installer.ini is not UTF-8 text"),
            Reason::NoInstallerGroup => f.write_str("installer.ini has no [Installer] group"),
        }
    }
}

/// The answers, and why they are the defaults when they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The parsed answers (all unset when `reason` is set).
    pub answers: InstallerAnswers,
    /// Why nothing was read, for the log; `None` for a normal read.
    pub reason: Option<Reason>,
}

impl Loaded {
    fn defaults(reason: Reason) -> Loaded {
        Loaded {
            answers: InstallerAnswers::default(),
            reason: Some(reason),
        }
    }
}

/// [`load`] of [`DEFAULT_PATH`], or of [`LEGACY_PATH`] when there is no
/// such file.
pub fn load_default() -> Loaded {
    load_either(Path::new(DEFAULT_PATH), Path::new(LEGACY_PATH))
}

/// [`load`] of `new`, or of `old` when `new` does not exist (a file that
/// exists but is wrong counts: the installer that wrote it is the new one).
pub fn load_either(new: &Path, old: &Path) -> Loaded {
    let l = load(new);
    if l.reason == Some(Reason::Missing) {
        let o = load(old);
        if o.reason != Some(Reason::Missing) {
            return o;
        }
    }
    l
}

/// Reads and parses the file. Never fails: see [`Loaded::reason`].
pub fn load(path: &Path) -> Loaded {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::defaults(Reason::Missing),
        Err(e) => return Loaded::defaults(Reason::Unreadable(e.kind().to_string())),
    };
    let mut buf = Vec::new();
    if let Err(e) = file.take(MAX_BYTES + 1).read_to_end(&mut buf) {
        return Loaded::defaults(Reason::Unreadable(e.kind().to_string()));
    }
    if buf.len() as u64 > MAX_BYTES {
        return Loaded::defaults(Reason::TooLarge);
    }
    match String::from_utf8(buf) {
        Ok(text) => parse(&text),
        Err(_) => Loaded::defaults(Reason::NotText),
    }
}

/// Parses the file's text. Values are trimmed, empty means unset, unknown
/// keys and groups are ignored. A language or keyboard value of an
/// impossible shape counts as unset (the page then shows).
pub fn parse(text: &str) -> Loaded {
    let ini = Ini::parse(text);
    if !ini.has_group("Installer") {
        return Loaded::defaults(Reason::NoInstallerGroup);
    }
    let get = |k| ini.get("Installer", k);
    let answers = InstallerAnswers {
        version: get("Version").and_then(|v| v.parse().ok()),
        language: get("Language")
            .filter(|v| locale_shape(v))
            .map(String::from),
        keyboard_layout: get("KeyboardLayout")
            .filter(|v| xkb_layout(v))
            .map(String::from),
        keyboard_variant: get("KeyboardVariant")
            .filter(|v| !v.is_empty() && xkb_variant(v))
            .map(String::from),
        network: get("Network").is_some_and(|v| {
            ["true", "1", "yes", "on"]
                .iter()
                .any(|t| v.eq_ignore_ascii_case(t))
        }),
    };
    Loaded {
        answers,
        reason: None,
    }
}

fn locale_shape(s: &str) -> bool {
    s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'@' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Loaded {
        load(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/installer")
                .join(name),
        )
    }

    #[test]
    fn the_new_path_wins_and_the_old_one_is_read_when_it_is_missing() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/installer");
        let full = fixtures.join("full.ini");
        let empty = fixtures.join("empty-values.ini");
        let none = fixtures.join("none.ini");
        // Only the old file (a machine Atlas Installer made).
        assert_eq!(load_either(&none, &full), load(&full));
        assert!(load_either(&none, &full).reason.is_none());
        // Both: the new one.
        assert_eq!(load_either(&full, &empty), load(&full));
        assert_eq!(load_either(&empty, &full), load(&empty));
        // Neither.
        assert_eq!(load_either(&none, &none).reason, Some(Reason::Missing));
        // A wrong new file is not covered by the old one.
        let junk = fixtures.join("garbage.ini");
        assert_eq!(load_either(&junk, &full), load(&junk));
    }

    #[test]
    fn full_file() {
        let l = fixture("full.ini");
        assert_eq!(l.reason, None);
        let a = l.answers;
        assert_eq!(a.version, Some(1));
        assert_eq!(a.language.as_deref(), Some("de_DE.UTF-8"));
        assert_eq!(a.keyboard_layout.as_deref(), Some("de"));
        assert_eq!(a.keyboard_variant.as_deref(), Some("nodeadkeys"));
        assert!(a.network);
        assert!(a.skip_language() && a.skip_keyboard() && a.wifi_may_skip());
    }

    #[test]
    fn empty_values_are_unset() {
        let l = fixture("empty-values.ini");
        assert_eq!(l.reason, None);
        assert_eq!(
            l.answers,
            InstallerAnswers {
                version: Some(1),
                ..Default::default()
            }
        );
        assert!(!l.answers.skip_language() && !l.answers.skip_keyboard());
        assert!(!l.answers.wifi_may_skip());
    }

    #[test]
    fn missing_file() {
        let l = load(Path::new("/nonexistent/dir/installer.ini"));
        assert_eq!(l.reason, Some(Reason::Missing));
        assert_eq!(l.answers, InstallerAnswers::default());
    }

    #[test]
    fn unreadable_is_a_directory() {
        let d = tempfile::tempdir().unwrap();
        let l = load(d.path());
        assert!(
            matches!(l.reason, Some(Reason::Unreadable(_))),
            "{:?}",
            l.reason
        );
        assert_eq!(l.answers, InstallerAnswers::default());
    }

    #[test]
    fn garbage_binary() {
        let l = fixture("garbage.bin");
        assert_eq!(l.reason, Some(Reason::NotText));
        assert_eq!(l.answers, InstallerAnswers::default());
    }

    #[test]
    fn garbage_text() {
        let l = fixture("garbage.ini");
        assert_eq!(l.reason, Some(Reason::NoInstallerGroup));
        assert_eq!(l.answers, InstallerAnswers::default());
    }

    #[test]
    fn higher_version_read_the_same_way() {
        let l = fixture("version-9.ini");
        assert_eq!(l.reason, None);
        assert_eq!(l.answers.version, Some(9));
        assert_eq!(l.answers.language.as_deref(), Some("fr_FR.UTF-8"));
        assert_eq!(l.answers.keyboard_layout.as_deref(), Some("fr"));
        assert!(!l.answers.network);
    }

    #[test]
    fn unknown_keys_groups_comments_and_whitespace() {
        let l = fixture("noisy.ini");
        assert_eq!(l.reason, None);
        let a = l.answers;
        assert_eq!(a.language.as_deref(), Some("en_GB.UTF-8"));
        assert_eq!(a.keyboard_layout.as_deref(), Some("gb"));
        assert_eq!(a.keyboard_variant, None);
        // Last duplicate wins; the other group's Language is ignored.
        assert!(a.network);
    }

    #[test]
    fn wrong_shapes_count_as_unset() {
        let l = parse(
            "[Installer]\nLanguage=en US; rm -rf\nKeyboardLayout=../x\nKeyboardVariant=A B\nNetwork=maybe\n",
        );
        assert_eq!(l.reason, None);
        assert_eq!(l.answers, InstallerAnswers::default());
    }

    #[test]
    fn too_large() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big.ini");
        let mut text = String::from("[Installer]\nLanguage=en_US.UTF-8\n");
        text.push_str(&"#".repeat(70_000));
        std::fs::write(&p, text).unwrap();
        let l = load(&p);
        assert_eq!(l.reason, Some(Reason::TooLarge));
        assert_eq!(l.answers, InstallerAnswers::default());
    }

    #[test]
    fn reason_text_is_plain() {
        assert_eq!(Reason::Missing.to_string(), "installer.ini is missing");
    }
}

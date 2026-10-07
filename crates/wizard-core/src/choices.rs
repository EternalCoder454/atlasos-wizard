//! The `Finish` choices (DESIGN.md, The helper): a validated, typed view of
//! the `a{sv}` the GUI sends. Unknown keys are refused.

use std::borrow::Borrow;
use std::collections::BTreeMap;

/// Accent colours the GUI offers: Telamon OS violet first, then KDE's set.
pub const ACCENTS: [&str; 9] = [
    "#6858E2", "#E93A9A", "#E93D58", "#E9643A", "#E8CB2D", "#3DD425", "#00D3B8", "#1D99F3",
    "#9B59D0",
];

/// Global Theme id of the light look.
pub const THEME_LIGHT: &str = "org.atlasos.desktop";
/// Global Theme id of the dark look.
pub const THEME_DARK: &str = "org.atlasos.dark.desktop";

/// A value of the choices map, simple enough for the helper to map zbus
/// values into it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A string.
    Str(String),
    /// A boolean.
    Bool(bool),
    /// A floating-point number.
    F64(f64),
    /// A nested map (only the `keyboard` key uses one).
    Map(BTreeMap<String, Value>),
}

/// Light or dark look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    /// Telamon OS Light.
    #[default]
    Light,
    /// Telamon OS Dark.
    Dark,
}

impl Look {
    /// The Global Theme id for `plasma-apply-lookandfeel --apply`.
    pub fn theme_id(self) -> &'static str {
        match self {
            Look::Light => THEME_LIGHT,
            Look::Dark => THEME_DARK,
        }
    }
}

/// Text scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextScale {
    /// 1.0
    #[default]
    Normal,
    /// 1.25
    Large,
    /// 1.5
    Larger,
}

impl TextScale {
    /// The factor as a number.
    pub fn factor(self) -> f64 {
        match self {
            TextScale::Normal => 1.0,
            TextScale::Large => 1.25,
            TextScale::Larger => 1.5,
        }
    }
}

/// A keyboard layout and variant (XKB names; the variant may be empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyboard {
    /// XKB layout name, e.g. `us`.
    pub layout: String,
    /// XKB variant name, e.g. `dvorak`, or empty.
    pub variant: String,
}

/// Validated choices. Missing keys keep their default.
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    /// Light or dark.
    pub look: Look,
    /// An entry of [`ACCENTS`] (canonical upper-case form).
    pub accent: &'static str,
    /// Text scale.
    pub text_scale: TextScale,
    /// High contrast colour scheme.
    pub high_contrast: bool,
    /// Screen reader on.
    pub screen_reader: bool,
    /// Send crash reports.
    pub crash_reports: bool,
    /// Keyboard layout, when chosen.
    pub keyboard: Option<Keyboard>,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            look: Look::Light,
            accent: ACCENTS[0],
            text_scale: TextScale::Normal,
            high_contrast: false,
            screen_reader: false,
            crash_reports: false,
            keyboard: None,
        }
    }
}

/// Why the choices were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoicesError {
    /// A key that is not one of the known choices.
    UnknownKey(String),
    /// The key's value has the wrong type or is not an allowed value.
    BadValue(&'static str),
}

impl ChoicesError {
    /// Stable identifier for logs and the GUI.
    pub fn code(&self) -> &'static str {
        match self {
            ChoicesError::UnknownKey(_) => "choices-unknown-key",
            ChoicesError::BadValue(_) => "choices-bad-value",
        }
    }
}

/// True for an XKB layout name: `^[a-z0-9_-]{1,32}$`. Checks the shape only,
/// not whether the layout is installed.
pub fn xkb_layout(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(xkb_byte)
}

/// True for an XKB variant name: empty, or the same shape as a layout.
pub fn xkb_variant(s: &str) -> bool {
    s.is_empty() || xkb_layout(s)
}

fn xkb_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
}

/// Validates a choices map. Accepts any iterator of key/value pairs (a
/// `BTreeMap<String, Value>`, a `HashMap`, a slice of tuples).
///
/// # Errors
/// [`ChoicesError::UnknownKey`] for a key that is not a choice,
/// [`ChoicesError::BadValue`] (naming the key) for a bad value.
pub fn validate<I, K, V>(map: I) -> Result<Choices, ChoicesError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Borrow<Value>,
{
    let mut c = Choices::default();
    for (key, value) in map {
        let value = value.borrow();
        match key.as_ref() {
            "look" => {
                c.look = match str_of(value, "look")? {
                    "light" => Look::Light,
                    "dark" => Look::Dark,
                    _ => return Err(ChoicesError::BadValue("look")),
                }
            }
            "accent" => {
                let s = str_of(value, "accent")?;
                c.accent = ACCENTS
                    .iter()
                    .find(|a| a.eq_ignore_ascii_case(s))
                    .ok_or(ChoicesError::BadValue("accent"))?;
            }
            "text_scale" => {
                c.text_scale = match value {
                    Value::F64(f) if *f == 1.0 => TextScale::Normal,
                    Value::F64(f) if *f == 1.25 => TextScale::Large,
                    Value::F64(f) if *f == 1.5 => TextScale::Larger,
                    Value::Str(s) if s == "1.0" => TextScale::Normal,
                    Value::Str(s) if s == "1.25" => TextScale::Large,
                    Value::Str(s) if s == "1.5" => TextScale::Larger,
                    _ => return Err(ChoicesError::BadValue("text_scale")),
                }
            }
            "high_contrast" => c.high_contrast = bool_of(value, "high_contrast")?,
            "screen_reader" => c.screen_reader = bool_of(value, "screen_reader")?,
            "crash_reports" => c.crash_reports = bool_of(value, "crash_reports")?,
            "keyboard" => c.keyboard = Some(keyboard_of(value)?),
            other => return Err(ChoicesError::UnknownKey(other.chars().take(64).collect())),
        }
    }
    Ok(c)
}

fn str_of<'a>(v: &'a Value, key: &'static str) -> Result<&'a str, ChoicesError> {
    match v {
        Value::Str(s) => Ok(s),
        _ => Err(ChoicesError::BadValue(key)),
    }
}

fn bool_of(v: &Value, key: &'static str) -> Result<bool, ChoicesError> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => Err(ChoicesError::BadValue(key)),
    }
}

fn keyboard_of(v: &Value) -> Result<Keyboard, ChoicesError> {
    let bad = ChoicesError::BadValue("keyboard");
    let Value::Map(m) = v else {
        return Err(bad);
    };
    let mut layout = None;
    let mut variant = String::new();
    for (k, v) in m {
        match (k.as_str(), v) {
            ("layout", Value::Str(s)) if xkb_layout(s) => layout = Some(s.clone()),
            ("variant", Value::Str(s)) if xkb_variant(s) => variant = s.clone(),
            _ => return Err(bad),
        }
    }
    Ok(Keyboard {
        layout: layout.ok_or(bad)?,
        variant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::Str(v.into())
    }

    fn map(pairs: &[(&str, Value)]) -> Vec<(String, Value)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(validate(map(&[])).unwrap(), Choices::default());
        assert_eq!(Choices::default().accent, "#6858E2");
    }

    #[test]
    fn full_valid() {
        let kb = Value::Map(BTreeMap::from([
            ("layout".to_string(), s("us")),
            ("variant".to_string(), s("dvorak")),
        ]));
        let c = validate(map(&[
            ("look", s("dark")),
            ("accent", s("#1d99f3")),
            ("text_scale", Value::F64(1.25)),
            ("high_contrast", Value::Bool(true)),
            ("screen_reader", Value::Bool(true)),
            ("crash_reports", Value::Bool(true)),
            ("keyboard", kb),
        ]))
        .unwrap();
        assert_eq!(c.look.theme_id(), "org.atlasos.dark.desktop");
        assert_eq!(c.accent, "#1D99F3");
        assert_eq!(c.text_scale.factor(), 1.25);
        assert!(c.high_contrast && c.screen_reader && c.crash_reports);
        assert_eq!(
            c.keyboard,
            Some(Keyboard {
                layout: "us".into(),
                variant: "dvorak".into()
            })
        );
        assert_eq!(Look::Light.theme_id(), "org.atlasos.desktop");
    }

    #[test]
    fn works_with_btreemap() {
        let m: BTreeMap<String, Value> = BTreeMap::from([("look".to_string(), s("light"))]);
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn unknown_key_refused() {
        assert_eq!(
            validate(map(&[("wallpaper", s("x"))])),
            Err(ChoicesError::UnknownKey("wallpaper".into()))
        );
        assert_eq!(
            validate(map(&[("Look", s("dark"))])).unwrap_err().code(),
            "choices-unknown-key"
        );
    }

    #[test]
    fn long_unknown_key_is_cut() {
        let k = "k".repeat(500);
        match validate(map(&[(k.as_str(), s("x"))])) {
            Err(ChoicesError::UnknownKey(n)) => assert_eq!(n.len(), 64),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bad_values() {
        let bad = |k, v| validate(map(&[(k, v)])).unwrap_err();
        assert_eq!(bad("look", s("blue")), ChoicesError::BadValue("look"));
        assert_eq!(
            bad("look", Value::Bool(true)),
            ChoicesError::BadValue("look")
        );
        assert_eq!(
            bad("accent", s("#000000")),
            ChoicesError::BadValue("accent")
        );
        assert_eq!(bad("accent", s("6858E2")), ChoicesError::BadValue("accent"));
        assert_eq!(
            bad("text_scale", Value::F64(2.0)),
            ChoicesError::BadValue("text_scale")
        );
        assert_eq!(
            bad("text_scale", Value::F64(f64::NAN)),
            ChoicesError::BadValue("text_scale")
        );
        assert_eq!(
            bad("text_scale", s("1.3")),
            ChoicesError::BadValue("text_scale")
        );
        assert_eq!(
            bad("high_contrast", s("yes")),
            ChoicesError::BadValue("high_contrast")
        );
        assert_eq!(
            bad("crash_reports", Value::F64(1.0)),
            ChoicesError::BadValue("crash_reports")
        );
        assert_eq!(bad("keyboard", s("us")), ChoicesError::BadValue("keyboard"));
    }

    #[test]
    fn text_scales_as_strings() {
        for (t, f) in [("1.0", 1.0), ("1.25", 1.25), ("1.5", 1.5)] {
            let c = validate(map(&[("text_scale", s(t))])).unwrap();
            assert_eq!(c.text_scale.factor(), f);
        }
    }

    #[test]
    fn every_listed_accent_is_valid() {
        for a in ACCENTS {
            assert_eq!(validate(map(&[("accent", s(a))])).unwrap().accent, a);
        }
    }

    #[test]
    fn xkb_names() {
        assert!(xkb_layout("us"));
        assert!(xkb_layout("de-neo_2"));
        assert!(xkb_layout(&"a".repeat(32)));
        assert!(!xkb_layout(""));
        assert!(!xkb_layout(&"a".repeat(33)));
        assert!(!xkb_layout("US"));
        assert!(!xkb_layout("us,de"));
        assert!(!xkb_layout("../x"));
        assert!(!xkb_layout("us\n"));
        assert!(xkb_variant(""));
        assert!(xkb_variant("nodeadkeys"));
        assert!(!xkb_variant("a b"));
    }

    #[test]
    fn keyboard_shape() {
        let kb = |pairs: &[(&str, Value)]| {
            validate(map(&[(
                "keyboard",
                Value::Map(
                    pairs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect(),
                ),
            )]))
        };
        assert!(kb(&[("layout", s("us"))]).is_ok());
        assert!(kb(&[("layout", s("us")), ("variant", s(""))]).is_ok());
        assert!(kb(&[("variant", s("x"))]).is_err());
        assert!(kb(&[("layout", s("U S"))]).is_err());
        assert!(kb(&[("layout", s("us")), ("variant", s("a/b"))]).is_err());
        assert!(kb(&[("layout", s("us")), ("model", s("pc105"))]).is_err());
    }
}

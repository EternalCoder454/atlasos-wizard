//! Errors as the pages see them: a stable code (the GUI shows its own
//! translated text for it, `qml/ErrorText.qml`) plus the English text for the
//! log. The helper's message is `<code>: <English text>`.

use std::fmt;

/// A failed call or step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: String,
    pub text: String,
}

impl Fail {
    pub fn new(code: &str, text: impl Into<String>) -> Fail {
        Fail {
            code: code.to_string(),
            text: text.into(),
        }
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.text)
    }
}

/// Splits `<code>: <text>` on the first `: `. A code is lower-case words and
/// digits joined by `-`; a message of another shape becomes `failed`.
pub fn split_message(msg: &str) -> Fail {
    if let Some((code, text)) = msg.split_once(": ")
        && !code.is_empty()
        && code.len() <= 48
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Fail::new(code, text);
    }
    Fail::new("failed", msg)
}

/// A D-Bus error: ours carry `<code>: <text>`; the bus's own errors map to
/// a code of their own.
pub fn from_dbus(name: &str, msg: &str) -> Fail {
    if name.starts_with("net.eterneon.telamon.Error.") {
        return split_message(msg);
    }
    match name {
        "org.freedesktop.DBus.Error.AccessDenied"
        | "org.freedesktop.DBus.Error.AuthFailed"
        | "org.freedesktop.PolicyKit1.Error.NotAuthorized" => {
            Fail::new("not-authorized", format!("{name}: {msg}"))
        }
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner"
        | "org.freedesktop.DBus.Error.NoReply"
        | "org.freedesktop.DBus.Error.Timeout" => {
            Fail::new("unreachable", format!("{name}: {msg}"))
        }
        _ => Fail::new("failed", format!("{name}: {msg}")),
    }
}

/// Every code a page can be given, for the test that `ErrorText.qml` has a
/// text for each.
#[cfg(test)]
pub const CODES: &[&str] = &[
    // wizard-core, user name / full name / password
    "name-empty",
    "name-too-long",
    "name-bad-start",
    "name-bad-char",
    "name-reserved",
    "name-exists",
    "name-unreadable",
    "full-name-too-long",
    "full-name-bad-char",
    "full-name-control",
    "password-too-short",
    "password-too-long",
    "password-not-text",
    "password-has-nul",
    "password-user-name",
    "password-full-name",
    "password-quality-user",
    "password-quality-dictionary",
    "password-quality-simple",
    "password-quality-repetitive",
    "password-quality-short",
    "password-quality-other",
    "password-check-unavailable",
    // the helper
    "user-name-exists",
    "accounts-password-failed",
    "setup-done",
    "polkit-denied",
    "not-setup-user",
    "caller-unknown",
    "no-sender",
    "shutting-down",
    "choices-bad-value",
    "settings-failed",
    "restart-failed",
    "verify-no-passwd-entry",
    "io",
    "worker",
    // this app
    "not-authorized",
    "unreachable",
    "timeout",
    "failed",
    "wifi-failed",
    "wifi-no-device",
    "orca-missing",
    "bad-argument",
    "list-unreadable",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_the_first_separator() {
        let f = split_message("user-name-exists: That name: taken.");
        assert_eq!(
            (f.code.as_str(), f.text.as_str()),
            ("user-name-exists", "That name: taken.")
        );
        assert_eq!(split_message("Boom: x").code, "failed");
        assert_eq!(split_message("no separator").code, "failed");
        assert_eq!(split_message(": x").code, "failed");
    }

    #[test]
    fn dbus_names_map() {
        assert_eq!(
            from_dbus("net.eterneon.telamon.Error.Invalid", "name-reserved: no").code,
            "name-reserved"
        );
        assert_eq!(
            from_dbus("org.freedesktop.DBus.Error.AccessDenied", "x").code,
            "not-authorized"
        );
        assert_eq!(
            from_dbus("org.freedesktop.DBus.Error.ServiceUnknown", "x").code,
            "unreachable"
        );
        assert_eq!(from_dbus("org.x.Y", "z").code, "failed");
    }

    #[test]
    fn every_code_has_a_text_in_qml() {
        let qml = include_str!("../qml/ErrorText.qml");
        for c in CODES {
            assert!(qml.contains(&format!("\"{c}\"")), "ErrorText.qml lacks {c}");
        }
        // and the wizard-core codes are all listed
        use wizard_core::password::PasswordError as P;
        use wizard_core::validate::{FullNameError as F, NameError as N};
        for c in [
            N::Empty.code(),
            N::TooLong.code(),
            N::BadStart.code(),
            N::BadChar.code(),
            N::Reserved.code(),
            N::Exists.code(),
            N::Unreadable.code(),
            F::TooLong.code(),
            F::BadChar.code(),
            F::Control.code(),
            P::TooShort.code(),
            P::TooLong.code(),
            P::NotText.code(),
            P::HasNul.code(),
            P::ContainsUserName.code(),
            P::ContainsFullName.code(),
            P::Unavailable.code(),
        ] {
            assert!(CODES.contains(&c), "{c} missing from CODES");
        }
    }
}

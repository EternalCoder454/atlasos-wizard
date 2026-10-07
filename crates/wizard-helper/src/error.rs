//! The helper's errors: five D-Bus error names, each with a stable code the GUI
//! maps to translated text and a short English message.
//!
//! On the bus the message is `"<code>: <English text>"`; the code is lower-case
//! words joined by `-` and never contains `:`. No message ever holds a
//! password or a hash.

use std::fmt;

/// Which of the five `net.eterneon.telamon.Error.*` names an error has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// polkit said no, or the caller is not the setup user.
    NotAuthorized,
    /// Setup is already done.
    SetupDone,
    /// An argument or a request the state does not allow.
    Invalid,
    /// AccountsService failed or is unreachable.
    AccountsService,
    /// Anything else.
    Failed,
}

impl Kind {
    /// The D-Bus error name's last part.
    pub fn name(self) -> &'static str {
        match self {
            Kind::NotAuthorized => "NotAuthorized",
            Kind::SetupDone => "SetupDone",
            Kind::Invalid => "Invalid",
            Kind::AccountsService => "AccountsService",
            Kind::Failed => "Failed",
        }
    }
}

/// An error of a helper method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperError {
    /// The D-Bus error name.
    pub kind: Kind,
    /// Stable code, e.g. `user-name-exists`.
    pub code: String,
    /// Short English text (for logs and as a fallback).
    pub text: String,
}

impl HelperError {
    /// A new error.
    pub fn new(kind: Kind, code: impl Into<String>, text: impl Into<String>) -> Self {
        HelperError {
            kind,
            code: code.into(),
            text: text.into(),
        }
    }

    /// polkit or the caller check refused.
    pub fn not_authorized(code: &str, text: &str) -> Self {
        Self::new(Kind::NotAuthorized, code, text)
    }

    /// Setup is done.
    pub fn setup_done() -> Self {
        Self::new(Kind::SetupDone, "setup-done", "Setup is already finished.")
    }

    /// A bad argument or request.
    pub fn invalid(code: &str, text: &str) -> Self {
        Self::new(Kind::Invalid, code, text)
    }

    /// AccountsService trouble.
    pub fn accounts(code: &str, text: &str) -> Self {
        Self::new(Kind::AccountsService, code, text)
    }

    /// Anything else.
    pub fn failed(code: &str, text: &str) -> Self {
        Self::new(Kind::Failed, code, text)
    }

    /// The message sent on the bus: `"<code>: <text>"`.
    pub fn message(&self) -> String {
        format!("{}: {}", self.code, self.text)
    }
}

impl fmt::Display for HelperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.name(), self.message())
    }
}

impl std::error::Error for HelperError {}

/// The error type of the D-Bus methods.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "net.eterneon.telamon.Error")]
pub enum DbusError {
    /// zbus's own errors (a malformed call and the like).
    #[zbus(error)]
    ZBus(zbus::Error),
    /// See [`Kind::NotAuthorized`].
    NotAuthorized(String),
    /// See [`Kind::SetupDone`].
    SetupDone(String),
    /// See [`Kind::Invalid`].
    Invalid(String),
    /// See [`Kind::AccountsService`].
    AccountsService(String),
    /// See [`Kind::Failed`].
    Failed(String),
}

impl From<HelperError> for DbusError {
    fn from(e: HelperError) -> Self {
        let m = e.message();
        match e.kind {
            Kind::NotAuthorized => DbusError::NotAuthorized(m),
            Kind::SetupDone => DbusError::SetupDone(m),
            Kind::Invalid => DbusError::Invalid(m),
            Kind::AccountsService => DbusError::AccountsService(m),
            Kind::Failed => DbusError::Failed(m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_code_then_text() {
        let e = HelperError::invalid("user-name-exists", "That user name is taken.");
        assert_eq!(e.message(), "user-name-exists: That user name is taken.");
        assert!(matches!(DbusError::from(e), DbusError::Invalid(_)));
    }

    #[test]
    fn every_kind_maps() {
        for (k, name) in [
            (Kind::NotAuthorized, "NotAuthorized"),
            (Kind::SetupDone, "SetupDone"),
            (Kind::Invalid, "Invalid"),
            (Kind::AccountsService, "AccountsService"),
            (Kind::Failed, "Failed"),
        ] {
            assert_eq!(k.name(), name);
            let d = DbusError::from(HelperError::new(k, "c", "t"));
            use zbus::DBusError as _;
            assert_eq!(
                d.name().as_str(),
                format!("net.eterneon.telamon.Error.{name}")
            );
        }
    }
}

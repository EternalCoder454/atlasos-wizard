//! Telamon Wizard core: everything that decides or validates, with no Qt, no
//! D-Bus and no async. The GUI, the root helper and the boot program all link
//! it, so a rule lives in exactly one place.
//!
//! Error types carry a stable `code()` string that the GUI maps to translated
//! text; nothing here is shown to a user. No error, `Debug` or `Display`
//! output ever contains a password.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod accounts;
pub mod boot;
pub mod choices;
pub mod fsutil;
pub mod installer_ini;
pub mod markers;
pub mod password;
pub mod state;
pub mod timefmt;
pub mod validate;

mod ini;

/// The crate version, written into the done marker as `Wizard=`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

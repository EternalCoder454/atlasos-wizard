//! Telamon Wizard's root D-Bus helper (`telamon-wizard-helper`): the only
//! privileged code the setup GUI reaches. Four methods, none of which takes a
//! path, a command, an argument vector or a unit name; see `docs/DESIGN.md`,
//! "The helper".
//!
//! Layers: [`service`] authorizes (polkit, then the caller's uid) and speaks
//! D-Bus; [`core`] does the work, re-validating everything with `wizard-core`;
//! [`backends`] are the traits (AccountsService, systemd, the lock commands,
//! the settings child) with their real implementations; [`apply`] is the
//! settings child that runs as the new account. Every filesystem path goes
//! through [`paths::Paths`].

#![deny(unsafe_code)]
#![warn(missing_docs)]

// The test-root feature takes every path from the environment; a release
// build (the RPM's) must never carry it.
#[cfg(all(feature = "test-root", not(debug_assertions)))]
compile_error!("the test-root feature is for test builds only, never a release build");

#[cfg(test)]
mod budget;

pub mod apply;
pub mod backends;
pub mod core;
pub mod error;
pub mod paths;
pub mod safefs;
pub mod service;

use std::sync::Arc;

use backends::{BusBackend, ChildApplier, SystemRunner};
use paths::Paths;

/// The real helper's logic over the system bus connection `conn`.
pub fn system_core(paths: Paths, conn: &zbus::Connection) -> core::Core {
    let bus = Arc::new(BusBackend::new(conn.clone()));
    core::Core::new(
        paths.clone(),
        bus.clone(),
        bus,
        Arc::new(SystemRunner::new(paths.clone())),
        Arc::new(ChildApplier::new(paths)),
    )
}

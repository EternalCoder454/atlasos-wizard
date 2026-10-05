//! AtlasOS Wizard's root D-Bus helper (`atlas-wizard-helper`): the only
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

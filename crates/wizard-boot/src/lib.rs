//! `telamon-wizard-boot`: `prepare` (every boot, before the display manager) and
//! `fallback` (text-mode account creation). The decisions are
//! `wizard_core::boot`'s; this crate gathers what they need and acts.
//!
//! It runs as root at every boot, so it never hangs (every external command
//! has a 20 s limit), never loops (the count of boots is kept before acting)
//! and never leaves the machine without a way to an account.

#![deny(unsafe_code)]

// The test-root feature takes every path from the environment; a release
// build (the RPM's) must never carry it.
#[cfg(all(feature = "test-root", not(debug_assertions)))]
compile_error!("the test-root feature is for test builds only, never a release build");

pub mod cmd;
pub mod console;
#[cfg(any(test, feature = "test-root"))]
pub mod fake;
pub mod fallback;
pub mod half;
pub mod lock;
pub mod paths;
pub mod prepare;
pub mod sig;
pub mod text;

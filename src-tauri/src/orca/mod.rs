//! Read-only integration with the Orca desktop app (macOS).
//!
//! Orca's state is only ever READ: through its public CLI first, and through a
//! private snapshot copy of its SQLite profile as a fallback. Nothing in this
//! module writes to, or opens for writing, any Orca file.
//!
//! `model`, `derive`, `plan` and `health` are pure and compile everywhere (so
//! they are tested on every OS); `source` is the macOS-only edge I/O.

pub mod derive;
pub mod health;
pub mod model;
pub mod plan;
#[cfg(target_os = "macos")]
pub mod probe;
#[cfg(target_os = "macos")]
pub mod source;

#[cfg(test)]
pub(crate) mod fixtures;

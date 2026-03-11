#![doc(hidden)]
//! Internal API for the awswit and autoawswit binaries.
//!
//! All modules are `pub` because both binaries need access, but this is not
//! a stable public API — breaking changes may occur without notice.

pub mod autorefresh;
pub mod aws;
pub mod cache;
pub mod cli;
pub mod config;
pub mod error;
pub mod history;
pub mod profile;
pub mod shell;
pub mod tui;
pub mod utils;

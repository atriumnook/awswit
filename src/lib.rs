#![doc(hidden)]
//! Internal API for the awswit binary.
//!
//! All modules are `pub` because tests and the binary need access, but this is
//! not a stable public API — breaking changes may occur without notice.

pub mod cli;
pub mod config;
pub mod context;
pub mod error;
pub mod history;
pub mod profile;
pub mod shell;
pub mod tui;
pub mod utils;

#![allow(clippy::collapsible_if)]
#![cfg_attr(test, allow(clippy::await_holding_lock))]
//! CerberusMCP — universal MCP management and gateway platform.

pub mod adapters;
pub mod backup;
pub mod cli;
pub mod commands;
pub mod diagnostics;
pub mod error;
pub mod gateway;
pub mod paths;
pub mod policy;
pub mod process;
pub mod registry;
pub mod routing;
pub mod secrets;
pub mod sync;

pub use error::{CerberusError, ExitCode, Result};

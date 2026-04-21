//! Bugbot command group - automated bug detection on code changes
//!
//! Subcommands:
//! - `check`: Analyze uncommitted/staged changes for potential bugs

pub mod baseline;
pub mod changes;
mod check;
pub mod dead;
pub mod diff;
pub mod first_run;
pub mod l2;
pub mod parsers;
pub mod runner;
pub mod signature;
pub mod text_format;
pub mod tools;
mod types;

pub use check::BugbotCheckArgs;
pub use types::*;

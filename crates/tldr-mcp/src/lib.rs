//! TLDR MCP - Model Context Protocol server library for Claude Code integration
//!
//! This library implements the MCP protocol layer and tool registry for the TLDR
//! code analysis system. It can be used as a library for embedding the MCP server
//! or testing, while the binary (`tldr-mcp`) provides the stdio server entry point.
//!
//! # Modules
//!
//! - [`cache`] - L1 in-process cache for tool results
//! - [`protocol`] - JSON-RPC 2.0 protocol handling
//! - [`tools`] - Tool definitions and registry

pub mod cache;
pub mod protocol;
pub mod tools;

mod server;
pub use server::run;

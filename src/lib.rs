//! # mcp-rtk
//!
//! A token-optimizing MCP proxy that sits between Claude and any upstream MCP
//! server, compressing tool responses to reduce token consumption by 60–90%.
//!
//! # Architecture
//!
//! ```text
//! Claude ←(stdio)→ mcp-rtk ←(stdio/subprocess)→ upstream MCP server
//! ```
//!
//! mcp-rtk is both an MCP **server** (for Claude) and an MCP **client** (for
//! upstream). It forwards `list_tools` and `call_tool` requests, applying JSON
//! compression filters on responses before returning them.
//!
//! # Modules
//!
//! * [`config`] — TOML configuration loading with per-tool filter rules.
//! * [`filter`] — The 8-stage filter pipeline and generic JSON compression
//!   functions.
//! * [`proxy`] — [`ProxyServer`](proxy::ProxyServer) and
//!   [`ProxyClient`](proxy::ProxyClient) implementing the MCP server/client
//!   handlers.
//! * [`tracking`] — SQLite-backed token savings metrics.

pub mod config;
pub mod discover;
pub mod display;
pub mod filter;
pub mod install;
pub mod proxy;
pub mod tracking;

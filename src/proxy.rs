//! MCP proxy server and client handlers.
//!
//! This module implements the core proxy logic: [`ProxyServer`] acts as an MCP
//! server for Claude (served over stdio), forwarding every request to the
//! upstream MCP server via [`ProxyClient`]. Tool call responses pass through the
//! [`FilterEngine`] before being returned to Claude, compressing JSON payloads
//! by 60-90%.
//!
//! # Architecture
//!
//! ```text
//! Claude <-(stdio)-> ProxyServer <-(child-process)-> upstream MCP server
//!                       |                              ^
//!                  FilterEngine                   ProxyClient
//! ```
//!
//! The upstream connection is established lazily: the stdio server starts
//! immediately so Claude Code can complete its MCP handshake, while the
//! upstream connection is initialized in the background.

use std::sync::Arc;

use arc_swap::ArcSwap;
use rmcp::handler::client::ClientHandler;
use rmcp::handler::server::ServerHandler;
use rmcp::model::*;
use rmcp::service::{Peer, RequestContext, RoleClient, RoleServer};
use rmcp::Error as McpError;

use crate::filter::FilterEngine;
use crate::tracking::Tracker;

/// MCP server handler that proxies requests to an upstream MCP server.
///
/// `ProxyServer` intercepts every `call_tool` response and applies the
/// [`FilterEngine`] pipeline before returning results to Claude, while
/// `list_tools` responses are forwarded as-is.
///
/// The filter engine is held behind an [`ArcSwap`] so it can be atomically
/// replaced at runtime when external presets change on disk (hot reload).
///
/// # Examples
///
/// ```no_run
/// # use std::sync::Arc;
/// # use arc_swap::ArcSwap;
/// # use mcp_rtk::config::Config;
/// # use mcp_rtk::filter::FilterEngine;
/// # use mcp_rtk::proxy::ProxyServer;
/// # use rmcp::service::{Peer, RoleClient};
/// let config = Arc::new(Config::from_upstream(&["npx", "some-mcp"], None).unwrap());
/// let engine = Arc::new(ArcSwap::from(Arc::new(FilterEngine::new(config))));
/// // let proxy = ProxyServer::new(engine, None, upstream_peer);
/// ```
#[derive(Clone)]
pub struct ProxyServer {
    /// Handle to the upstream MCP server.
    upstream: Peer<RoleClient>,
    /// The filter engine, atomically swappable for hot reload.
    filter: Arc<ArcSwap<FilterEngine>>,
    /// Optional token-savings tracker (SQLite-backed).
    tracker: Option<Arc<Tracker>>,
    /// Peer handle for the downstream (Claude) connection.
    peer: Option<Peer<RoleServer>>,
}

impl ProxyServer {
    /// Create a new proxy server with an already-connected upstream peer.
    ///
    /// # Arguments
    ///
    /// * `engine` — The shared, hot-reloadable filter engine.
    /// * `tracker` — Optional [`Tracker`] for recording token savings.
    /// * `upstream` — Peer handle to the upstream MCP server.
    pub fn new(
        engine: Arc<ArcSwap<FilterEngine>>,
        tracker: Option<Arc<Tracker>>,
        upstream: Peer<RoleClient>,
    ) -> Self {
        Self {
            upstream,
            filter: engine,
            tracker,
            peer: None,
        }
    }
}

impl ServerHandler for ProxyServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(true),
                }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "mcp-rtk".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some("Token-optimizing MCP proxy".into()),
        }
    }

    fn get_peer(&self) -> Option<Peer<RoleServer>> {
        self.peer.clone()
    }

    fn set_peer(&mut self, peer: Peer<RoleServer>) {
        self.peer = Some(peer);
    }

    fn list_tools(
        &self,
        request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let upstream = self.upstream.clone();
        async move {
            // Ensure params is always Some — rmcp serializes None as
            // "params": null which causes some transports to hang.
            let params = request.or(Some(PaginatedRequestParamInner { cursor: None }));
            upstream.list_tools(params).await.map_err(|e| {
                McpError::internal_error(format!("upstream list_tools failed: {e}"), None)
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        // Load the current engine snapshot (lock-free, zero-copy on the hot path).
        // If a hot reload swaps the engine mid-flight, this request finishes
        // with the snapshot it started with.
        let filter = self.filter.load_full();
        let tracker = self.tracker.clone();
        let preset = filter
            .config()
            .preset
            .clone()
            .unwrap_or_else(|| "generic".to_string());
        let tool_name = request.name.to_string();
        let upstream = self.upstream.clone();

        async move {
            let result = upstream.call_tool(request).await.map_err(|e| {
                McpError::internal_error(format!("upstream call_tool failed: {e}"), None)
            })?;

            let filtered_content: Vec<Content> = result
                .content
                .into_iter()
                .map(|content| {
                    filter_content(&filter, &tool_name, content, tracker.as_ref(), &preset)
                })
                .collect();

            Ok(CallToolResult {
                content: filtered_content,
                is_error: result.is_error,
            })
        }
    }
}

/// MCP client handler for the upstream server connection.
///
/// `ProxyClient` maintains the peer handle to the upstream MCP server. It
/// implements [`ClientHandler`] with default behavior — no custom notification
/// handling is needed since all filtering happens on the server side.
///
/// # Examples
///
/// ```no_run
/// # use mcp_rtk::proxy::ProxyClient;
/// let client = ProxyClient::new();
/// ```
#[derive(Clone, Default)]
pub struct ProxyClient {
    /// Peer handle to the upstream MCP server.
    peer: Option<Peer<RoleClient>>,
}

impl ProxyClient {
    /// Create a new proxy client with no peer connection.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClientHandler for ProxyClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: Default::default(),
            capabilities: Default::default(),
            client_info: Implementation {
                name: "mcp-rtk-client".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        }
    }

    fn get_peer(&self) -> Option<Peer<RoleClient>> {
        self.peer.clone()
    }

    fn set_peer(&mut self, peer: Peer<RoleClient>) {
        self.peer = Some(peer);
    }
}

/// Apply the filter pipeline to a single MCP content block.
///
/// Only text content is filtered; images and resources pass through unchanged.
/// If a [`Tracker`] is provided, the raw and filtered sizes are recorded.
fn filter_content(
    filter: &FilterEngine,
    tool_name: &str,
    content: Content,
    tracker: Option<&Arc<Tracker>>,
    preset: &str,
) -> Content {
    match &content.raw {
        RawContent::Text(text_content) => {
            let raw = &text_content.text;
            let filtered = filter.filter(tool_name, raw);

            if let Some(tracker) = tracker {
                if let Err(e) = tracker.track(tool_name, raw, &filtered, preset) {
                    tracing::warn!("Failed to track tool call: {e}");
                }
            }

            let raw_len = raw.len().max(1);
            tracing::debug!(
                tool = tool_name,
                raw_len = raw.len(),
                filtered_len = filtered.len(),
                savings_pct = format!(
                    "{:.1}%",
                    (1.0 - filtered.len() as f64 / raw_len as f64) * 100.0
                ),
                "Filtered tool response"
            );

            Annotated {
                raw: RawContent::Text(RawTextContent { text: filtered }),
                annotations: content.annotations,
            }
        }
        _ => content,
    }
}

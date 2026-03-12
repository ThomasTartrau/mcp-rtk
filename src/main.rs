//! Binary entry point for the mcp-rtk proxy.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rmcp::transport::{child_process::TokioChildProcess, io};
use rmcp::ServiceExt;
use tokio::process::Command;
use tracing_subscriber::EnvFilter;

use mcp_rtk::config::Config;
use mcp_rtk::proxy::{ProxyClient, ProxyServer};
use mcp_rtk::tracking::Tracker;

/// Token-optimizing MCP proxy — zero-config, preset-based.
///
/// Wraps any MCP server and compresses tool responses to save 60–90% tokens.
///
/// # Usage
///
/// ```text
/// mcp-rtk -- npx @nicepkg/gitlab-mcp
/// mcp-rtk --preset gitlab -- node /path/to/server.js
/// mcp-rtk --config custom.toml -- npx @nicepkg/gitlab-mcp
/// mcp-rtk gain
/// mcp-rtk gain --history
/// ```
#[derive(Parser)]
#[command(name = "mcp-rtk", version, about = "Token-optimizing MCP proxy")]
struct Cli {
    /// Path to optional TOML config file (for power-user overrides).
    #[arg(short, long)]
    config: Option<String>,

    /// Force a specific preset (overrides auto-detection).
    #[arg(short, long)]
    preset: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// Upstream MCP command and arguments (everything after --).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    upstream: Vec<String>,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Commands {
    /// Show token savings statistics.
    Gain {
        /// Show recent call history instead of the summary.
        #[arg(long)]
        history: bool,
    },
    /// Analyze Claude Code history for MCP servers that would benefit from mcp-rtk.
    Discover {
        /// Number of days to look back (default: 30).
        #[arg(long, default_value = "30")]
        days: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // Handle subcommands
    match &cli.command {
        Some(Commands::Gain { history }) => {
            let config = Config::load_for_gain(cli.config.as_deref().map(std::path::Path::new))?;
            let tracker = Tracker::new(&config.tracking.db_path)?;
            if *history {
                tracker.print_history()?;
            } else {
                tracker.print_stats()?;
            }
            return Ok(());
        }
        Some(Commands::Discover { days }) => {
            mcp_rtk::discover::run_discover(*days)?;
            return Ok(());
        }
        None => {}
    }

    // Proxy mode: need upstream args
    if cli.upstream.is_empty() {
        anyhow::bail!(
            "No upstream command provided.\n\n\
             Usage: mcp-rtk -- <command> [args...]\n\
             Example: mcp-rtk -- npx @nicepkg/gitlab-mcp\n\n\
             Available presets: {}\n\
             Use --preset <name> to force a preset.\n\
             Use --config <file> for custom overrides.",
            Config::available_presets().join(", ")
        );
    }

    let upstream_refs: Vec<&str> = cli.upstream.iter().map(|s| s.as_str()).collect();
    let mut config = Config::from_upstream(
        &upstream_refs,
        cli.config.as_deref().map(std::path::Path::new),
    )?;

    // Override preset if explicitly specified
    if let Some(preset_name) = &cli.preset {
        if let Some(preset) = Config::load_preset_by_name(preset_name) {
            for (k, v) in preset {
                config.filters.tools.insert(k, v);
            }
            config.preset = Some(preset_name.clone());
        } else {
            anyhow::bail!(
                "Unknown preset: {preset_name}\nAvailable: {}",
                Config::available_presets().join(", ")
            );
        }
    }

    let config = Arc::new(config);

    if let Some(ref preset) = config.preset {
        tracing::info!("Using preset: {preset}");
    } else {
        tracing::info!("No preset detected, using generic defaults");
    }

    let tracker = if config.tracking.enabled {
        Tracker::new(&config.tracking.db_path)
            .map(|t| Some(Arc::new(t)))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to initialize tracker: {e}");
                None
            })
    } else {
        None
    };

    // Connect to upstream first, then serve stdio.
    // The upstream handshake takes ~250ms. Claude Code buffers its
    // initialize request in the pipe, so it's not lost.
    let mut cmd = Command::new(&config.upstream.command);
    cmd.args(&config.upstream.args);
    cmd.stderr(std::process::Stdio::null());
    for (k, v) in &config.upstream.env {
        cmd.env(k, v);
    }

    tracing::info!(
        "Spawning upstream: {} {}",
        config.upstream.command,
        config.upstream.args.join(" ")
    );

    let child_transport =
        TokioChildProcess::new(&mut cmd).context("Failed to spawn upstream MCP server")?;

    let client = ProxyClient::new();
    let upstream_service = client
        .serve(child_transport)
        .await
        .context("Failed to connect to upstream MCP server")?;

    let upstream_peer = upstream_service.peer().clone();
    tracing::info!("Connected to upstream MCP server");

    // Now start stdio server — Claude Code's initialize request has been
    // buffered in the pipe and will be read immediately.
    let proxy = ProxyServer::new(config.clone(), tracker, upstream_peer);
    let stdio_transport = io::stdio();
    let server = proxy
        .serve(stdio_transport)
        .await
        .context("Failed to start proxy server on stdio")?;

    tracing::info!("mcp-rtk proxy listening on stdio");

    // Wait for either side to finish
    tokio::select! {
        _ = server.waiting() => {
            tracing::info!("Stdio server stopped");
        }
        _ = upstream_service.waiting() => {
            tracing::info!("Upstream connection closed");
        }
    }

    Ok(())
}

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
        /// Export stats in the given format (for scripting). Supported: json
        #[arg(long)]
        export: Option<String>,
    },
    /// Analyze Claude Code history for MCP servers that would benefit from mcp-rtk.
    Discover {
        /// Number of days to look back (default: 30).
        #[arg(long, default_value = "30")]
        days: u32,
    },
    /// Validate a preset or config TOML file.
    ValidatePreset {
        /// Path to the TOML file to validate.
        file: String,
    },
    /// Test filters on stdin JSON without running a proxy.
    DryRun {
        /// Preset to use.
        #[arg(long)]
        preset: Option<String>,
        /// Path to config file.
        #[arg(short, long)]
        config: Option<String>,
        /// Tool name to simulate (determines which filter rules apply).
        #[arg(long)]
        tool: String,
    },
    /// Browse available filter presets.
    Presets {
        #[command(subcommand)]
        action: PresetsAction,
    },
}

#[derive(Subcommand)]
enum PresetsAction {
    /// List all available presets.
    List,
    /// Show the full TOML content of a preset.
    Show {
        /// Preset name (e.g. "gitlab", "grafana").
        name: String,
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
        Some(Commands::Gain { history, export }) => {
            let config = Config::load_for_gain(cli.config.as_deref().map(std::path::Path::new))?;
            let tracker = Tracker::new(&config.tracking.db_path)?;
            if let Some(format) = export {
                match format.as_str() {
                    "json" => tracker.export_json()?,
                    _ => anyhow::bail!("Unknown export format: {format}. Supported: json"),
                }
            } else if *history {
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
        Some(Commands::ValidatePreset { file }) => {
            mcp_rtk::config::validate_preset_file(std::path::Path::new(file))?;
            return Ok(());
        }
        Some(Commands::DryRun {
            preset,
            config: dry_config,
            tool,
        }) => {
            use mcp_rtk::display::*;
            use mcp_rtk::filter::FilterEngine;
            use std::io::Read;

            // Build config: if preset specified, use a fake upstream with that preset keyword
            // If config specified, load from file
            let config_path = dry_config.as_deref().map(std::path::Path::new);
            let fake_upstream: Vec<&str> = if let Some(ref p) = preset {
                vec!["dry-run", p] // preset keyword will be detected from args
            } else {
                vec!["dry-run"]
            };

            let mut config = Config::from_upstream(&fake_upstream, config_path)?;

            // Override preset if explicitly specified (in case auto-detect didn't work)
            if let Some(ref preset_name) = preset {
                if config.preset.is_none() {
                    if let Some(preset_rules) = Config::load_preset_by_name(preset_name) {
                        for (k, v) in preset_rules {
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
            }

            let engine = FilterEngine::new(Arc::new(config));

            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .context("Failed to read from stdin")?;

            let input = input.trim();
            if input.is_empty() {
                anyhow::bail!("No input received on stdin. Pipe JSON into the command:\n  echo '{{\"key\": \"value\"}}' | mcp-rtk dry-run --tool <name>");
            }

            let filtered = engine.filter(tool, input);

            // Print stats to stderr so stdout is clean JSON
            let input_bytes = input.len();
            let output_bytes = filtered.len();
            let saved = input_bytes.saturating_sub(output_bytes);
            let pct = if input_bytes > 0 {
                (saved as f64 / input_bytes as f64) * 100.0
            } else {
                0.0
            };

            let pct_color = pct_to_color(pct);
            eprintln!();
            eprintln!("  {DIM}Tool:{RESET}    {BOLD}{tool}{RESET}");
            if let Some(ref p) = preset {
                eprintln!("  {DIM}Preset:{RESET}  {BOLD}{p}{RESET}");
            }
            eprintln!(
                "  {DIM}Input:{RESET}   {} bytes (~{} tokens)",
                input_bytes,
                input_bytes / 4
            );
            eprintln!(
                "  {DIM}Output:{RESET}  {} bytes (~{} tokens)",
                output_bytes,
                output_bytes / 4
            );
            eprintln!("  {DIM}Saved:{RESET}   {pct_color}{BOLD}{saved} bytes ({pct:.1}%){RESET}");
            eprintln!();

            // Pretty-print if valid JSON, otherwise raw
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&filtered) {
                println!("{}", serde_json::to_string_pretty(&parsed).unwrap());
            } else {
                println!("{filtered}");
            }

            return Ok(());
        }
        Some(Commands::Presets { action }) => {
            match action {
                PresetsAction::List => mcp_rtk::config::list_presets(),
                PresetsAction::Show { name } => mcp_rtk::config::show_preset(name)?,
            }
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

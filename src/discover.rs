//! Analyze Claude Code session logs to find MCP servers that would benefit
//! from mcp-rtk filtering.
//!
//! Scans `~/.claude/projects/*/` JSONL files, extracts MCP tool calls and
//! their responses, simulates filtering with available presets, and reports
//! estimated token savings.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::config::Config;
use crate::display::*;
use crate::filter::FilterEngine;
use crate::tracking::Tracker;
use std::sync::Arc;

/// Stats for a single MCP tool.
#[derive(Default)]
struct ToolStats {
    calls: usize,
    total_raw_bytes: usize,
    max_raw_bytes: usize,
}

/// Stats for a single MCP server.
#[derive(Default)]
struct ServerStats {
    tools: HashMap<String, ToolStats>,
    has_preset: bool,
    preset_name: Option<String>,
    already_proxied: bool,
}

/// Scan Claude Code session logs and report MCP savings opportunities.
pub fn run_discover(days: u32) -> Result<()> {
    let claude_dir = get_claude_projects_dir()?;

    println!();
    println!("  {BOLD}{GREEN}MCP-RTK{RESET}{DIM} — Discover Savings Opportunities{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(56));
    println!();

    let max_age_secs = days as u64 * 86400;
    let sessions = find_session_files(&claude_dir, max_age_secs)?;

    if sessions.is_empty() {
        println!("  {DIM}No Claude Code sessions found in the last {days} days.{RESET}");
        println!();
        return Ok(());
    }

    println!(
        "  {DIM}Scanning {count} sessions from the last {days} days...{RESET}",
        count = sessions.len()
    );
    println!();

    // Load tracker DB to detect already-proxied servers
    let tracked_presets = Tracker::new("~/.local/share/mcp-rtk/metrics.db")
        .and_then(|t| t.tracked_presets())
        .unwrap_or_default();

    // Build engines for each preset to simulate filtering
    let available_presets = Config::available_presets();
    let engines: HashMap<String, FilterEngine> = available_presets
        .iter()
        .filter_map(|name| {
            let fake_args = [format!("{name}-mcp")];
            let refs: Vec<&str> = fake_args.iter().map(|s| s.as_str()).collect();
            Config::from_upstream(&refs, None)
                .ok()
                .map(|c| (name.to_string(), FilterEngine::new(Arc::new(c))))
        })
        .collect();

    // Also build a generic engine for servers without presets
    let generic_config = Config::from_upstream(&["unknown-mcp"], None)?;
    let generic_engine = FilterEngine::new(Arc::new(generic_config));

    // Scan all sessions
    let mut servers: HashMap<String, ServerStats> = HashMap::new();
    let mut parse_errors: usize = 0;

    for session_path in &sessions {
        let errs = scan_session(session_path, &mut servers).unwrap_or(0);
        parse_errors += errs;
    }

    if parse_errors > 0 {
        tracing::debug!("Skipped {parse_errors} malformed JSONL lines across all sessions");
    }

    if servers.is_empty() {
        println!(
            "  {DIM}No MCP tool calls found in {} sessions.{RESET}",
            sessions.len()
        );
        println!();
        return Ok(());
    }

    // Mark servers that are already proxied via mcp-rtk
    for (server_name, stats) in &mut servers {
        detect_server_info(stats, server_name, &available_presets, &tracked_presets);
    }

    // Now simulate filtering for each server's tools
    let mut report: Vec<ServerReport> = Vec::new();

    for (server_name, stats) in &servers {
        // Determine the best preset for this server
        let (engine, preset_name) = match &stats.preset_name {
            Some(name) => {
                if let Some(e) = engines.get(name) {
                    (e, Some(name.as_str()))
                } else {
                    (&generic_engine, None)
                }
            }
            None => (&generic_engine, None),
        };

        let mut tool_reports: Vec<ToolReport> = Vec::new();
        let mut total_raw: usize = 0;
        let mut total_filtered: usize = 0;

        for (tool_name, tool_stats) in &stats.tools {
            total_raw += tool_stats.total_raw_bytes;
            let filtered_bytes = if stats.already_proxied {
                // Already going through mcp-rtk — no additional savings to report
                tool_stats.total_raw_bytes
            } else {
                estimate_filtered_size(engine, tool_name, tool_stats.total_raw_bytes)
            };
            total_filtered += filtered_bytes;

            let savings_pct = if tool_stats.total_raw_bytes > 0 {
                (1.0 - filtered_bytes as f64 / tool_stats.total_raw_bytes as f64) * 100.0
            } else {
                0.0
            };

            tool_reports.push(ToolReport {
                name: tool_name.clone(),
                calls: tool_stats.calls,
                raw_bytes: tool_stats.total_raw_bytes,
                savings_pct,
                max_response_bytes: tool_stats.max_raw_bytes,
            });
        }

        tool_reports.sort_by(|a, b| b.raw_bytes.cmp(&a.raw_bytes));

        let overall_savings = if total_raw > 0 {
            (1.0 - total_filtered as f64 / total_raw as f64) * 100.0
        } else {
            0.0
        };

        report.push(ServerReport {
            name: server_name.clone(),
            has_preset: stats.has_preset,
            preset_name: preset_name.map(str::to_string),
            already_proxied: stats.already_proxied,
            total_calls: stats.tools.values().map(|t| t.calls).sum(),
            total_raw_bytes: total_raw,
            estimated_savings_pct: overall_savings,
            tools: tool_reports,
        });
    }

    // Sort by potential savings (most bytes saveable first)
    report.sort_by(|a, b| {
        let a_saveable = (a.total_raw_bytes as f64 * a.estimated_savings_pct / 100.0) as usize;
        let b_saveable = (b.total_raw_bytes as f64 * b.estimated_savings_pct / 100.0) as usize;
        b_saveable.cmp(&a_saveable)
    });

    // Print report
    for server in &report {
        print_server_report(server);
    }

    // Summary
    let total_raw: usize = report.iter().map(|s| s.total_raw_bytes).sum();
    let unproxied_raw: usize = report
        .iter()
        .filter(|s| !s.already_proxied)
        .map(|s| s.total_raw_bytes)
        .sum();
    let unproxied_saveable: usize = report
        .iter()
        .filter(|s| !s.already_proxied)
        .map(|s| (s.total_raw_bytes as f64 * s.estimated_savings_pct / 100.0) as usize)
        .sum();

    println!("  {DIM}{}{RESET}", "─".repeat(56));
    println!(
        "  {BOLD}Total MCP traffic:{RESET}  {} tokens across {} servers",
        fmt_tokens(total_raw),
        report.len(),
    );
    if unproxied_raw > 0 {
        println!(
            "  {BOLD}{GREEN}Potential savings:{RESET}  {GREEN}~{} tokens{RESET} ({:.0}% of unproxied traffic)",
            fmt_tokens(unproxied_saveable),
            unproxied_saveable as f64 / unproxied_raw as f64 * 100.0,
        );
    }
    println!();

    // Actionable recommendations
    // Only recommend servers with meaningful traffic (>4K tokens = >16KB raw)
    let unproxied: Vec<&ServerReport> = report
        .iter()
        .filter(|s| !s.already_proxied && s.total_raw_bytes > 16_000)
        .collect();
    if !unproxied.is_empty() {
        println!("  {BOLD}Recommendations:{RESET}");
        println!();
        for server in &unproxied {
            let action = if server.has_preset {
                format!(
                    "Wrap with mcp-rtk (preset: {CYAN}{}{RESET})",
                    server.preset_name.as_deref().unwrap_or("generic")
                )
            } else {
                format!("Wrap with mcp-rtk {DIM}(generic filters, ~40% savings){RESET}")
            };
            let saveable =
                (server.total_raw_bytes as f64 * server.estimated_savings_pct / 100.0) as usize;
            println!(
                "  {YELLOW}→{RESET} {BOLD}{}{RESET}: {action}  {DIM}(~{} tokens/period){RESET}",
                server.name,
                fmt_tokens(saveable),
            );
        }
        println!();
    }

    Ok(())
}

struct ServerReport {
    name: String,
    has_preset: bool,
    preset_name: Option<String>,
    already_proxied: bool,
    total_calls: usize,
    total_raw_bytes: usize,
    estimated_savings_pct: f64,
    tools: Vec<ToolReport>,
}

struct ToolReport {
    name: String,
    calls: usize,
    raw_bytes: usize,
    savings_pct: f64,
    max_response_bytes: usize,
}

fn print_server_report(server: &ServerReport) {
    let status = if server.already_proxied {
        format!("{GREEN}✓ proxied{RESET}")
    } else if server.has_preset {
        format!("{YELLOW}● preset available{RESET}")
    } else {
        format!("{DIM}○ generic only{RESET}")
    };

    println!(
        "  {BOLD}{CYAN}{}{RESET}  {status}  {DIM}({} calls, {} tokens raw){RESET}",
        server.name,
        server.total_calls,
        fmt_tokens(server.total_raw_bytes),
    );

    if !server.already_proxied {
        let bar = render_block_bar(server.estimated_savings_pct / 100.0, 20);
        let color = pct_to_color(server.estimated_savings_pct);
        println!(
            "  {bar}  {color}{BOLD}{:.0}%{RESET} {DIM}estimated savings{RESET}",
            server.estimated_savings_pct,
        );
    }

    // Top 5 tools by traffic
    for tool in server.tools.iter().take(5) {
        let size_label = if tool.max_response_bytes > 10_000 {
            format!("{RED}large{RESET}")
        } else if tool.max_response_bytes > 2_000 {
            format!("{YELLOW}medium{RESET}")
        } else {
            format!("{DIM}small{RESET}")
        };

        let savings_label = if tool.savings_pct >= 60.0 {
            format!("{GREEN}{:>3.0}%{RESET}", tool.savings_pct)
        } else if tool.savings_pct >= 30.0 {
            format!("{YELLOW}{:>3.0}%{RESET}", tool.savings_pct)
        } else {
            format!("{DIM}{:>3.0}%{RESET}", tool.savings_pct)
        };

        println!(
            "    {DIM}├{RESET} {:<30} {:>4}x  {:>7} tokens  {size_label}  {savings_label}",
            truncate_name(&tool.name, 30),
            tool.calls,
            fmt_tokens(tool.raw_bytes),
        );
    }
    if server.tools.len() > 5 {
        println!(
            "    {DIM}└ ... and {} more tools{RESET}",
            server.tools.len() - 5
        );
    }

    println!();
}

/// Scan a single JSONL session file for MCP tool calls.
///
/// Returns the number of unparseable lines (for diagnostics).
fn scan_session(path: &Path, servers: &mut HashMap<String, ServerStats>) -> Result<usize> {
    let file = std::fs::File::open(path).context("open session")?;
    let reader = std::io::BufReader::new(file);
    let mut parse_errors: usize = 0;

    // Track tool_use_id -> (server_name, tool_name) for matching results
    let mut pending_calls: HashMap<String, (String, String)> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                parse_errors += 1;
                continue;
            }
        };
        if line.is_empty() {
            continue;
        }

        let entry: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_errors += 1;
                continue;
            }
        };

        // Look for messages with content arrays
        let content = extract_content(&entry);
        if content.is_empty() {
            continue;
        }

        for item in content {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("tool_use") => {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        if name.starts_with("mcp__") {
                            let tool_use_id = item
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Parse server name from mcp__servername__toolname
                            let parts: Vec<&str> = name.splitn(3, "__").collect();
                            if parts.len() == 3 {
                                let server_name = parts[1].to_string();
                                let tool_name = parts[2].to_string();
                                pending_calls.insert(tool_use_id, (server_name, tool_name));
                            }
                        }
                    }
                }
                Some("tool_result") => {
                    let tool_use_id = item
                        .get("tool_use_id")
                        .and_then(|id| id.as_str())
                        .unwrap_or("")
                        .to_string();

                    if let Some((server_name, tool_name)) = pending_calls.remove(&tool_use_id) {
                        let result_bytes = measure_result_size(item);
                        if result_bytes > 0 {
                            let tool_stats = servers
                                .entry(server_name)
                                .or_default()
                                .tools
                                .entry(tool_name)
                                .or_default();
                            tool_stats.calls += 1;
                            tool_stats.total_raw_bytes += result_bytes;
                            if result_bytes > tool_stats.max_raw_bytes {
                                tool_stats.max_raw_bytes = result_bytes;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(parse_errors)
}

/// Extract content array from various JSONL entry shapes.
fn extract_content(entry: &Value) -> Vec<&Value> {
    let mut items = Vec::new();

    // Direct message content
    if let Some(content) = entry
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    {
        items.extend(content.iter());
    }

    // Nested in data.message.message.content (agent progress)
    if let Some(content) = entry
        .get("data")
        .and_then(|d| d.get("message"))
        .and_then(|m| m.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    {
        items.extend(content.iter());
    }

    // Nested in data.message.content (user tool_result)
    // Only if the deeper path (data.message.message.content) didn't already match
    if items.is_empty() {
        if let Some(content) = entry
            .get("data")
            .and_then(|d| d.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            items.extend(content.iter());
        }
    }

    items
}

/// Measure the size of a tool_result's text content.
fn measure_result_size(item: &Value) -> usize {
    let mut total = 0;

    // tool_result content can be a string or array of content blocks
    if let Some(content) = item.get("content") {
        match content {
            Value::String(s) => {
                total += s.len();
            }
            Value::Array(arr) => {
                for block in arr {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            total += text.len();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    total
}

/// Detect if a server has an available preset and if it's already proxied.
///
/// A server is considered "already proxied" when the mcp-rtk tracker DB
/// contains entries for its preset name — meaning `mcp-rtk gain` has data.
fn detect_server_info(
    stats: &mut ServerStats,
    server_name: &str,
    available_presets: &[String],
    tracked_presets: &HashSet<String>,
) {
    for preset in available_presets {
        if server_name.contains(preset.as_str()) {
            stats.has_preset = true;
            stats.preset_name = Some(preset.to_string());
            stats.already_proxied = tracked_presets.contains(preset);
            return;
        }
    }
    // No preset match — check if the server name itself appears in tracker
    stats.already_proxied = tracked_presets.contains(server_name);
}

/// Estimate filtered size for a tool based on generic/preset filter rules.
///
/// Uses a heuristic: preset tools save 60-80%, generic saves 30-50%.
fn estimate_filtered_size(engine: &FilterEngine, tool_name: &str, raw_bytes: usize) -> usize {
    // Conservative estimate based on typical savings
    // With preset: ~65% savings, generic: ~40% savings
    let config = engine.config();
    let has_tool_rules = config.filters.tools.contains_key(tool_name);

    let savings_ratio = if has_tool_rules {
        0.65 // Tool-specific rules: 65% savings
    } else {
        0.40 // Generic defaults only: 40% savings
    };

    ((1.0 - savings_ratio) * raw_bytes as f64) as usize
}

/// Find all session JSONL files modified within `max_age_secs` seconds.
fn find_session_files(claude_dir: &Path, max_age_secs: u64) -> Result<Vec<PathBuf>> {
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    if !claude_dir.exists() {
        return Ok(Vec::new());
    }

    let now = std::time::SystemTime::now();

    for entry in std::fs::read_dir(claude_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let project_dir = entry.path();
        if let Ok(entries) = std::fs::read_dir(&project_dir) {
            for file_entry in entries.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    if let Ok(metadata) = path.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            let age = now.duration_since(modified).unwrap_or_default();
                            if age.as_secs() < max_age_secs {
                                files.push((path, modified));
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort by modification time (newest first), reusing stored timestamps
    files.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(files.into_iter().map(|(path, _)| path).collect())
}

fn get_claude_projects_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("Neither HOME nor USERPROFILE is set")?;
    Ok(PathBuf::from(home).join(".claude").join("projects"))
}

/// Wrapper: discover uses usize, shared display uses i64.
fn fmt_tokens(bytes: usize) -> String {
    format_tokens(bytes as i64)
}

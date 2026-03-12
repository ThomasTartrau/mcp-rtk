//! SQLite-backed token savings metrics.
//!
//! The [`Tracker`] records every tool call's raw and filtered byte sizes in a
//! local SQLite database. The `mcp-rtk gain` subcommand reads these metrics to
//! display a colorful summary with per-tool breakdowns and an efficiency meter.
//!
//! # Database Schema
//!
//! ```sql
//! CREATE TABLE tool_calls (
//!     id INTEGER PRIMARY KEY AUTOINCREMENT,
//!     timestamp TEXT DEFAULT (datetime('now')),
//!     tool_name TEXT NOT NULL,
//!     input_bytes INTEGER NOT NULL,
//!     output_bytes INTEGER NOT NULL,
//!     saved_bytes INTEGER NOT NULL,
//!     savings_pct REAL NOT NULL
//! );
//! ```

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::display::*;

/// SQLite-backed tracker for recording and displaying token savings metrics.
///
/// Thread-safe via an internal `Mutex<Connection>`, satisfying the `Sync`
/// requirement of [`ServerHandler`](rmcp::handler::server::ServerHandler).
///
/// # Examples
///
/// ```no_run
/// # use mcp_rtk::tracking::Tracker;
/// let tracker = Tracker::new("~/.local/share/mcp-rtk/metrics.db").unwrap();
/// tracker.track("list_merge_requests", "{...raw...}", "{...filtered...}", "gitlab").unwrap();
/// tracker.print_stats().unwrap();
/// ```
pub struct Tracker {
    conn: Mutex<Connection>,
}

impl Tracker {
    /// Open or create the tracking database at the given path.
    ///
    /// Supports `~/` expansion. Creates parent directories if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the database directory cannot be created or the
    /// SQLite connection fails to open.
    pub fn new(db_path: &str) -> Result<Self> {
        let expanded = expand_path(db_path);
        if let Some(parent) = expanded.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create tracking database directory")?;
        }
        let conn = Connection::open(&expanded).context("Failed to open tracking database")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT DEFAULT (datetime('now')),
                tool_name TEXT NOT NULL,
                input_bytes INTEGER NOT NULL,
                output_bytes INTEGER NOT NULL,
                saved_bytes INTEGER NOT NULL,
                savings_pct REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tool_calls_timestamp ON tool_calls(timestamp);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_tool ON tool_calls(tool_name);",
        )
        .context("Failed to initialize tracking tables")?;

        // Migration: add preset column if missing
        let has_preset: bool = conn
            .prepare("SELECT preset FROM tool_calls LIMIT 0")
            .is_ok();
        if !has_preset {
            conn.execute_batch(
                "ALTER TABLE tool_calls ADD COLUMN preset TEXT NOT NULL DEFAULT 'unknown';",
            )
            .context("Failed to add preset column")?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record a single tool call's raw and filtered output sizes.
    ///
    /// Token count is estimated as `bytes / 4`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database lock is poisoned or the insert fails.
    pub fn track(
        &self,
        tool_name: &str,
        raw_output: &str,
        filtered_output: &str,
        preset: &str,
    ) -> Result<()> {
        let input_bytes = raw_output.len() as i64;
        let output_bytes = filtered_output.len() as i64;
        // Clamp to zero: filtered output can rarely exceed raw when
        // JSON re-serialization or custom transforms add characters.
        let saved_bytes = (input_bytes - output_bytes).max(0);
        let savings_pct = if input_bytes > 0 {
            (saved_bytes as f64 / input_bytes as f64) * 100.0
        } else {
            0.0
        };

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        conn.execute(
            "INSERT INTO tool_calls (tool_name, input_bytes, output_bytes, saved_bytes, savings_pct, preset)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![tool_name, input_bytes, output_bytes, saved_bytes, savings_pct, preset],
        )?;

        Ok(())
    }

    /// Print a colorful summary of all-time token savings to stdout.
    ///
    /// Includes an efficiency meter bar and a per-tool breakdown table with
    /// impact bars.
    ///
    /// # Errors
    ///
    /// Returns an error if the database lock is poisoned or query fails.
    pub fn print_stats(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;

        // Fetch per-tool stats grouped by preset
        let mut stmt = conn.prepare(
            "SELECT
                preset,
                tool_name,
                COUNT(*) as calls,
                SUM(input_bytes) as total_input,
                SUM(output_bytes) as total_output,
                SUM(saved_bytes) as total_saved,
                AVG(savings_pct) as avg_pct
             FROM tool_calls
             GROUP BY preset, tool_name
             ORDER BY preset, total_saved DESC",
        )?;

        struct ToolRow {
            preset: String,
            name: String,
            calls: i64,
            saved: i64,
            avg_pct: f64,
        }

        let rows: Vec<ToolRow> = stmt
            .query_map([], |row| {
                Ok(ToolRow {
                    preset: row.get(0)?,
                    name: row.get(1)?,
                    calls: row.get(2)?,
                    saved: row.get(5)?,
                    avg_pct: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let grand_calls: i64 = rows.iter().map(|r| r.calls).sum();
        let grand_input: i64 = conn.query_row(
            "SELECT COALESCE(SUM(input_bytes), 0) FROM tool_calls",
            [],
            |row| row.get(0),
        )?;
        let grand_saved: i64 = rows.iter().map(|r| r.saved).sum();
        let grand_output = grand_input - grand_saved;
        let grand_pct = if grand_input > 0 {
            (grand_saved as f64 / grand_input as f64) * 100.0
        } else {
            0.0
        };

        let saved_tokens = grand_saved / 4;

        // ── Header ──────────────────────────────────────────
        println!();
        println!("  {BOLD}{GREEN}MCP-RTK{RESET}{DIM} - Token Savings{RESET}");
        println!("  {DIM}{}{RESET}", "─".repeat(56));
        println!();

        // ── Summary (two columns) ───────────────────────────
        println!(
            "  {DIM}Calls{RESET}  {BOLD}{WHITE}{:<12}{RESET}  {DIM}Input{RESET}   {WHITE}{} tokens{RESET}",
            grand_calls,
            format_number(grand_input / 4),
        );
        println!(
            "  {DIM}Saved{RESET}  {BOLD}{GREEN}{:<12}{RESET}  {DIM}Output{RESET}  {WHITE}{} tokens{RESET}",
            format!("{} ({:.0}%)", format_number(saved_tokens), grand_pct),
            format_number(grand_output / 4),
        );
        println!();

        // ── Efficiency bar ──────────────────────────────────
        let bar_width: usize = 40;
        let bar = render_block_bar(grand_pct / 100.0, bar_width);
        let pct_color = pct_to_color(grand_pct);
        println!("  {bar}  {pct_color}{BOLD}{:.1}%{RESET}", grand_pct);
        println!();

        // ── Per-tool table ──────────────────────────────────
        if rows.is_empty() {
            println!("  {DIM}No tool calls recorded yet.{RESET}");
            println!();
            return Ok(());
        }

        // Collect unique presets in insertion order
        let mut seen = std::collections::HashSet::new();
        let mut presets: Vec<String> = Vec::new();
        for row in &rows {
            if seen.insert(row.preset.clone()) {
                presets.push(row.preset.clone());
            }
        }

        let max_saved = rows.iter().map(|r| r.saved).max().unwrap_or(1).max(1);

        for preset in &presets {
            let preset_rows: Vec<&ToolRow> = rows.iter().filter(|r| &r.preset == preset).collect();
            let preset_saved: i64 = preset_rows.iter().map(|r| r.saved).sum();
            let preset_calls: i64 = preset_rows.iter().map(|r| r.calls).sum();

            println!(
                "  {DIM}─── {RESET}{BOLD}{}{RESET}{DIM} ({} calls, {} saved) {}─{RESET}",
                preset,
                preset_calls,
                format_tokens(preset_saved),
                "─".repeat(30usize.saturating_sub(preset.len())),
            );
            println!();
            println!(
                "  {DIM}{:<28} {:>5}  {:>8}  {:>5}{RESET}",
                "Tool", "Count", "Saved", "Avg%"
            );
            println!();

            for row in &preset_rows {
                let pct_color = pct_to_color(row.avg_pct);
                let bar_ratio = row.saved as f64 / max_saved as f64;
                let inline_bar = render_block_bar(bar_ratio, 16);

                println!(
                    "  {BOLD}{WHITE}{:<28}{RESET} {:>5}  {:>8}  {pct_color}{:>4.0}%{RESET}  {inline_bar}",
                    truncate_name(&row.name, 28),
                    row.calls,
                    format_tokens(row.saved),
                    row.avg_pct,
                );
            }

            println!();
        }

        println!();
        Ok(())
    }

    /// Print the last 50 tool calls with timestamps and savings percentages.
    ///
    /// # Errors
    ///
    /// Returns an error if the database lock is poisoned or query fails.
    pub fn print_history(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, tool_name, input_bytes, output_bytes, savings_pct, preset
             FROM tool_calls
             ORDER BY timestamp DESC
             LIMIT 50",
        )?;

        let rows: Vec<(String, String, i64, i64, f64, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        println!();
        println!("  {BOLD}{GREEN}MCP-RTK{RESET}{DIM} ── Recent Calls{RESET}");
        println!("  {DIM}{}{RESET}", "─".repeat(76));
        println!();

        if rows.is_empty() {
            println!("  {DIM}No tool calls recorded yet.{RESET}");
            println!();
            return Ok(());
        }

        println!(
            "  {DIM}{:<19} {:<8} {:<22} {:>7} {:>7} {:>6}{RESET}",
            "Timestamp", "Preset", "Tool", "In", "Out", "Saved"
        );
        println!();

        for (ts, name, input, output, pct, preset) in &rows {
            let pct_color = pct_to_color(*pct);
            let saved_bytes = input - output;

            println!(
                "  {DIM}{:<19}{RESET} {YELLOW}{:<8}{RESET} {WHITE}{:<22}{RESET} {:>7} {:>7} {pct_color}{BOLD}{:>5.0}%{RESET}  {DIM}{}{RESET}",
                ts.get(..19).unwrap_or(ts),
                truncate_name(preset, 8),
                truncate_name(name, 22),
                format_tokens(*input),
                format_tokens(*output),
                pct,
                if saved_bytes > 0 {
                    format!("-{} tk", format_tokens(saved_bytes))
                } else {
                    String::new()
                },
            );
        }

        println!();
        Ok(())
    }

    /// Return the set of preset names that have tracking data.
    ///
    /// Used by `discover` to detect which servers are already proxied.
    pub fn tracked_presets(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
        let mut stmt =
            conn.prepare("SELECT DISTINCT preset FROM tool_calls WHERE preset != 'unknown'")?;
        let presets: std::collections::HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(presets)
    }
}

/// Expand `~/` prefix to the user's home directory.
fn expand_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

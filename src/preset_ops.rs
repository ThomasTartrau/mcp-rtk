//! Preset operations: interactive init and remote pull.

use std::io::{self, BufRead, Write as IoWrite};
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::display::*;

// ── Preset Init ──────────────────────────────────────────────────────

/// Run the interactive preset scaffolding.
pub fn run_preset_init(output: Option<&str>) -> Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stderr = io::stderr();
    let mut out = stderr.lock();

    writeln!(
        out,
        "\n  {BOLD}{GREEN}mcp-rtk{RESET}{DIM} — preset init{RESET}"
    )?;
    writeln!(out, "  {DIM}{}{RESET}\n", "─".repeat(56))?;

    // Preset name
    let name = prompt(&mut reader, &mut out, "Preset name (e.g. github)")?;
    if name.is_empty() {
        anyhow::bail!("Preset name cannot be empty");
    }

    // Detection keywords
    let keywords_raw = prompt(
        &mut reader,
        &mut out,
        "Detection keywords, comma-separated (e.g. github-mcp, github)",
    )?;
    let keywords: Vec<&str> = keywords_raw
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    // Tools
    writeln!(
        out,
        "\n  {DIM}Enter tool names, one per line. Empty line to finish.{RESET}"
    )?;

    let mut tools: Vec<ToolSpec> = Vec::new();
    loop {
        let tool_name = prompt(&mut reader, &mut out, "Tool name (empty to finish)")?;
        if tool_name.is_empty() {
            break;
        }

        let keep_raw = prompt(
            &mut reader,
            &mut out,
            &format!("  {tool_name}: keep_fields (comma-separated, empty to skip)"),
        )?;
        let keep_fields: Vec<String> = keep_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let truncate_raw = prompt(
            &mut reader,
            &mut out,
            &format!("  {tool_name}: truncate_strings_at (empty for default)"),
        )?;
        let truncate: Option<usize> = truncate_raw.parse().ok();

        let max_items_raw = prompt(
            &mut reader,
            &mut out,
            &format!("  {tool_name}: max_array_items (empty for default)"),
        )?;
        let max_items: Option<usize> = max_items_raw.parse().ok();

        let condense_raw = prompt(
            &mut reader,
            &mut out,
            &format!("  {tool_name}: condense_users? (y/n, empty for default)"),
        )?;
        let condense = match condense_raw.to_lowercase().as_str() {
            "y" | "yes" => Some(true),
            "n" | "no" => Some(false),
            _ => None,
        };

        // Validate tool name: alphanumeric, underscore, hyphen only
        if !tool_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            writeln!(
                out,
                "  {RED}✗{RESET} Invalid tool name (alphanumeric, _, - only). Skipping."
            )?;
            continue;
        }

        tools.push(ToolSpec {
            name: tool_name,
            keep_fields,
            truncate_strings_at: truncate,
            max_array_items: max_items,
            condense_users: condense,
        });
    }

    // Generate TOML
    let toml = generate_toml(&name, &keywords, &tools);

    // Determine output path
    let output_path = if let Some(p) = output {
        PathBuf::from(p)
    } else {
        PathBuf::from(format!("{name}.toml"))
    };

    std::fs::write(&output_path, &toml)
        .context(format!("Failed to write {}", output_path.display()))?;

    writeln!(
        out,
        "\n  {GREEN}✓{RESET} Preset written to {BOLD}{}{RESET}",
        output_path.display()
    )?;
    writeln!(
        out,
        "  {DIM}Validate with: mcp-rtk validate-preset {}{RESET}",
        output_path.display()
    )?;
    writeln!(
        out,
        "  {DIM}Test with:     echo '{{}}' | mcp-rtk dry-run --config {} --tool <name>{RESET}\n",
        output_path.display()
    )?;

    Ok(())
}

struct ToolSpec {
    name: String,
    keep_fields: Vec<String>,
    truncate_strings_at: Option<usize>,
    max_array_items: Option<usize>,
    condense_users: Option<bool>,
}

fn generate_toml(name: &str, keywords: &[&str], tools: &[ToolSpec]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {name} preset for mcp-rtk\n"));
    out.push_str(&format!(
        "# Auto-detected from: {}\n\n",
        keywords.join(", ")
    ));

    for tool in tools {
        out.push_str(&format!("[tools.{}]\n", tool.name));
        if !tool.keep_fields.is_empty() {
            out.push_str(&format!(
                "keep_fields = [{}]\n",
                tool.keep_fields
                    .iter()
                    .map(|f| format!("\"{f}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(n) = tool.truncate_strings_at {
            out.push_str(&format!("truncate_strings_at = {n}\n"));
        }
        if let Some(n) = tool.max_array_items {
            out.push_str(&format!("max_array_items = {n}\n"));
        }
        if let Some(c) = tool.condense_users {
            out.push_str(&format!("condense_users = {c}\n"));
        }
        out.push('\n');
    }

    out
}

fn prompt(reader: &mut impl BufRead, out: &mut impl IoWrite, label: &str) -> Result<String> {
    write!(out, "  {CYAN}?{RESET} {label}: ")?;
    out.flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

// ── Preset Pull ──────────────────────────────────────────────────────

/// Fetch a preset TOML from a URL and save it locally.
pub fn run_preset_pull(url: &str, output: Option<&str>) -> Result<()> {
    let stderr = io::stderr();
    let mut out = stderr.lock();

    writeln!(
        out,
        "\n  {BOLD}{GREEN}mcp-rtk{RESET}{DIM} — preset pull{RESET}"
    )?;
    writeln!(out, "  {DIM}{}{RESET}\n", "─".repeat(56))?;
    writeln!(out, "  {DIM}Fetching:{RESET} {url}")?;

    let content = fetch_url(url)?;

    // Validate it's a valid preset TOML
    let preset: crate::config::PresetConfig =
        toml::from_str(&content).context("Downloaded file is not a valid preset TOML")?;

    // Determine output path
    let output_path = if let Some(p) = output {
        PathBuf::from(p)
    } else {
        let filename = url_to_filename(url);
        let dir = crate::config::external_presets_dir()?;
        dir.join(filename)
    };

    std::fs::write(&output_path, &content)
        .context(format!("Failed to write {}", output_path.display()))?;

    writeln!(
        out,
        "  {GREEN}✓{RESET} Saved to {BOLD}{}{RESET}",
        output_path.display()
    )?;
    writeln!(out, "  {DIM}Tools:{RESET}   {}", preset.tools.len())?;
    for name in preset.tools.keys() {
        writeln!(out, "    {DIM}•{RESET} {name}")?;
    }
    writeln!(
        out,
        "\n  {DIM}Use with: mcp-rtk --config {} -- <command>{RESET}",
        output_path.display()
    )?;
    writeln!(
        out,
        "  {DIM}Validate: mcp-rtk validate-preset {}{RESET}\n",
        output_path.display()
    )?;

    Ok(())
}

fn fetch_url(url: &str) -> Result<String> {
    use std::process::Command;

    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "30",
            "--max-filesize",
            "1048576",
            "--proto",
            "=https",
            "--",
            url,
        ])
        .output()
        .context("Failed to run curl. Is curl installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to fetch {url}: {stderr}");
    }

    String::from_utf8(output.stdout).context("Response is not valid UTF-8")
}

fn url_to_filename(url: &str) -> String {
    let base = url.rsplit('/').next().unwrap_or("preset.toml");
    // Sanitize: strip path separators, reject traversal
    let sanitized: String = base.chars().filter(|c| *c != '/' && *c != '\\').collect();
    if sanitized.is_empty() || sanitized.contains("..") {
        return "preset.toml".to_string();
    }
    if sanitized.ends_with(".toml") {
        sanitized
    } else {
        format!("{sanitized}.toml")
    }
}

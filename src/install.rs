//! Install and uninstall mcp-rtk wrapping in MCP JSON config files.
//!
//! Supports three config formats:
//! - `.mcp.json` / `mcp.json` (Claude Code) — top-level `mcpServers`
//! - `claude_desktop_config.json` (Claude Desktop) — top-level `mcpServers`
//! - `~/.claude.json` (Claude Code user scope) — top-level `mcpServers`
//!   **and** per-project `projects.<path>.mcpServers`
//!
//! Uses atomic writes and backups to avoid corrupting the user's config.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::display::*;

const MCP_RTK_BIN: &str = "mcp-rtk";

// ── File I/O ────────────────────────────────────────────────────────

fn read_config(path: &Path) -> Result<(String, Value)> {
    let content =
        std::fs::read_to_string(path).context(format!("Failed to read {}", path.display()))?;
    let value: Value =
        serde_json::from_str(&content).context(format!("Invalid JSON in {}", path.display()))?;
    Ok((content, value))
}

fn create_backup(path: &Path) -> Result<PathBuf> {
    let backup = path.with_extension("json.bak");
    std::fs::copy(path, &backup)
        .context(format!("Failed to create backup at {}", backup.display()))?;
    Ok(backup)
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, content).context("Failed to write temporary file")?;
    std::fs::rename(&temp, path).context("Failed to rename temporary file to target")?;
    Ok(())
}

// ── Indentation ─────────────────────────────────────────────────────

fn detect_indent(content: &str) -> String {
    for line in content.lines() {
        if line.starts_with('\t') {
            return "\t".to_string();
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if spaces > 0 {
            return " ".repeat(spaces);
        }
    }
    "  ".to_string()
}

fn serialize_with_indent(value: &Value, indent: &str, trailing_newline: bool) -> String {
    let pretty = serde_json::to_string_pretty(value).unwrap();
    if indent == "  " {
        return if trailing_newline {
            format!("{pretty}\n")
        } else {
            pretty
        };
    }
    let mut result = String::with_capacity(pretty.len());
    for (i, line) in pretty.lines().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        let stripped = line.trim_start();
        let leading_spaces = line.len() - stripped.len();
        let indent_level = leading_spaces / 2;
        for _ in 0..indent_level {
            result.push_str(indent);
        }
        result.push_str(stripped);
    }
    if trailing_newline {
        result.push('\n');
    }
    result
}

// ── Server classification ───────────────────────────────────────────

fn is_stdio_server(entry: &Value) -> bool {
    match entry.get("type").and_then(|t| t.as_str()) {
        None | Some("stdio") => entry.get("command").is_some(),
        _ => false,
    }
}

fn is_already_wrapped(entry: &Value) -> bool {
    entry.get("command").and_then(|c| c.as_str()) == Some(MCP_RTK_BIN)
}

// ── Wrap / Unwrap ───────────────────────────────────────────────────

fn wrap_server(entry: &mut Value) -> bool {
    let command = match entry.get("command").and_then(|c| c.as_str()) {
        Some(c) => c.to_string(),
        None => return false,
    };
    let existing_args: Vec<Value> = entry
        .get("args")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    let mut new_args: Vec<Value> = vec![Value::String("--".to_string()), Value::String(command)];
    new_args.extend(existing_args);

    entry["command"] = Value::String(MCP_RTK_BIN.to_string());
    entry["args"] = Value::Array(new_args);
    true
}

fn unwrap_server(entry: &mut Value) -> bool {
    let args: Vec<String> = match entry.get("args").and_then(|a| a.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        None => return false,
    };

    let separator_idx = match args.iter().position(|a| a == "--") {
        Some(idx) => idx,
        None => return false,
    };

    let after_separator = &args[separator_idx + 1..];
    if after_separator.is_empty() {
        return false;
    }

    let original_command = after_separator[0].clone();
    let original_args: Vec<Value> = after_separator[1..]
        .iter()
        .map(|s| Value::String(s.clone()))
        .collect();

    entry["command"] = Value::String(original_command);
    if original_args.is_empty() {
        if let Some(obj) = entry.as_object_mut() {
            obj.remove("args");
        }
    } else {
        entry["args"] = Value::Array(original_args);
    }
    true
}

// ── Reporting structs ───────────────────────────────────────────────

struct InstallReport {
    wrapped: Vec<String>,
    skipped_already: Vec<String>,
    skipped_non_stdio: Vec<String>,
}

impl InstallReport {
    fn new() -> Self {
        Self {
            wrapped: Vec::new(),
            skipped_already: Vec::new(),
            skipped_non_stdio: Vec::new(),
        }
    }

    fn has_changes(&self) -> bool {
        !self.wrapped.is_empty()
    }

    fn has_any_match(&self) -> bool {
        !self.wrapped.is_empty()
            || !self.skipped_already.is_empty()
            || !self.skipped_non_stdio.is_empty()
    }
}

struct UninstallReport {
    unwrapped: Vec<String>,
    skipped_not_wrapped: Vec<String>,
}

impl UninstallReport {
    fn new() -> Self {
        Self {
            unwrapped: Vec::new(),
            skipped_not_wrapped: Vec::new(),
        }
    }

    fn has_changes(&self) -> bool {
        !self.unwrapped.is_empty()
    }

    fn has_any_match(&self) -> bool {
        !self.unwrapped.is_empty() || !self.skipped_not_wrapped.is_empty()
    }
}

// ── Process a single mcpServers map ─────────────────────────────────

fn install_servers(
    servers: &mut serde_json::Map<String, Value>,
    server_filter: Option<&str>,
    scope: &str,
    report: &mut InstallReport,
) {
    let server_names: Vec<String> = servers.keys().cloned().collect();

    for name in server_names {
        if let Some(filter) = server_filter {
            if name != filter {
                continue;
            }
        }
        let entry = servers.get_mut(&name).unwrap();
        let display_name = if scope.is_empty() {
            name.clone()
        } else {
            format!("{name} ({scope})")
        };

        if !is_stdio_server(entry) {
            report.skipped_non_stdio.push(display_name);
            continue;
        }
        if is_already_wrapped(entry) {
            report.skipped_already.push(display_name);
            continue;
        }
        if wrap_server(entry) {
            report.wrapped.push(display_name);
        }
    }
}

fn uninstall_servers(
    servers: &mut serde_json::Map<String, Value>,
    server_filter: Option<&str>,
    scope: &str,
    report: &mut UninstallReport,
) {
    let server_names: Vec<String> = servers.keys().cloned().collect();

    for name in server_names {
        if let Some(filter) = server_filter {
            if name != filter {
                continue;
            }
        }
        let entry = servers.get_mut(&name).unwrap();
        let display_name = if scope.is_empty() {
            name.clone()
        } else {
            format!("{name} ({scope})")
        };

        if !is_already_wrapped(entry) {
            report.skipped_not_wrapped.push(display_name);
            continue;
        }
        if unwrap_server(entry) {
            report.unwrapped.push(display_name);
        }
    }
}

// ── Collect all mcpServers locations ────────────────────────────────

/// Collect all server names across top-level and projects scopes.
fn collect_all_server_names(config: &Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(servers) = config.get("mcpServers").and_then(|s| s.as_object()) {
        names.extend(servers.keys().cloned());
    }
    if let Some(projects) = config.get("projects").and_then(|p| p.as_object()) {
        for (_path, project) in projects {
            if let Some(servers) = project.get("mcpServers").and_then(|s| s.as_object()) {
                names.extend(servers.keys().cloned());
            }
        }
    }
    names
}

// ── Public entry points ─────────────────────────────────────────────

pub fn run_install(path: &Path, server_filter: Option<&str>) -> Result<()> {
    let (raw_content, mut config) = read_config(path)?;
    let indent = detect_indent(&raw_content);
    let trailing_newline = raw_content.ends_with('\n');

    let all_names = collect_all_server_names(&config);
    let mut report = InstallReport::new();

    // Process top-level mcpServers
    if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        install_servers(servers, server_filter, "", &mut report);
    }

    // Process projects.<path>.mcpServers (claude.json format)
    if let Some(projects) = config.get_mut("projects").and_then(|p| p.as_object_mut()) {
        let project_paths: Vec<String> = projects.keys().cloned().collect();
        for project_path in project_paths {
            if let Some(servers) = projects
                .get_mut(&project_path)
                .and_then(|p| p.get_mut("mcpServers"))
                .and_then(|s| s.as_object_mut())
            {
                install_servers(servers, server_filter, &project_path, &mut report);
            }
        }
    }

    // If filtering by name and nothing found anywhere
    if let Some(filter) = server_filter {
        if !report.has_any_match() {
            anyhow::bail!(
                "Server '{}' not found. Available: {}",
                filter,
                all_names.join(", ")
            );
        }
    }

    // No mcpServers found at all
    if all_names.is_empty() {
        anyhow::bail!("No 'mcpServers' found in {}", path.display());
    }

    if !report.has_changes() {
        eprintln!();
        eprintln!("  {DIM}No servers to wrap.{RESET}");
        for name in &report.skipped_already {
            eprintln!("  {DIM}○ {name} — already wrapped{RESET}");
        }
        for name in &report.skipped_non_stdio {
            eprintln!("  {DIM}○ {name} — non-stdio transport{RESET}");
        }
        eprintln!();
        return Ok(());
    }

    let output = serialize_with_indent(&config, &indent, trailing_newline);
    serde_json::from_str::<Value>(&output).context("Internal error: generated invalid JSON")?;

    let backup_path = create_backup(path)?;
    atomic_write(path, &output)?;

    eprintln!();
    eprintln!("  {BOLD}{GREEN}mcp-rtk{RESET}{DIM} — install{RESET}");
    eprintln!("  {DIM}{}{RESET}", "─".repeat(56));
    eprintln!();
    eprintln!("  {DIM}Config:{RESET}  {}", path.display());
    eprintln!("  {DIM}Backup:{RESET}  {}", backup_path.display());
    eprintln!();
    for name in &report.wrapped {
        eprintln!("  {GREEN}✓{RESET} {BOLD}{name}{RESET} wrapped with mcp-rtk");
    }
    for name in &report.skipped_already {
        eprintln!("  {DIM}○ {name} — already wrapped{RESET}");
    }
    for name in &report.skipped_non_stdio {
        eprintln!("  {DIM}○ {name} — non-stdio transport{RESET}");
    }
    eprintln!();

    Ok(())
}

pub fn run_uninstall(path: &Path, server_filter: Option<&str>) -> Result<()> {
    let (raw_content, mut config) = read_config(path)?;
    let indent = detect_indent(&raw_content);
    let trailing_newline = raw_content.ends_with('\n');

    let all_names = collect_all_server_names(&config);
    let mut report = UninstallReport::new();

    // Process top-level mcpServers
    if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        uninstall_servers(servers, server_filter, "", &mut report);
    }

    // Process projects.<path>.mcpServers (claude.json format)
    if let Some(projects) = config.get_mut("projects").and_then(|p| p.as_object_mut()) {
        let project_paths: Vec<String> = projects.keys().cloned().collect();
        for project_path in project_paths {
            if let Some(servers) = projects
                .get_mut(&project_path)
                .and_then(|p| p.get_mut("mcpServers"))
                .and_then(|s| s.as_object_mut())
            {
                uninstall_servers(servers, server_filter, &project_path, &mut report);
            }
        }
    }

    if let Some(filter) = server_filter {
        if !report.has_any_match() {
            anyhow::bail!(
                "Server '{}' not found. Available: {}",
                filter,
                all_names.join(", ")
            );
        }
    }

    if all_names.is_empty() {
        anyhow::bail!("No 'mcpServers' found in {}", path.display());
    }

    if !report.has_changes() {
        eprintln!();
        eprintln!("  {DIM}No servers to unwrap.{RESET}");
        for name in &report.skipped_not_wrapped {
            eprintln!("  {DIM}○ {name} — not wrapped by mcp-rtk{RESET}");
        }
        eprintln!();
        return Ok(());
    }

    let output = serialize_with_indent(&config, &indent, trailing_newline);
    serde_json::from_str::<Value>(&output).context("Internal error: generated invalid JSON")?;

    let backup_path = create_backup(path)?;
    atomic_write(path, &output)?;

    eprintln!();
    eprintln!("  {BOLD}{GREEN}mcp-rtk{RESET}{DIM} — uninstall{RESET}");
    eprintln!("  {DIM}{}{RESET}", "─".repeat(56));
    eprintln!();
    eprintln!("  {DIM}Config:{RESET}  {}", path.display());
    eprintln!("  {DIM}Backup:{RESET}  {}", backup_path.display());
    eprintln!();
    for name in &report.unwrapped {
        eprintln!("  {GREEN}✓{RESET} {BOLD}{name}{RESET} restored to original");
    }
    for name in &report.skipped_not_wrapped {
        eprintln!("  {DIM}○ {name} — not wrapped by mcp-rtk{RESET}");
    }
    eprintln!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_config(dir: &TempDir, content: &str) -> PathBuf {
        let path = dir.path().join("config.json");
        fs::write(&path, content).unwrap();
        path
    }

    fn read_back(path: &Path) -> Value {
        let content = fs::read_to_string(path).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    // ── .mcp.json / claude_desktop_config.json (top-level mcpServers) ──

    #[test]
    fn install_wraps_stdio_servers() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "npx",
      "args": ["-y", "@nicepkg/gitlab-mcp"]
    },
    "memory": {
      "command": "node",
      "args": ["server.js"]
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        let gitlab = &config["mcpServers"]["gitlab"];
        assert_eq!(gitlab["command"], "mcp-rtk");
        assert_eq!(gitlab["args"][0], "--");
        assert_eq!(gitlab["args"][1], "npx");
        assert_eq!(gitlab["args"][2], "-y");
        assert_eq!(gitlab["args"][3], "@nicepkg/gitlab-mcp");

        let memory = &config["mcpServers"]["memory"];
        assert_eq!(memory["command"], "mcp-rtk");
        assert_eq!(memory["args"][0], "--");
        assert_eq!(memory["args"][1], "node");
    }

    #[test]
    fn install_skips_non_stdio() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "remote": {
      "type": "http",
      "url": "https://example.com/mcp"
    },
    "local": {
      "type": "stdio",
      "command": "my-server"
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["remote"]["type"], "http");
        assert_eq!(
            config["mcpServers"]["remote"]
                .get("command")
                .and_then(|c| c.as_str()),
            None
        );
        assert_eq!(config["mcpServers"]["local"]["command"], "mcp-rtk");
    }

    #[test]
    fn install_skips_already_wrapped() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "mcp-rtk",
      "args": ["--", "npx", "-y", "@nicepkg/gitlab-mcp"]
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        let args = config["mcpServers"]["gitlab"]["args"].as_array().unwrap();
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "--");
        assert_eq!(args[1], "npx");
    }

    #[test]
    fn install_creates_backup() {
        let dir = TempDir::new().unwrap();
        let original = r#"{
  "mcpServers": {
    "s": {
      "command": "srv"
    }
  }
}
"#;
        let path = write_config(&dir, original);

        run_install(&path, None).unwrap();

        let backup = path.with_extension("json.bak");
        assert!(backup.exists());
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    }

    #[test]
    fn install_preserves_4_space_indent() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            "{\n    \"mcpServers\": {\n        \"s\": {\n            \"command\": \"srv\"\n        }\n    }\n}\n",
        );

        run_install(&path, None).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("    \"mcpServers\""));
    }

    #[test]
    fn install_server_filter() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "npx",
      "args": ["gitlab-mcp"]
    },
    "memory": {
      "command": "node",
      "args": ["memory.js"]
    }
  }
}
"#,
        );

        run_install(&path, Some("gitlab")).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["gitlab"]["command"], "mcp-rtk");
        assert_eq!(config["mcpServers"]["memory"]["command"], "node");
    }

    #[test]
    fn install_invalid_json_fails() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, "not json at all {{{");

        let result = run_install(&path, None);
        assert!(result.is_err());
    }

    #[test]
    fn install_missing_file_fails() {
        let result = run_install(Path::new("/tmp/nonexistent_mcp_rtk_test.json"), None);
        assert!(result.is_err());
    }

    #[test]
    fn install_no_mcp_servers_fails() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, r#"{"settings": true}"#);

        let result = run_install(&path, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No 'mcpServers' found"));
    }

    #[test]
    fn uninstall_restores_original() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "mcp-rtk",
      "args": ["--", "npx", "-y", "@nicepkg/gitlab-mcp"]
    }
  }
}
"#,
        );

        run_uninstall(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["gitlab"]["command"], "npx");
        let args = config["mcpServers"]["gitlab"]["args"].as_array().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "@nicepkg/gitlab-mcp");
    }

    #[test]
    fn uninstall_handles_preset_flags() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "mcp-rtk",
      "args": ["--preset", "gitlab", "--", "npx", "-y", "@nicepkg/gitlab-mcp"]
    }
  }
}
"#,
        );

        run_uninstall(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["gitlab"]["command"], "npx");
        let args = config["mcpServers"]["gitlab"]["args"].as_array().unwrap();
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "@nicepkg/gitlab-mcp");
    }

    #[test]
    fn uninstall_removes_args_when_empty() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "s": {
      "command": "mcp-rtk",
      "args": ["--", "my-server"]
    }
  }
}
"#,
        );

        run_uninstall(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["s"]["command"], "my-server");
        assert!(config["mcpServers"]["s"].get("args").is_none());
    }

    #[test]
    fn roundtrip_install_uninstall() {
        let dir = TempDir::new().unwrap();
        let original = r#"{
  "mcpServers": {
    "gitlab": {
      "command": "npx",
      "args": ["-y", "@nicepkg/gitlab-mcp"],
      "env": {
        "GITLAB_TOKEN": "abc"
      }
    }
  }
}
"#;
        let path = write_config(&dir, original);

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["gitlab"]["command"], "mcp-rtk");

        run_uninstall(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["gitlab"]["command"], "npx");
        let args = config["mcpServers"]["gitlab"]["args"].as_array().unwrap();
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "@nicepkg/gitlab-mcp");
        assert_eq!(config["mcpServers"]["gitlab"]["env"]["GITLAB_TOKEN"], "abc");
    }

    #[test]
    fn install_claude_desktop_format() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["filesystem"]["command"], "mcp-rtk");
        assert_eq!(config["mcpServers"]["filesystem"]["args"][0], "--");
        assert_eq!(config["mcpServers"]["filesystem"]["args"][1], "npx");
    }

    #[test]
    fn uninstall_server_not_found() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "npx"
    }
  }
}
"#,
        );

        let result = run_uninstall(&path, Some("nonexistent"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    // ── claude.json format (projects.<path>.mcpServers) ──────────────

    #[test]
    fn install_claude_json_top_level_and_projects() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "global-server": {
      "command": "npx",
      "args": ["-y", "global-mcp"]
    }
  },
  "projects": {
    "/home/user/project-a": {
      "mcpServers": {
        "project-gitlab": {
          "command": "npx",
          "args": ["-y", "@nicepkg/gitlab-mcp"]
        }
      }
    },
    "/home/user/project-b": {
      "mcpServers": {
        "project-grafana": {
          "type": "stdio",
          "command": "npx",
          "args": ["-y", "mcp-grafana"]
        }
      }
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);

        // Top-level server wrapped
        assert_eq!(config["mcpServers"]["global-server"]["command"], "mcp-rtk");
        assert_eq!(config["mcpServers"]["global-server"]["args"][0], "--");
        assert_eq!(config["mcpServers"]["global-server"]["args"][1], "npx");

        // Project-a server wrapped
        let proj_a = &config["projects"]["/home/user/project-a"]["mcpServers"]["project-gitlab"];
        assert_eq!(proj_a["command"], "mcp-rtk");
        assert_eq!(proj_a["args"][0], "--");
        assert_eq!(proj_a["args"][1], "npx");

        // Project-b server wrapped
        let proj_b = &config["projects"]["/home/user/project-b"]["mcpServers"]["project-grafana"];
        assert_eq!(proj_b["command"], "mcp-rtk");
        assert_eq!(proj_b["args"][0], "--");
    }

    #[test]
    fn uninstall_claude_json_projects() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "projects": {
    "/home/user/myproject": {
      "mcpServers": {
        "gitlab": {
          "command": "mcp-rtk",
          "args": ["--", "npx", "-y", "@nicepkg/gitlab-mcp"]
        },
        "memory": {
          "command": "node",
          "args": ["memory.js"]
        }
      }
    }
  }
}
"#,
        );

        run_uninstall(&path, None).unwrap();

        let config = read_back(&path);
        let servers = &config["projects"]["/home/user/myproject"]["mcpServers"];

        // gitlab unwrapped
        assert_eq!(servers["gitlab"]["command"], "npx");
        let args = servers["gitlab"]["args"].as_array().unwrap();
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "@nicepkg/gitlab-mcp");

        // memory untouched
        assert_eq!(servers["memory"]["command"], "node");
    }

    #[test]
    fn install_claude_json_skips_http_in_projects() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "projects": {
    "/tmp/proj": {
      "mcpServers": {
        "remote": {
          "type": "http",
          "url": "https://example.com/mcp"
        },
        "local": {
          "command": "my-server"
        }
      }
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        let servers = &config["projects"]["/tmp/proj"]["mcpServers"];
        assert_eq!(servers["remote"]["type"], "http");
        assert!(servers["remote"].get("command").is_none());
        assert_eq!(servers["local"]["command"], "mcp-rtk");
    }

    #[test]
    fn install_claude_json_server_filter_across_scopes() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "gitlab": {
      "command": "npx",
      "args": ["global-gitlab"]
    }
  },
  "projects": {
    "/tmp/proj": {
      "mcpServers": {
        "gitlab": {
          "command": "npx",
          "args": ["project-gitlab"]
        },
        "other": {
          "command": "other-server"
        }
      }
    }
  }
}
"#,
        );

        // Filter by name "gitlab" — should wrap in BOTH scopes
        run_install(&path, Some("gitlab")).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["gitlab"]["command"], "mcp-rtk");

        let proj = &config["projects"]["/tmp/proj"]["mcpServers"];
        assert_eq!(proj["gitlab"]["command"], "mcp-rtk");
        assert_eq!(proj["other"]["command"], "other-server");
    }

    #[test]
    fn roundtrip_claude_json_format() {
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"{
  "mcpServers": {
    "global": {
      "command": "global-srv",
      "args": ["--flag"]
    }
  },
  "projects": {
    "/tmp/p": {
      "mcpServers": {
        "local": {
          "command": "local-srv",
          "env": {
            "KEY": "val"
          }
        }
      }
    }
  }
}
"#,
        );

        run_install(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["global"]["command"], "mcp-rtk");
        assert_eq!(
            config["projects"]["/tmp/p"]["mcpServers"]["local"]["command"],
            "mcp-rtk"
        );

        run_uninstall(&path, None).unwrap();

        let config = read_back(&path);
        assert_eq!(config["mcpServers"]["global"]["command"], "global-srv");
        assert_eq!(
            config["projects"]["/tmp/p"]["mcpServers"]["local"]["command"],
            "local-srv"
        );
        assert_eq!(
            config["projects"]["/tmp/p"]["mcpServers"]["local"]["env"]["KEY"],
            "val"
        );
    }
}

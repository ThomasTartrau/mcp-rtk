//! Configuration loading with preset auto-detection.
//!
//! mcp-rtk uses a layered configuration approach:
//!
//! 1. **Generic defaults** (`config/default.toml`) — sensible rules for any MCP.
//! 2. **Presets** (`config/presets/*.toml`) — community-contributed, tool-specific
//!    filter rules for known MCP servers. Auto-detected from the upstream command.
//! 3. **User config** (optional `--config`) — power-user overrides.
//!
//! All layers are merged: presets add tool rules on top of defaults, and user
//! config overrides everything.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Generic default filter rules (no tool-specific entries).
static DEFAULT_FILTERS: &str = include_str!("../config/default.toml");

/// Known presets, embedded at compile time.
static PRESETS: &[(&str, &[&str], &str)] = &[
    (
        "gitlab",
        &["gitlab-mcp", "gitlab"],
        include_str!("../config/presets/gitlab.toml"),
    ),
    (
        "grafana",
        &["mcp-grafana", "grafana"],
        include_str!("../config/presets/grafana.toml"),
    ),
    // To add a new preset:
    // ("github", &["github-mcp", "github"], include_str!("../config/presets/github.toml")),
];

/// Top-level configuration for mcp-rtk.
///
/// # Examples
///
/// ```no_run
/// # use mcp_rtk::config::Config;
/// # fn example() -> anyhow::Result<()> {
/// let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None)?;
/// let rules = config.get_tool_rules("list_merge_requests");
/// assert!(!rules.keep_fields.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    /// Upstream MCP server command and environment.
    pub upstream: UpstreamConfig,
    /// Filter rules (default + preset + user overrides).
    pub filters: FilterConfig,
    /// Token-savings tracking configuration.
    pub tracking: TrackingConfig,
    /// Name of the detected/selected preset, if any.
    pub preset: Option<String>,
}

/// How to spawn the upstream MCP server.
#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamConfig {
    /// The executable to run (e.g. `"node"`).
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables. Values starting with `$` are resolved from
    /// the current process environment.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Container for the default filter rules and per-tool overrides.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FilterConfig {
    /// Rules applied to every tool unless overridden.
    #[serde(default)]
    pub default: ToolFilterRules,
    /// Per-tool overrides, keyed by MCP tool name.
    #[serde(default, alias = "tools")]
    pub tools: HashMap<String, ToolFilterRules>,
}

/// Declarative filter rules for a single tool (or the default).
///
/// All fields are optional so that tool-specific sections only need to
/// specify what they override.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolFilterRules {
    /// Whitelist of JSON field names to keep (applied first).
    #[serde(default)]
    pub keep_fields: Vec<String>,
    /// Blacklist of JSON field names to strip recursively.
    #[serde(default)]
    pub strip_fields: Vec<String>,
    /// Replace user objects (`{id, name, username, …}`) with just `"username"`.
    #[serde(default)]
    pub condense_users: Option<bool>,
    /// Maximum character length for any string value.
    #[serde(default)]
    pub truncate_strings_at: Option<usize>,
    /// Maximum number of items in any JSON array.
    #[serde(default)]
    pub max_array_items: Option<usize>,
    /// Remove all `null` and empty-string fields.
    #[serde(default)]
    pub strip_nulls: Option<bool>,
    /// Unwrap single-key wrapper objects (`{"data": [...]}` → `[...]`).
    #[serde(default)]
    pub flatten: Option<bool>,
    /// Regex-based string replacements applied last.
    #[serde(default)]
    pub custom_transforms: Vec<CustomTransform>,
}

/// A single regex-based string replacement.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomTransform {
    /// The regex pattern to match.
    pub pattern: String,
    /// The replacement string (supports `$1`-style capture groups).
    pub replacement: String,
}

/// SQLite tracking configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TrackingConfig {
    /// Whether to record per-call metrics.
    #[serde(default = "default_tracking_enabled")]
    pub enabled: bool,
    /// Path to the SQLite database. Supports `~/` expansion.
    #[serde(default = "default_db_path")]
    pub db_path: String,
}

impl Default for TrackingConfig {
    fn default() -> Self {
        Self {
            enabled: default_tracking_enabled(),
            db_path: default_db_path(),
        }
    }
}

fn default_tracking_enabled() -> bool {
    true
}

fn default_db_path() -> String {
    "~/.local/share/mcp-rtk/metrics.db".to_string()
}

/// Preset filter rules (no upstream section — just `[tools.*]`).
///
/// External presets can include an optional `[meta]` section with detection
/// keywords for auto-discovery:
///
/// ```toml
/// [meta]
/// keywords = ["github-mcp", "github"]
///
/// [tools.list_repos]
/// keep_fields = ["id", "name", "full_name"]
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct PresetConfig {
    /// Optional metadata for auto-detection (used by external presets).
    #[serde(default)]
    pub meta: Option<PresetMeta>,
    #[serde(default)]
    pub tools: HashMap<String, ToolFilterRules>,
}

/// Metadata for an external preset, enabling auto-detection from the upstream
/// command.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PresetMeta {
    /// Keywords that trigger this preset when found in the upstream command.
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// An external preset loaded from the filesystem at runtime.
#[derive(Debug, Clone)]
pub struct ExternalPreset {
    /// Preset name (derived from the filename without `.toml`).
    pub name: String,
    /// Keywords for auto-detection (from `[meta]`).
    pub keywords: Vec<String>,
    /// The parsed preset configuration.
    pub config: PresetConfig,
    /// Path to the source TOML file.
    pub path: PathBuf,
}

/// Return the directory for external (user/community) presets.
///
/// Defaults to `~/.local/share/mcp-rtk/presets/`. The directory is created
/// if it does not exist.
pub fn external_presets_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let dir = PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("mcp-rtk")
        .join("presets");
    std::fs::create_dir_all(&dir)
        .context(format!("Failed to create presets dir: {}", dir.display()))?;
    Ok(dir)
}

/// User-supplied configuration file. All sections are optional.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UserConfig {
    /// Optional upstream override (env vars from config are merged).
    #[serde(default)]
    pub upstream: Option<UpstreamConfig>,
    #[serde(default)]
    pub(crate) filters: Option<FilterConfig>,
    #[serde(default)]
    tracking: Option<TrackingConfig>,
    /// Explicitly select a preset (overrides auto-detection).
    #[serde(default)]
    preset: Option<String>,
}

/// Simple glob matching: `*` matches any sequence of characters, `?` matches
/// exactly one character. No other special syntax is supported.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }

    pi == p.len()
}

impl Config {
    /// Build configuration from upstream command args with optional user config.
    ///
    /// This is the primary entry point. The upstream command is taken from
    /// `upstream_args` (e.g. `["npx", "@nicepkg/gitlab-mcp"]`). A preset is
    /// auto-detected from the command (including external presets from
    /// `~/.local/share/mcp-rtk/presets/`), and an optional user config file
    /// provides overrides.
    ///
    /// # Errors
    ///
    /// Returns an error if the user config file cannot be read or parsed.
    pub fn from_upstream(upstream_args: &[&str], config_path: Option<&Path>) -> Result<Self> {
        let defaults = Self::load_defaults()?;
        let externals = Self::load_external_presets();

        // Build upstream from args
        let mut upstream = if let Some((cmd, args)) = upstream_args.split_first() {
            UpstreamConfig {
                command: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                env: HashMap::new(),
            }
        } else {
            anyhow::bail!("No upstream command provided. Usage: mcp-rtk -- <command> [args...]");
        };

        // Load user config if provided
        let user_config = if let Some(path) = config_path {
            let content = std::fs::read_to_string(path).context("Failed to read config file")?;
            Some(toml::from_str::<UserConfig>(&content).context("Failed to parse config file")?)
        } else {
            None
        };

        // Determine preset: user explicit > auto-detect (embedded + external)
        let preset_name = user_config
            .as_ref()
            .and_then(|u| u.preset.clone())
            .or_else(|| Self::detect_preset_all(upstream_args, &externals));

        // Layer: defaults → preset → user config
        let mut filters = defaults;
        if let Some(ref name) = preset_name {
            if let Some(preset) = Self::load_preset_all(name, &externals) {
                for (k, v) in preset.tools {
                    filters.tools.insert(k, v);
                }
            }
        }

        let mut tracking = TrackingConfig::default();

        // Apply user overrides
        if let Some(user) = user_config {
            // Merge env vars from user config upstream (if any)
            if let Some(user_upstream) = user.upstream {
                for (k, v) in user_upstream.env {
                    upstream.env.insert(k, v);
                }
            }
            if let Some(user_filters) = user.filters {
                // User default rules merge on top
                filters.default = merge_tool_rules(&filters.default, &user_filters.default);
                // User tool rules override
                for (k, v) in user_filters.tools {
                    filters.tools.insert(k, v);
                }
            }
            if let Some(t) = user.tracking {
                tracking = t;
            }
        }

        // Resolve upstream env: inherit from parent process env
        let upstream = Self::resolve_env(upstream);

        Ok(Config {
            upstream,
            filters,
            tracking,
            preset: preset_name,
        })
    }

    /// Build a complete config with optional preset override.
    ///
    /// Convenience wrapper around [`from_upstream`](Self::from_upstream) that
    /// also applies a `--preset` override. Used by both initial startup and
    /// hot reload to avoid duplicating the override logic.
    pub fn build(
        upstream_args: &[&str],
        config_path: Option<&Path>,
        preset_override: Option<&str>,
    ) -> Result<Self> {
        let mut config = Self::from_upstream(upstream_args, config_path)?;

        if let Some(preset_name) = preset_override {
            if let Some(preset_rules) = Self::load_preset_by_name(preset_name) {
                for (k, v) in preset_rules {
                    config.filters.tools.insert(k, v);
                }
                config.preset = Some(preset_name.to_string());
            } else {
                anyhow::bail!(
                    "Unknown preset: {preset_name}\nAvailable: {}",
                    Self::available_presets().join(", ")
                );
            }
        }

        Ok(config)
    }

    /// Load configuration for the `gain` subcommand (no upstream needed).
    ///
    /// # Errors
    ///
    /// Returns an error if the user config file cannot be read or parsed.
    pub fn load_for_gain(config_path: Option<&Path>) -> Result<Self> {
        let defaults = Self::load_defaults()?;
        let mut tracking = TrackingConfig::default();

        if let Some(path) = config_path {
            let content = std::fs::read_to_string(path).context("Failed to read config file")?;
            let user: UserConfig =
                toml::from_str(&content).context("Failed to parse config file")?;
            if let Some(t) = user.tracking {
                tracking = t;
            }
        }

        Ok(Config {
            upstream: UpstreamConfig {
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
            },
            filters: defaults,
            tracking,
            preset: None,
        })
    }

    /// Load the generic default filter rules.
    fn load_defaults() -> Result<FilterConfig> {
        toml::from_str(DEFAULT_FILTERS).context("Failed to parse built-in defaults")
    }

    /// Auto-detect a preset name from the upstream command args (embedded only).
    fn detect_preset(args: &[&str]) -> Option<String> {
        let joined = args.join(" ").to_lowercase();
        for (name, keywords, _) in PRESETS {
            for keyword in *keywords {
                if joined.contains(keyword) {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Auto-detect a preset name from upstream args, checking both embedded
    /// and external presets. Embedded presets take priority.
    fn detect_preset_all(args: &[&str], externals: &[ExternalPreset]) -> Option<String> {
        if let Some(name) = Self::detect_preset(args) {
            return Some(name);
        }
        let joined = args.join(" ").to_lowercase();
        for ext in externals {
            for keyword in &ext.keywords {
                if joined.contains(&keyword.to_lowercase()) {
                    return Some(ext.name.clone());
                }
            }
        }
        None
    }

    /// Load a preset's tool rules by name (embedded + external).
    ///
    /// Returns the tool-specific filter rules from the preset, or `None` if
    /// the preset name is unknown.
    pub fn load_preset_by_name(name: &str) -> Option<HashMap<String, ToolFilterRules>> {
        let externals = Self::load_external_presets();
        Self::load_preset_all(name, &externals).map(|p| p.tools)
    }

    /// Load a preset by name from the embedded presets.
    fn load_preset(name: &str) -> Option<PresetConfig> {
        for (preset_name, _, toml_content) in PRESETS {
            if *preset_name == name {
                return toml::from_str(toml_content).ok();
            }
        }
        None
    }

    /// Load a preset by name, checking both embedded and external presets.
    /// Embedded presets take priority.
    fn load_preset_all(name: &str, externals: &[ExternalPreset]) -> Option<PresetConfig> {
        if let Some(preset) = Self::load_preset(name) {
            return Some(preset);
        }
        externals
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.config.clone())
    }

    /// Scan `~/.local/share/mcp-rtk/presets/` for external preset TOML files.
    ///
    /// Each `.toml` file is parsed as a [`PresetConfig`]. The preset name is
    /// derived from the filename (without extension). An optional `[meta]`
    /// section provides detection keywords for auto-discovery.
    ///
    /// Invalid files are silently skipped.
    pub fn load_external_presets() -> Vec<ExternalPreset> {
        let dir = match external_presets_dir() {
            Ok(d) => d,
            Err(_) => return vec![],
        };

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return vec![],
        };

        let mut presets = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() != Some(std::ffi::OsStr::new("toml")) {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let config = match toml::from_str::<PresetConfig>(&content) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Skipping invalid preset {}: {e}", path.display());
                    continue;
                }
            };
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let keywords = config
                .meta
                .as_ref()
                .map(|m| m.keywords.clone())
                .unwrap_or_default();
            presets.push(ExternalPreset {
                name,
                keywords,
                config,
                path,
            });
        }

        if !presets.is_empty() {
            let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
            tracing::debug!(
                "Loaded {} external preset(s): {}",
                presets.len(),
                names.join(", ")
            );
        }

        presets
    }

    /// Resolve env vars: values starting with `$` are read from the process env.
    ///
    /// Env vars from the parent process are also inherited automatically by the
    /// child process, so most env vars don't need to be in the config at all.
    ///
    /// # Security
    ///
    /// Only config values explicitly prefixed with `$` are resolved. The config
    /// file itself must be trusted — anyone who can write to it can control which
    /// env vars are forwarded and which command is spawned.
    fn resolve_env(mut upstream: UpstreamConfig) -> UpstreamConfig {
        let resolved: HashMap<String, String> = upstream
            .env
            .iter()
            .map(|(k, v)| {
                let resolved = if let Some(var_name) = v.strip_prefix('$') {
                    std::env::var(var_name).unwrap_or_default()
                } else {
                    v.clone()
                };
                (k.clone(), resolved)
            })
            .collect();
        upstream.env = resolved;
        upstream
    }

    /// Return the merged filter rules for a given tool name.
    ///
    /// Tool-specific rules override the defaults. Lists (`strip_fields`,
    /// `custom_transforms`) are concatenated; scalars use the tool value
    /// if present, otherwise the default.
    ///
    /// Lookup order:
    /// 1. Exact match by tool name (fast path).
    /// 2. Glob pattern match — keys containing `*` or `?` are tested against
    ///    the tool name using [`glob_match`].
    pub fn get_tool_rules(&self, tool_name: &str) -> MergedRules {
        let defaults = &self.filters.default;

        // Exact match first
        if let Some(specific) = self.filters.tools.get(tool_name) {
            return MergedRules::merge(defaults, Some(specific));
        }

        // Glob pattern match
        for (pattern, rules) in &self.filters.tools {
            if (pattern.contains('*') || pattern.contains('?')) && glob_match(pattern, tool_name) {
                return MergedRules::merge(defaults, Some(rules));
            }
        }

        MergedRules::merge(defaults, None)
    }

    /// List all available preset names (embedded + external).
    pub fn available_presets() -> Vec<String> {
        let mut names: Vec<String> = PRESETS
            .iter()
            .map(|(name, _, _)| name.to_string())
            .collect();
        for ext in Self::load_external_presets() {
            if !names.iter().any(|n| n == &ext.name) {
                names.push(ext.name);
            }
        }
        names
    }
}

/// Print a table of all available presets (embedded + external).
pub fn list_presets() {
    use crate::display::*;

    println!();
    println!("  {BOLD}{GREEN}MCP-RTK{RESET}{DIM} — Available Presets{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(56));

    // Embedded presets
    println!();
    println!("  {DIM}Built-in:{RESET}");
    for (name, keywords, toml_content) in PRESETS {
        let tool_count = toml_content.matches("[tools.").count();
        println!(
            "  {BOLD}{WHITE}{:<12}{RESET}  {DIM}detected from:{RESET} {YELLOW}{}{RESET}  {DIM}({} tools){RESET}",
            name,
            keywords.join(", "),
            tool_count,
        );
    }

    // External presets
    let externals = Config::load_external_presets();
    if !externals.is_empty() {
        println!();
        println!("  {DIM}External (~/.local/share/mcp-rtk/presets/):{RESET}");
        for ext in &externals {
            let tool_count = ext.config.tools.len();
            let kw = if ext.keywords.is_empty() {
                "manual only".to_string()
            } else {
                ext.keywords.join(", ")
            };
            println!(
                "  {BOLD}{WHITE}{:<12}{RESET}  {DIM}detected from:{RESET} {YELLOW}{}{RESET}  {DIM}({} tools){RESET}",
                ext.name, kw, tool_count,
            );
        }
    }

    println!();
    println!("  {DIM}Use `mcp-rtk presets show <name>` to see the full TOML.{RESET}");
    if externals.is_empty() {
        println!("  {DIM}Drop .toml presets in ~/.local/share/mcp-rtk/presets/ for auto-discovery.{RESET}");
    }
    println!();
}

/// Print the full TOML content of a named preset (embedded or external).
pub fn show_preset(name: &str) -> Result<()> {
    use crate::display::*;

    // Check embedded presets
    for (preset_name, keywords, toml_content) in PRESETS {
        if *preset_name == name {
            println!();
            println!("  {BOLD}{GREEN}{}{RESET}{DIM} preset{RESET}", name);
            println!("  {DIM}Auto-detected from: {}{RESET}", keywords.join(", "));
            println!();
            print_toml_highlighted(toml_content);
            println!();
            return Ok(());
        }
    }

    // Check external presets
    for ext in Config::load_external_presets() {
        if ext.name == name {
            let content = std::fs::read_to_string(&ext.path)
                .context(format!("Failed to read {}", ext.path.display()))?;
            let kw = if ext.keywords.is_empty() {
                "none (use --preset to select)".to_string()
            } else {
                ext.keywords.join(", ")
            };
            println!();
            println!(
                "  {BOLD}{GREEN}{}{RESET}{DIM} preset (external){RESET}",
                name
            );
            println!("  {DIM}Auto-detected from: {kw}{RESET}");
            println!("  {DIM}Path: {}{RESET}", ext.path.display());
            println!();
            print_toml_highlighted(&content);
            println!();
            return Ok(());
        }
    }

    anyhow::bail!(
        "Unknown preset: {name}\nAvailable: {}",
        Config::available_presets().join(", ")
    );
}

fn print_toml_highlighted(content: &str) {
    use crate::display::*;

    for line in content.lines() {
        if line.starts_with('#') {
            println!("  {DIM}{line}{RESET}");
        } else if line.starts_with("[tools.") || line.starts_with("[meta]") {
            println!("  {BOLD}{CYAN}{line}{RESET}");
        } else if line.is_empty() {
            println!();
        } else {
            println!("  {line}");
        }
    }
}

/// Merge two sets of tool filter rules (user on top of base).
fn merge_tool_rules(base: &ToolFilterRules, user: &ToolFilterRules) -> ToolFilterRules {
    ToolFilterRules {
        keep_fields: if user.keep_fields.is_empty() {
            base.keep_fields.clone()
        } else {
            user.keep_fields.clone()
        },
        strip_fields: {
            let mut fields = base.strip_fields.clone();
            fields.extend(user.strip_fields.clone());
            fields
        },
        condense_users: user.condense_users.or(base.condense_users),
        truncate_strings_at: user.truncate_strings_at.or(base.truncate_strings_at),
        max_array_items: user.max_array_items.or(base.max_array_items),
        strip_nulls: user.strip_nulls.or(base.strip_nulls),
        flatten: user.flatten.or(base.flatten),
        custom_transforms: {
            let mut t = base.custom_transforms.clone();
            t.extend(user.custom_transforms.clone());
            t
        },
    }
}

/// Fully resolved filter rules for a single tool call.
///
/// Produced by [`Config::get_tool_rules`] — the result of merging the default
/// rules with any tool-specific overrides.
#[derive(Debug, Clone)]
pub struct MergedRules {
    /// Whitelist of JSON field names to keep.
    pub keep_fields: Vec<String>,
    /// Blacklist of JSON field names to strip recursively.
    pub strip_fields: Vec<String>,
    /// Whether to condense user objects to bare usernames.
    pub condense_users: bool,
    /// Maximum character length for any string value.
    pub truncate_strings_at: usize,
    /// Maximum number of items in any JSON array.
    pub max_array_items: usize,
    /// Whether to remove null and empty-string fields.
    pub strip_nulls: bool,
    /// Whether to unwrap single-key wrapper objects.
    pub flatten: bool,
    /// Compiled regex-based string replacements.
    pub custom_transforms: Vec<CustomTransform>,
}

impl MergedRules {
    fn merge(defaults: &ToolFilterRules, specific: Option<&ToolFilterRules>) -> Self {
        let s = specific.cloned().unwrap_or_default();
        Self {
            keep_fields: if s.keep_fields.is_empty() {
                defaults.keep_fields.clone()
            } else {
                s.keep_fields
            },
            strip_fields: {
                let mut fields = defaults.strip_fields.clone();
                fields.extend(s.strip_fields);
                fields
            },
            condense_users: s
                .condense_users
                .or(defaults.condense_users)
                .unwrap_or(false),
            truncate_strings_at: s
                .truncate_strings_at
                .or(defaults.truncate_strings_at)
                .unwrap_or(usize::MAX),
            max_array_items: s
                .max_array_items
                .or(defaults.max_array_items)
                .unwrap_or(usize::MAX),
            strip_nulls: s.strip_nulls.or(defaults.strip_nulls).unwrap_or(false),
            flatten: s.flatten.or(defaults.flatten).unwrap_or(false),
            custom_transforms: {
                let mut t = defaults.custom_transforms.clone();
                t.extend(s.custom_transforms);
                t
            },
        }
    }
}

/// Validate a preset or user config TOML file and print a diagnostic report.
///
/// Parses the file as either a preset (`[tools.*]` format) or a full user
/// config (`[filters.*]` format). Reports the tools defined, active rules
/// per tool, and any warnings (conflicting options, invalid regex, etc.).
///
/// # Errors
///
/// Returns an error if the file cannot be read or is not valid TOML for
/// either format.
pub fn validate_preset_file(path: &Path) -> Result<()> {
    use crate::display::*;

    let content = std::fs::read_to_string(path)
        .context(format!("Failed to read file: {}", path.display()))?;

    // Try parsing as a preset (tools.* format)
    let preset_result = toml::from_str::<PresetConfig>(&content);
    // Try parsing as a full user config (filters.* format)
    let user_result = toml::from_str::<UserConfig>(&content);

    let (tools, is_preset) = match (preset_result, user_result) {
        (Ok(preset), _) => (preset.tools, true),
        (_, Ok(user)) => {
            let filters = user.filters.unwrap_or_default();
            (filters.tools, false)
        }
        (Err(e1), Err(_)) => {
            anyhow::bail!("Failed to parse TOML:\n{e1}");
        }
    };

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    println!();
    println!(
        "  {BOLD}{GREEN}✓{RESET} {BOLD}{file_name}{RESET} is valid {}",
        if is_preset { "preset" } else { "config" }
    );
    println!();

    // Stats
    println!("  {DIM}Tools defined:{RESET}  {BOLD}{}{RESET}", tools.len());

    // List tools with their active rules
    if !tools.is_empty() {
        println!();
        println!("  {DIM}Tool rules:{RESET}");
        for (name, rules) in &tools {
            let mut active = Vec::new();
            if !rules.keep_fields.is_empty() {
                active.push(format!("keep:{}", rules.keep_fields.len()));
            }
            if !rules.strip_fields.is_empty() {
                active.push(format!("strip:{}", rules.strip_fields.len()));
            }
            if rules.condense_users == Some(true) {
                active.push("condense_users".into());
            }
            if let Some(n) = rules.truncate_strings_at {
                active.push(format!("truncate:{n}"));
            }
            if let Some(n) = rules.max_array_items {
                active.push(format!("max_items:{n}"));
            }
            if rules.strip_nulls == Some(true) {
                active.push("strip_nulls".into());
            }
            if rules.flatten == Some(true) {
                active.push("flatten".into());
            }
            if !rules.custom_transforms.is_empty() {
                active.push(format!("transforms:{}", rules.custom_transforms.len()));
            }

            println!(
                "    {BOLD}{WHITE}{:<32}{RESET} {DIM}{}{RESET}",
                name,
                active.join(", ")
            );
        }
    }

    // Warnings
    let mut warnings = Vec::new();
    for (name, rules) in &tools {
        if !rules.keep_fields.is_empty() && !rules.strip_fields.is_empty() {
            warnings.push(format!(
                "{name}: has both keep_fields and strip_fields (keep_fields takes priority, strip_fields may be redundant)"
            ));
        }
        if rules.truncate_strings_at == Some(0) {
            warnings.push(format!(
                "{name}: truncate_strings_at is 0 (all strings will be empty)"
            ));
        }
        if rules.max_array_items == Some(0) {
            warnings.push(format!(
                "{name}: max_array_items is 0 (all arrays will be empty)"
            ));
        }
    }

    // Validate custom_transforms regex patterns
    for (name, rules) in &tools {
        for (i, transform) in rules.custom_transforms.iter().enumerate() {
            if regex::Regex::new(&transform.pattern).is_err() {
                warnings.push(format!(
                    "{name}: custom_transform[{i}] has invalid regex: {}",
                    transform.pattern
                ));
            }
        }
    }

    if !warnings.is_empty() {
        println!();
        println!("  {YELLOW}Warnings:{RESET}");
        for w in &warnings {
            println!("    {YELLOW}⚠{RESET}  {w}");
        }
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_defaults() {
        let filters = Config::load_defaults().unwrap();
        assert!(filters.default.strip_nulls.unwrap_or(false));
        assert!(filters.default.condense_users.unwrap_or(false));
        assert!(filters.default.flatten.unwrap_or(false));
    }

    #[test]
    fn detect_gitlab_preset() {
        assert_eq!(
            Config::detect_preset(&["npx", "@nicepkg/gitlab-mcp"]),
            Some("gitlab".to_string())
        );
        assert_eq!(
            Config::detect_preset(&["node", "/path/to/gitlab-mcp/build/index.js"]),
            Some("gitlab".to_string())
        );
    }

    #[test]
    fn detect_no_preset() {
        assert_eq!(
            Config::detect_preset(&["node", "/path/to/custom-server.js"]),
            None
        );
    }

    #[test]
    fn from_upstream_with_gitlab_preset() {
        let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap();
        assert_eq!(config.preset, Some("gitlab".to_string()));
        assert_eq!(config.upstream.command, "npx");
        assert_eq!(config.upstream.args, vec!["@nicepkg/gitlab-mcp"]);
        // GitLab preset should have tool-specific rules
        let rules = config.get_tool_rules("list_merge_requests");
        assert!(!rules.keep_fields.is_empty());
        assert!(rules.condense_users);
    }

    #[test]
    fn from_upstream_without_preset() {
        let config = Config::from_upstream(&["node", "my-custom-server.js"], None).unwrap();
        assert_eq!(config.preset, None);
        // Should still have generic defaults
        let rules = config.get_tool_rules("any_tool");
        assert!(rules.strip_nulls);
        assert!(rules.condense_users);
        assert!(rules.keep_fields.is_empty());
    }

    #[test]
    fn from_upstream_no_args_fails() {
        let result = Config::from_upstream(&[], None);
        assert!(result.is_err());
    }

    #[test]
    fn available_presets_includes_gitlab() {
        let presets = Config::available_presets();
        assert!(presets.iter().any(|p| p == "gitlab"));
    }

    #[test]
    fn get_tool_rules_merges_preset_and_defaults() {
        let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap();
        let rules = config.get_tool_rules("list_merge_requests");
        // From preset
        assert!(!rules.keep_fields.is_empty());
        // From defaults
        assert!(rules.strip_nulls);
        assert!(rules.strip_fields.contains(&"avatar_url".to_string()));
    }

    #[test]
    fn glob_match_star() {
        assert!(glob_match("list_*", "list_issues"));
        assert!(glob_match("list_*", "list_merge_requests"));
        assert!(!glob_match("list_*", "get_issue"));
        assert!(glob_match("*_requests", "list_merge_requests"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn glob_match_question() {
        assert!(glob_match("get_issue?", "get_issues"));
        assert!(!glob_match("get_issue?", "get_issue"));
        assert!(glob_match("get_?ssue", "get_issue"));
    }

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("list_issues", "list_issues"));
        assert!(!glob_match("list_issues", "list_merge_requests"));
    }

    #[test]
    fn get_tool_rules_glob_pattern() {
        let mut config = Config::from_upstream(&["echo", "test-server"], None).unwrap();
        config.filters.tools.insert(
            "list_*".to_string(),
            ToolFilterRules {
                keep_fields: vec!["id".to_string(), "name".to_string()],
                max_array_items: Some(5),
                ..Default::default()
            },
        );

        let rules = config.get_tool_rules("list_something");
        assert_eq!(rules.keep_fields, vec!["id", "name"]);
        assert_eq!(rules.max_array_items, 5);
    }

    #[test]
    fn get_tool_rules_exact_match_takes_priority_over_glob() {
        let mut config = Config::from_upstream(&["echo", "test-server"], None).unwrap();
        config.filters.tools.insert(
            "list_*".to_string(),
            ToolFilterRules {
                keep_fields: vec!["id".to_string(), "name".to_string()],
                ..Default::default()
            },
        );
        config.filters.tools.insert(
            "list_special".to_string(),
            ToolFilterRules {
                keep_fields: vec!["special_field".to_string()],
                ..Default::default()
            },
        );

        let rules = config.get_tool_rules("list_special");
        assert_eq!(rules.keep_fields, vec!["special_field"]);
    }

    #[test]
    fn load_external_presets_from_dir() {
        let temp = std::env::temp_dir().join("mcp-rtk-test-ext-presets");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        // Write an external preset with meta
        std::fs::write(
            temp.join("github.toml"),
            r#"
[meta]
keywords = ["github-mcp", "github"]

[tools.list_repos]
keep_fields = ["id", "name", "full_name"]
max_array_items = 20
"#,
        )
        .unwrap();

        // Write one without meta
        std::fs::write(
            temp.join("jira.toml"),
            r#"
[tools.list_issues]
keep_fields = ["key", "summary"]
"#,
        )
        .unwrap();

        // Write an invalid file (should be skipped)
        std::fs::write(temp.join("bad.toml"), "not valid {{{{").unwrap();

        // Write a non-toml file (should be skipped)
        std::fs::write(temp.join("readme.txt"), "ignore me").unwrap();

        // Manually scan the temp dir
        let entries = std::fs::read_dir(&temp).unwrap();
        let mut presets = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() != Some(std::ffi::OsStr::new("toml")) {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let config = match toml::from_str::<super::PresetConfig>(&content) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let keywords = config
                .meta
                .as_ref()
                .map(|m| m.keywords.clone())
                .unwrap_or_default();
            presets.push(super::ExternalPreset {
                name,
                keywords,
                config,
                path,
            });
        }

        // Should find github and jira, not bad.toml or readme.txt
        assert_eq!(presets.len(), 2);
        let github = presets.iter().find(|p| p.name == "github").unwrap();
        assert_eq!(github.keywords, vec!["github-mcp", "github"]);
        assert!(github.config.tools.contains_key("list_repos"));

        let jira = presets.iter().find(|p| p.name == "jira").unwrap();
        assert!(jira.keywords.is_empty());
        assert!(jira.config.tools.contains_key("list_issues"));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn detect_preset_all_finds_external() {
        let externals = vec![super::ExternalPreset {
            name: "github".to_string(),
            keywords: vec!["github-mcp".to_string(), "github".to_string()],
            config: super::PresetConfig {
                meta: None,
                tools: HashMap::new(),
            },
            path: std::path::PathBuf::from("/tmp/github.toml"),
        }];

        // Embedded takes priority
        assert_eq!(
            Config::detect_preset_all(&["npx", "gitlab-mcp"], &externals),
            Some("gitlab".to_string())
        );
        // External is found
        assert_eq!(
            Config::detect_preset_all(&["npx", "github-mcp"], &externals),
            Some("github".to_string())
        );
        // No match
        assert_eq!(
            Config::detect_preset_all(&["node", "custom-server"], &externals),
            None
        );
    }

    #[test]
    fn preset_config_parses_with_meta() {
        let toml_str = r#"
[meta]
keywords = ["test-mcp", "test"]

[tools.list_items]
keep_fields = ["id", "name"]
"#;
        let config: super::PresetConfig = toml::from_str(toml_str).unwrap();
        let meta = config.meta.unwrap();
        assert_eq!(meta.keywords, vec!["test-mcp", "test"]);
        assert!(config.tools.contains_key("list_items"));
    }

    #[test]
    fn preset_config_parses_without_meta() {
        let toml_str = r#"
[tools.list_items]
keep_fields = ["id", "name"]
"#;
        let config: super::PresetConfig = toml::from_str(toml_str).unwrap();
        assert!(config.meta.is_none());
        assert!(config.tools.contains_key("list_items"));
    }
}

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
use std::path::Path;

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
#[derive(Debug, Clone, Deserialize)]
struct PresetConfig {
    #[serde(default)]
    tools: HashMap<String, ToolFilterRules>,
}

/// User-supplied configuration file. All sections are optional.
#[derive(Debug, Clone, Deserialize)]
struct UserConfig {
    /// Optional upstream override (env vars from config are merged).
    #[serde(default)]
    pub upstream: Option<UpstreamConfig>,
    #[serde(default)]
    filters: Option<FilterConfig>,
    #[serde(default)]
    tracking: Option<TrackingConfig>,
    /// Explicitly select a preset (overrides auto-detection).
    #[serde(default)]
    preset: Option<String>,
}

impl Config {
    /// Build configuration from upstream command args with optional user config.
    ///
    /// This is the primary entry point. The upstream command is taken from
    /// `upstream_args` (e.g. `["npx", "@nicepkg/gitlab-mcp"]`). A preset is
    /// auto-detected from the command, and an optional user config file
    /// provides overrides.
    ///
    /// # Errors
    ///
    /// Returns an error if the user config file cannot be read or parsed.
    pub fn from_upstream(upstream_args: &[&str], config_path: Option<&Path>) -> Result<Self> {
        let defaults = Self::load_defaults()?;

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

        // Determine preset: user explicit > auto-detect from command
        let preset_name = user_config
            .as_ref()
            .and_then(|u| u.preset.clone())
            .or_else(|| Self::detect_preset(upstream_args));

        // Layer: defaults → preset → user config
        let mut filters = defaults;
        if let Some(ref name) = preset_name {
            if let Some(preset) = Self::load_preset(name) {
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

    /// Auto-detect a preset name from the upstream command args.
    ///
    /// Checks if any arg contains a known keyword (e.g. "gitlab-mcp" or "gitlab").
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

    /// Load a preset's tool rules by name.
    ///
    /// Returns the tool-specific filter rules from the preset, or `None` if
    /// the preset name is unknown.
    pub fn load_preset_by_name(name: &str) -> Option<HashMap<String, ToolFilterRules>> {
        Self::load_preset(name).map(|p| p.tools)
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
    pub fn get_tool_rules(&self, tool_name: &str) -> MergedRules {
        let defaults = &self.filters.default;
        let tool_specific = self.filters.tools.get(tool_name);
        MergedRules::merge(defaults, tool_specific)
    }

    /// List all available preset names.
    pub fn available_presets() -> Vec<&'static str> {
        PRESETS.iter().map(|(name, _, _)| *name).collect()
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
        assert!(presets.contains(&"gitlab"));
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
}

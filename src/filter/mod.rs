//! The 8-stage filter pipeline and generic JSON compression functions.
//!
//! [`FilterEngine`] is the central entry point: given a tool name and its raw
//! JSON output, it resolves the merged filter rules from the configuration and
//! applies the following stages in order:
//!
//! 1. **keep_fields** — whitelist of JSON field names to retain.
//! 2. **strip_fields** — blacklist of JSON field names to remove recursively.
//! 3. **condense_users** — replace user objects with bare usernames.
//! 4. **strip_nulls** — remove `null` and empty-string fields.
//! 5. **flatten** — unwrap single-key wrapper objects.
//! 6. **truncate_strings** — cap string values at a maximum length.
//! 7. **collapse_arrays** — limit array sizes with a summary entry.
//! 8. **custom_transforms** — regex-based string replacements.
//!
//! The low-level JSON manipulation functions live in the [`json`] submodule.

pub mod json;

use crate::config::{Config, CustomTransform, MergedRules};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Engine that applies the 8-stage JSON compression pipeline.
///
/// The engine holds a reference to the shared [`Config`] and resolves per-tool
/// filter rules on each call via [`Config::get_tool_rules`].
///
/// # Examples
///
/// ```no_run
/// # use std::sync::Arc;
/// # use mcp_rtk::config::Config;
/// # use mcp_rtk::filter::FilterEngine;
/// let config = Arc::new(Config::from_upstream(&["npx", "some-mcp"], None).unwrap());
/// let engine = FilterEngine::new(config);
/// let filtered = engine.filter("list_merge_requests", r#"[{"iid":1,"title":"Fix"}]"#);
/// ```
pub struct FilterEngine {
    config: Arc<Config>,
    /// Pre-compiled regex transforms, keyed by tool name.
    /// `""` key holds the default transforms.
    compiled_transforms: HashMap<String, Vec<(Regex, String)>>,
}

impl FilterEngine {
    /// Create a new filter engine with the given configuration.
    ///
    /// Precompiles all regex transforms from defaults and per-tool rules.
    pub fn new(config: Arc<Config>) -> Self {
        let mut compiled_transforms = HashMap::new();

        // Compile merged transforms for each known tool
        for tool_name in config.filters.tools.keys() {
            let rules = config.get_tool_rules(tool_name);
            if !rules.custom_transforms.is_empty() {
                compiled_transforms.insert(
                    tool_name.clone(),
                    compile_transforms(&rules.custom_transforms),
                );
            }
        }

        // Compile default-only transforms for unknown tools (key = "")
        if !config.filters.default.custom_transforms.is_empty() {
            compiled_transforms.insert(
                String::new(),
                compile_transforms(&config.filters.default.custom_transforms),
            );
        }

        Self {
            config,
            compiled_transforms,
        }
    }

    /// Access the underlying configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Maximum raw response size (10 MB) to prevent OOM from malicious upstreams.
    const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

    /// Apply the full filter pipeline to a tool's raw output string.
    ///
    /// If `raw` is valid JSON, it is parsed and run through all 8 pipeline
    /// stages. If parsing fails, only plain-text truncation is applied.
    /// Responses exceeding 10 MB are truncated before parsing.
    pub fn filter(&self, tool_name: &str, raw: &str) -> String {
        let rules = self.config.get_tool_rules(tool_name);

        // Guard against oversized upstream responses (OOM protection)
        if raw.len() > Self::MAX_RESPONSE_BYTES {
            tracing::warn!(
                tool = tool_name,
                size = raw.len(),
                "Response exceeds {} bytes, applying plain-text truncation only",
                Self::MAX_RESPONSE_BYTES,
            );
            return self.filter_plain_text(raw, &rules);
        }

        let parsed = serde_json::from_str::<Value>(raw);
        let mut value = match parsed {
            Ok(v) => v,
            Err(_) => {
                // If not valid JSON, apply string-level truncation only
                return self.filter_plain_text(raw, &rules);
            }
        };

        self.apply_pipeline(tool_name, &mut value, &rules);
        serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string())
    }

    fn apply_pipeline(&self, tool_name: &str, value: &mut Value, rules: &MergedRules) {
        // 1. Keep fields (whitelist) — must come first
        if !rules.keep_fields.is_empty() {
            json::keep_fields(value, &rules.keep_fields);
        }

        // 2. Strip fields (blacklist)
        if !rules.strip_fields.is_empty() {
            json::strip_fields(value, &rules.strip_fields);
        }

        // 3. Condense user objects
        if rules.condense_users {
            json::condense_user_objects(value);
        }

        // 4. Strip nulls
        if rules.strip_nulls {
            json::strip_null_fields(value);
        }

        // 5. Flatten single-key wrappers
        if rules.flatten {
            json::flatten_single_key_objects(value);
        }

        // 6. Truncate strings
        json::truncate_strings(value, rules.truncate_strings_at);

        // 7. Collapse arrays
        json::collapse_arrays(value, rules.max_array_items);

        // 8. Custom transforms (pre-compiled at engine creation)
        if !rules.custom_transforms.is_empty() {
            if let Some(compiled) = self.compiled_transforms.get(tool_name) {
                json::apply_custom_transforms(value, compiled);
            } else if let Some(compiled) = self.compiled_transforms.get("") {
                // Fall back to default transforms
                json::apply_custom_transforms(value, compiled);
            }
        }
    }

    fn filter_plain_text(&self, text: &str, rules: &MergedRules) -> String {
        // Use the configured limit, but never exceed the OOM safety cap
        let limit = rules.truncate_strings_at.min(Self::MAX_RESPONSE_BYTES);
        if limit < text.len() {
            let mut end = limit;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let mut truncated = text[..end].to_string();
            truncated.push_str("...[truncated]");
            truncated
        } else {
            text.to_string()
        }
    }
}

/// Compile [`CustomTransform`] patterns into regex objects.
///
/// Invalid patterns are silently skipped.
fn compile_transforms(transforms: &[CustomTransform]) -> Vec<(Regex, String)> {
    transforms
        .iter()
        .filter_map(|t| {
            Regex::new(&t.pattern)
                .ok()
                .map(|re| (re, t.replacement.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;

    fn test_config() -> Config {
        Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap()
    }

    #[test]
    fn test_filter_list_merge_requests() {
        let config = Arc::new(test_config());
        let engine = FilterEngine::new(config);

        let input = json!([{
            "iid": 42,
            "title": "Fix login",
            "state": "opened",
            "author": {"id": 1, "name": "John", "username": "john", "avatar_url": "http://..."},
            "source_branch": "fix-login",
            "target_branch": "main",
            "web_url": "https://gitlab.com/mr/42",
            "description": "A very long description that should not appear",
            "created_at": "2024-01-01",
            "updated_at": "2024-01-02",
            "_links": {"self": "..."},
            "task_completion_status": {"count": 0},
            "time_stats": {},
            "extra_field": true
        }]);

        let result = engine.filter("list_merge_requests", &input.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();

        // Should keep whitelisted fields
        assert!(parsed[0].get("iid").is_some());
        assert!(parsed[0].get("title").is_some());
        assert!(parsed[0].get("state").is_some());
        // Author should be condensed
        assert_eq!(parsed[0]["author"], json!("john"));
        // Should NOT contain stripped/non-whitelisted fields
        assert!(parsed[0].get("description").is_none());
        assert!(parsed[0].get("_links").is_none());
        assert!(parsed[0].get("extra_field").is_none());
    }

    #[test]
    fn test_filter_plain_text_truncation() {
        let config = Arc::new(test_config());
        let engine = FilterEngine::new(config);

        let long_text = "x".repeat(10000);
        let result = engine.filter("get_job_log", &long_text);
        assert!(result.len() < 10000);
        assert!(result.ends_with("...[truncated]"));
    }
}

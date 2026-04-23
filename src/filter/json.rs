//! Low-level JSON manipulation functions for the filter pipeline.
//!
//! Each function operates on a mutable [`serde_json::Value`] in place,
//! recursively traversing objects and arrays. They are designed to be composed
//! in sequence by [`FilterEngine`](super::FilterEngine).
//!
//! All recursive functions are depth-limited to [`MAX_DEPTH`] (128) as a
//! defense-in-depth measure, matching `serde_json`'s default parse limit.

use serde_json::Value;

/// Maximum recursion depth for JSON traversal (matches serde_json's parse limit).
const MAX_DEPTH: usize = 128;

/// Remove all `null` and empty-string (`""`) fields recursively.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::strip_null_fields;
/// let mut v = json!({"a": null, "b": 1, "c": ""});
/// strip_null_fields(&mut v);
/// assert_eq!(v, json!({"b": 1}));
/// ```
pub fn strip_null_fields(value: &mut Value) {
    strip_null_fields_inner(value, 0);
}

fn strip_null_fields_inner(value: &mut Value, depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null() && *v != Value::String(String::new()));
            for v in map.values_mut() {
                strip_null_fields_inner(v, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_null_fields_inner(v, depth + 1);
            }
        }
        _ => {}
    }
}

/// Remove specific fields by name, recursively through all nested objects.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::strip_fields;
/// let mut v = json!({"name": "test", "avatar_url": "http://...", "nested": {"avatar_url": "x"}});
/// strip_fields(&mut v, &["avatar_url".to_string()]);
/// assert_eq!(v, json!({"name": "test", "nested": {}}));
/// ```
pub fn strip_fields(value: &mut Value, fields: &[String]) {
    if fields.is_empty() {
        return;
    }
    strip_fields_inner(value, fields, 0);
}

fn strip_fields_inner(value: &mut Value, fields: &[String], depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            map.retain(|k, _| !fields.iter().any(|f| f == k));
            for v in map.values_mut() {
                strip_fields_inner(v, fields, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_fields_inner(v, fields, depth + 1);
            }
        }
        _ => {}
    }
}

/// Keep only whitelisted fields at the top level of objects.
///
/// Applied recursively to arrays so that each object element is filtered.
/// Nested arrays within kept fields are also filtered.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::keep_fields;
/// let mut v = json!({"iid": 1, "title": "Fix", "extra": true});
/// keep_fields(&mut v, &["iid".to_string(), "title".to_string()]);
/// assert_eq!(v, json!({"iid": 1, "title": "Fix"}));
/// ```
pub fn keep_fields(value: &mut Value, fields: &[String]) {
    if fields.is_empty() {
        return;
    }
    match value {
        Value::Object(map) => {
            map.retain(|k, _| fields.iter().any(|f| f == k));
            // Do NOT recurse into nested arrays/objects — the whitelist
            // applies only to the current object level.  Nested structures
            // (e.g. user objects inside `assignees`) are left intact for
            // later pipeline stages (condense_users, etc.) to handle.
        }
        Value::Array(arr) => {
            // Top-level arrays (e.g. `[{mr1}, {mr2}]`): apply whitelist
            // to each element.
            for v in arr.iter_mut() {
                keep_fields(v, fields);
            }
        }
        _ => {}
    }
}

/// Condense user objects to compact `{id, username}` objects.
///
/// Detects user-like objects by the presence of a `"username"` field and
/// replaces common user keys (`author`, `assignee`, `merged_by`, etc.) with
/// a compact object containing only `id` and `username`. Arrays of users
/// (`assignees`, `reviewers`, `participants`) become arrays of compact
/// objects. Non-user entries in arrays are preserved as-is.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::condense_user_objects;
/// let mut v = json!({"author": {"id": 1, "username": "john", "avatar_url": "..."}});
/// condense_user_objects(&mut v);
/// assert_eq!(v, json!({"author": {"id": 1, "username": "john"}}));
/// ```
pub fn condense_user_objects(value: &mut Value) {
    condense_user_objects_inner(value, 0);
}

fn condense_user_objects_inner(value: &mut Value, depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            let user_keys = ["author", "assignee", "merged_by", "closed_by", "user"];
            for key in &user_keys {
                if let Some(user_val) = map.get_mut(*key) {
                    if let Some(compact) = extract_user_compact(user_val) {
                        *user_val = compact;
                    }
                }
            }
            let user_array_keys = ["assignees", "reviewers", "participants"];
            for key in &user_array_keys {
                if let Some(Value::Array(arr)) = map.get(*key) {
                    let compacted: Vec<Value> = arr
                        .iter()
                        .map(|v| extract_user_compact(v).unwrap_or_else(|| v.clone()))
                        .collect();
                    map.insert(key.to_string(), Value::Array(compacted));
                }
            }
            for v in map.values_mut() {
                condense_user_objects_inner(v, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                condense_user_objects_inner(v, depth + 1);
            }
        }
        _ => {}
    }
}

fn extract_user_compact(value: &Value) -> Option<Value> {
    if let Value::Object(map) = value {
        if let Some(username) = map.get("username").and_then(|v| v.as_str()) {
            let mut compact = serde_json::Map::new();
            if let Some(id) = map.get("id") {
                compact.insert("id".to_string(), id.clone());
            }
            compact.insert("username".to_string(), Value::String(username.to_string()));
            return Some(Value::Object(compact));
        }
    }
    None
}

/// Truncate string values longer than `max_len` characters.
///
/// Truncation is UTF-8 safe: the cut point is adjusted to a valid character
/// boundary. A `"...[truncated]"` suffix is appended to truncated strings.
/// Passing `usize::MAX` is a no-op.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::truncate_strings;
/// let mut v = json!({"body": "a]".repeat(100)});
/// truncate_strings(&mut v, 50);
/// assert!(v["body"].as_str().unwrap().ends_with("...[truncated]"));
/// ```
pub fn truncate_strings(value: &mut Value, max_len: usize) {
    if max_len == usize::MAX {
        return;
    }
    truncate_strings_inner(value, max_len, 0);
}

fn truncate_strings_inner(value: &mut Value, max_len: usize, depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::String(s) => {
            if s.len() > max_len {
                // Find a valid UTF-8 char boundary at or before max_len
                let mut end = max_len;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push_str("...[truncated]");
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                truncate_strings_inner(v, max_len, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                truncate_strings_inner(v, max_len, depth + 1);
            }
        }
        _ => {}
    }
}

/// Collapse arrays longer than `max_items`, appending a summary entry.
///
/// The summary entry is a string like `"... and 7 more"`. Passing
/// `usize::MAX` is a no-op.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::collapse_arrays;
/// let mut v = json!([1, 2, 3, 4, 5]);
/// collapse_arrays(&mut v, 2);
/// assert_eq!(v, json!([1, 2, "... and 3 more"]));
/// ```
pub fn collapse_arrays(value: &mut Value, max_items: usize) {
    if max_items == usize::MAX {
        return;
    }
    collapse_arrays_inner(value, max_items, 0);
}

fn collapse_arrays_inner(value: &mut Value, max_items: usize, depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Array(arr) => {
            // First recurse into items
            for v in arr.iter_mut() {
                collapse_arrays_inner(v, max_items, depth + 1);
            }
            let total = arr.len();
            if total > max_items {
                arr.truncate(max_items);
                arr.push(Value::String(format!("... and {} more", total - max_items)));
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                collapse_arrays_inner(v, max_items, depth + 1);
            }
        }
        _ => {}
    }
}

/// Flatten single-key wrapper objects by unwrapping the inner value.
///
/// `{"data": [...]}` becomes `[...]`. Only unwraps when the inner value is
/// an array or object. Applied recursively.
///
/// # Examples
///
/// ```
/// # use serde_json::json;
/// # use mcp_rtk::filter::json::flatten_single_key_objects;
/// let mut v = json!({"data": [1, 2, 3]});
/// flatten_single_key_objects(&mut v);
/// assert_eq!(v, json!([1, 2, 3]));
/// ```
pub fn flatten_single_key_objects(value: &mut Value) {
    flatten_single_key_objects_inner(value, 0);
}

fn flatten_single_key_objects_inner(value: &mut Value, depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(only_key) = map.keys().next().cloned() {
                    if map
                        .get(&only_key)
                        .is_some_and(|v| v.is_array() || v.is_object())
                    {
                        if let Some(inner) = map.remove(&only_key) {
                            *value = inner;
                            flatten_single_key_objects_inner(value, depth + 1);
                            return;
                        }
                    }
                }
            }
            for v in map.values_mut() {
                flatten_single_key_objects_inner(v, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                flatten_single_key_objects_inner(v, depth + 1);
            }
        }
        _ => {}
    }
}

/// Apply regex-based string replacements to all string values.
///
/// Each transform is a compiled regex and its replacement string (supports
/// `$1`-style capture group references). Transforms are applied in order.
pub fn apply_custom_transforms(value: &mut Value, transforms: &[(regex::Regex, String)]) {
    if transforms.is_empty() {
        return;
    }
    apply_custom_transforms_inner(value, transforms, 0);
}

fn apply_custom_transforms_inner(
    value: &mut Value,
    transforms: &[(regex::Regex, String)],
    depth: usize,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::String(s) => {
            for (re, replacement) in transforms {
                let replaced = re.replace_all(s, replacement.as_str());
                if replaced != s.as_str() {
                    *s = replaced.into_owned();
                }
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                apply_custom_transforms_inner(v, transforms, depth + 1);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                apply_custom_transforms_inner(v, transforms, depth + 1);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_strip_nulls() {
        let mut v = json!({"a": null, "b": 1, "c": "", "d": {"e": null, "f": 2}});
        strip_null_fields(&mut v);
        assert_eq!(v, json!({"b": 1, "d": {"f": 2}}));
    }

    #[test]
    fn test_strip_fields() {
        let mut v =
            json!({"avatar_url": "http://...", "name": "test", "nested": {"avatar_url": "x"}});
        strip_fields(&mut v, &["avatar_url".to_string()]);
        assert_eq!(v, json!({"name": "test", "nested": {}}));
    }

    #[test]
    fn test_keep_fields() {
        let mut v = json!({"iid": 1, "title": "Fix", "description": "long", "extra": true});
        keep_fields(&mut v, &["iid".to_string(), "title".to_string()]);
        assert_eq!(v, json!({"iid": 1, "title": "Fix"}));
    }

    #[test]
    fn test_condense_users() {
        let mut v = json!({
            "author": {"id": 1, "name": "John", "username": "john", "avatar_url": "http://..."},
            "assignees": [
                {"id": 2, "name": "Jane", "username": "jane", "avatar_url": "http://..."}
            ]
        });
        condense_user_objects(&mut v);
        assert_eq!(
            v,
            json!({"author": {"id": 1, "username": "john"}, "assignees": [{"id": 2, "username": "jane"}]})
        );
    }

    #[test]
    fn test_truncate_strings() {
        let mut v = json!({"body": "a".repeat(200)});
        truncate_strings(&mut v, 50);
        let s = v["body"].as_str().unwrap();
        assert!(s.len() < 200);
        assert!(s.ends_with("...[truncated]"));
    }

    #[test]
    fn test_collapse_arrays() {
        let mut v = json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        collapse_arrays(&mut v, 3);
        assert_eq!(v.as_array().unwrap().len(), 4); // 3 items + summary
        assert_eq!(v[3], json!("... and 7 more"));
    }

    #[test]
    fn test_flatten_single_key() {
        let mut v = json!({"data": [1, 2, 3]});
        flatten_single_key_objects(&mut v);
        assert_eq!(v, json!([1, 2, 3]));
    }
}

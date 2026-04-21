#![recursion_limit = "512"]
//! Black-box integration tests for mcp-rtk filter presets.
//!
//! Each test feeds realistic MCP tool responses through the real
//! [`FilterEngine`] with the actual preset config, then asserts on:
//! - Which fields survive / get removed
//! - Minimum savings percentage
//! - No data corruption (key values intact)

#[allow(dead_code)]
mod common;

use common::{assert_savings, generic_engine, gitlab_engine, gl_user, grafana_engine};
use serde_json::{json, Value};

// ===========================================================================
// GITLAB PRESET TESTS
// ===========================================================================

#[test]
fn gitlab_list_merge_requests() {
    let engine = gitlab_engine();
    let raw = json!([{
        "id": 80001, "iid": 1, "project_id": 123,
        "title": "Fix login",
        "state": "opened",
        "author": gl_user("thomas"),
        "source_branch": "fix-login",
        "target_branch": "main",
        "web_url": "https://gitlab.com/mr/1",
        "created_at": "2024-01-01",
        "updated_at": "2024-01-02",
        "description": "x".repeat(2000),
        "merged_by": null,
        "_links": {"self": "https://..."},
        "time_stats": {"time_estimate": 0},
        "task_completion_status": {"count": 0},
        "references": {"short": "!1"},
        "assignees": [gl_user("alice")],
        "reviewers": [gl_user("bob")],
        "labels": ["bug"],
        "draft": false,
        "sha": "abc123",
        "squash": false,
        "has_conflicts": false,
        "user_notes_count": 3,
        "upvotes": 0,
        "downvotes": 0,
        "pipeline": {"id": 999, "status": "success"}
    }]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("list_merge_requests", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // Key fields survive
    assert_eq!(filtered[0]["iid"], 1);
    assert_eq!(filtered[0]["title"], "Fix login");
    assert_eq!(filtered[0]["state"], "opened");
    assert_eq!(filtered[0]["source_branch"], "fix-login");

    // Author condensed to username
    assert_eq!(filtered[0]["author"], "thomas");

    // Verbose fields removed
    assert!(filtered[0].get("id").is_none());
    assert!(filtered[0].get("project_id").is_none());
    assert!(filtered[0].get("description").is_none());
    assert!(filtered[0].get("_links").is_none());
    assert!(filtered[0].get("time_stats").is_none());
    assert!(filtered[0].get("pipeline").is_none());
    assert!(filtered[0].get("sha").is_none());

    assert_savings("list_merge_requests", &raw_str, &filtered_str, 70.0);
}

#[test]
fn gitlab_get_merge_request() {
    let engine = gitlab_engine();
    let raw = json!({
        "id": 80042, "iid": 42, "project_id": 123,
        "title": "Add OAuth2",
        "state": "merged",
        "author": gl_user("thomas"),
        "assignees": [gl_user("alice"), gl_user("bob")],
        "reviewers": [gl_user("charlie")],
        "source_branch": "feat/oauth",
        "target_branch": "main",
        "web_url": "https://gitlab.com/mr/42",
        "description": format!("Long description {}", "x".repeat(3000)),
        "labels": ["feature", "backend"],
        "created_at": "2024-01-01",
        "updated_at": "2024-01-15",
        "merge_status": "can_be_merged",
        "merged_by": gl_user("admin"),
        "_links": {"self": "..."},
        "time_stats": {},
        "task_completion_status": {},
        "references": {},
        "pipeline": {"id": 999},
        "diff_refs": {},
        "sha": "abc"
    });

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("get_merge_request", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // Description kept but truncated
    assert!(filtered.get("description").is_some());
    assert!(filtered["description"].as_str().unwrap().len() < 3100);

    // Users condensed
    assert_eq!(filtered["author"], "thomas");
    assert_eq!(filtered["assignees"], json!(["alice", "bob"]));
    assert_eq!(filtered["reviewers"], json!(["charlie"]));

    // Labels kept
    assert_eq!(filtered["labels"], json!(["feature", "backend"]));

    assert_savings("get_merge_request", &raw_str, &filtered_str, 15.0);
}

#[test]
fn gitlab_list_merge_request_notes() {
    let engine = gitlab_engine();
    let raw = json!([
        {
            "id": 1001,
            "body": format!("Good work! {}", "x".repeat(2000)),
            "author": gl_user("alice"),
            "created_at": "2024-01-05",
            "updated_at": "2024-01-05",
            "system": false,
            "type": "DiffNote",
            "noteable_type": "MergeRequest",
            "noteable_id": 42,
            "attachment": null,
            "resolvable": true,
            "resolved": false,
            "position": {"base_sha": "abc", "head_sha": "def"}
        },
        {
            "id": 1002,
            "body": "Merged",
            "author": gl_user("thomas"),
            "created_at": "2024-01-06",
            "system": true,
            "type": null,
            "noteable_type": "MergeRequest",
            "noteable_id": 42,
            "attachment": null
        }
    ]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("list_merge_request_notes", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // Body truncated
    assert!(filtered[0]["body"].as_str().unwrap().len() < 1600);
    // Author condensed
    assert_eq!(filtered[0]["author"], "alice");
    // system field kept
    assert_eq!(filtered[1]["system"], true);
    // Verbose fields removed
    assert!(filtered[0].get("id").is_none());
    assert!(filtered[0].get("noteable_type").is_none());
    assert!(filtered[0].get("position").is_none());

    assert_savings("list_merge_request_notes", &raw_str, &filtered_str, 30.0);
}

#[test]
fn gitlab_list_issues() {
    let engine = gitlab_engine();
    let raw = json!([{
        "id": 5000, "iid": 10, "project_id": 123,
        "title": "Bug: login fails",
        "state": "opened",
        "author": gl_user("thomas"),
        "labels": ["bug", "P1"],
        "description": "x".repeat(1000),
        "created_at": "2024-01-01",
        "updated_at": "2024-01-02",
        "web_url": "https://gitlab.com/issues/10",
        "_links": {},
        "time_stats": {},
        "task_completion_status": {},
        "references": {},
        "assignees": [gl_user("alice")],
        "milestone": null,
        "closed_by": null,
        "upvotes": 0,
        "downvotes": 0,
        "user_notes_count": 5,
        "due_date": null,
        "confidential": false,
        "weight": null
    }]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("list_issues", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    assert_eq!(filtered[0]["iid"], 10);
    assert_eq!(filtered[0]["title"], "Bug: login fails");
    assert_eq!(filtered[0]["author"], "thomas");
    assert!(filtered[0].get("id").is_none());
    assert!(filtered[0].get("description").is_none());

    assert_savings("list_issues", &raw_str, &filtered_str, 60.0);
}

#[test]
fn gitlab_get_merge_request_diffs() {
    let engine = gitlab_engine();
    let raw = json!([{
        "old_path": "src/main.rs",
        "new_path": "src/main.rs",
        "a_mode": "100644",
        "b_mode": "100644",
        "new_file": false,
        "renamed_file": false,
        "deleted_file": false,
        "diff": format!("@@ -1,5 +1,10 @@\n+added line\n {}", "x".repeat(5000)),
        "too_large": false,
        "binary": false,
        "generated_file": false
    }]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("get_merge_request_diffs", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    assert_eq!(filtered[0]["old_path"], "src/main.rs");
    assert!(filtered[0].get("a_mode").is_none());
    assert!(filtered[0].get("binary").is_none());
    // Diff truncated
    assert!(filtered[0]["diff"].as_str().unwrap().len() < 2100);

    assert_savings("get_merge_request_diffs", &raw_str, &filtered_str, 40.0);
}

#[test]
fn gitlab_list_pipelines() {
    let engine = gitlab_engine();
    let raw = json!([{
        "id": 999, "iid": 50, "project_id": 123,
        "sha": "abc123",
        "ref": "main",
        "status": "success",
        "source": "push",
        "created_at": "2024-01-01",
        "updated_at": "2024-01-01",
        "web_url": "https://gitlab.com/pipelines/999",
        "name": "build",
        "yaml_errors": null
    }]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("list_pipelines", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    assert_eq!(filtered[0]["status"], "success");
    assert_eq!(filtered[0]["sha"], "abc123");
    assert!(filtered[0].get("project_id").is_none());
    assert!(filtered[0].get("yaml_errors").is_none());

    assert_savings("list_pipelines", &raw_str, &filtered_str, 30.0);
}

// ===========================================================================
// GRAFANA PRESET TESTS
// ===========================================================================

#[test]
fn grafana_search_dashboards() {
    let engine = grafana_engine();
    let raw = json!([
        {
            "id": 11, "orgId": 1,
            "permanentlyDeleteDate": "0001-01-01T00:00:00.000Z",
            "tags": ["logs", "loki"],
            "title": "API Logs V2",
            "type": "dash-db",
            "uid": "cab2ab8a-fcad",
            "uri": "db/api-logs-v2",
            "url": "/d/cab2ab8a-fcad/api-logs-v2",
            "folderId": 8,
            "folderTitle": "BI Netir",
            "folderUid": "df56d4f7doyyod",
            "folderUrl": "/dashboards/f/df56d4f7doyyod/bi-netir"
        },
        {
            "id": 12, "orgId": 1,
            "permanentlyDeleteDate": "0001-01-01T00:00:00.000Z",
            "tags": [],
            "title": "Node Exporter",
            "type": "dash-db",
            "uid": "rYdddlPWk",
            "uri": "db/node-exporter",
            "url": "/d/rYdddlPWk/node-exporter"
        }
    ]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("search_dashboards", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // Key fields survive
    assert_eq!(filtered[0]["title"], "API Logs V2");
    assert_eq!(filtered[0]["uid"], "cab2ab8a-fcad");
    assert!(filtered[0].get("url").is_some());
    assert!(filtered[0].get("folderTitle").is_some());

    // Verbose fields removed
    assert!(filtered[0].get("id").is_none());
    assert!(filtered[0].get("orgId").is_none());
    assert!(filtered[0].get("permanentlyDeleteDate").is_none());
    assert!(filtered[0].get("uri").is_none());
    assert!(filtered[0].get("folderId").is_none());
    assert!(filtered[0].get("folderUrl").is_none());

    assert_savings("search_dashboards", &raw_str, &filtered_str, 30.0);
}

#[test]
fn grafana_get_datasource_by_uid() {
    let engine = grafana_engine();
    let raw = json!({
        "access": "proxy",
        "id": 1,
        "isDefault": true,
        "jsonData": {
            "httpHeaderName1": "x-scope-orgid",
            "httpHeaderName2": "cf-access-client-id",
            "prometheusType": "Mimir",
            "timeInterval": "15s",
            "tlsSkipVerify": false
        },
        "name": "Mimir",
        "orgId": 1,
        "secureJsonFields": {"httpHeaderValue1": true},
        "type": "prometheus",
        "typeLogoUrl": "public/plugins/prometheus/img/prometheus_logo.svg",
        "uid": "PAE45454D0EDB9216",
        "url": "https://mimir.france-nuage.fr/prometheus",
        "version": 1
    });

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("get_datasource_by_uid", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    assert_eq!(filtered["name"], "Mimir");
    assert_eq!(filtered["type"], "prometheus");
    assert_eq!(filtered["uid"], "PAE45454D0EDB9216");
    assert!(filtered.get("secureJsonFields").is_none());
    assert!(filtered.get("jsonData").is_none());
    assert!(filtered.get("typeLogoUrl").is_none());
    assert!(filtered.get("access").is_none());
    assert!(filtered.get("version").is_none());

    assert_savings("get_datasource_by_uid", &raw_str, &filtered_str, 40.0);
}

#[test]
fn grafana_get_dashboard_summary() {
    let engine = grafana_engine();
    let raw = json!({
        "uid": "rYdddlPWk",
        "title": "Node Exporter Full",
        "tags": ["linux"],
        "panelCount": 128,
        "panels": [
            {"id": 261, "title": "CPU / Mem / Disk", "type": "row", "queryCount": 0},
            {"id": 20, "title": "CPU Busy", "type": "gauge", "description": "Overall CPU busy percentage", "queryCount": 1},
        ],
        "variables": [{"name": "DS_PROMETHEUS", "type": "datasource", "label": "Datasource"}],
        "timeRange": {"from": "now-24h", "to": "now"},
        "refresh": "1m",
        "meta": {
            "annotationsPermissions": {"dashboard": {"canAdd": true, "canDelete": true, "canEdit": true}},
            "apiVersion": "v0alpha1",
            "canDelete": true, "canEdit": true, "canSave": true, "canStar": true,
            "created": "2025-08-19T13:21:29.000Z",
            "createdBy": "netir",
            "expires": "0001-01-01T00:00:00.000Z",
            "folderTitle": "General",
            "slug": "node-exporter-full",
            "type": "db",
            "updated": "2025-08-19T13:32:50.000Z",
            "updatedBy": "netir",
            "url": "/d/rYdddlPWk/node-exporter-full",
            "version": 5
        }
    });

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("get_dashboard_summary", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    assert_eq!(filtered["title"], "Node Exporter Full");
    assert_eq!(filtered["panelCount"], 128);
    assert!(filtered.get("panels").is_some());

    // meta block removed
    assert!(filtered.get("meta").is_none());

    assert_savings("get_dashboard_summary", &raw_str, &filtered_str, 40.0);
}

#[test]
fn grafana_query_prometheus() {
    let engine = grafana_engine();
    let raw = json!([
        {
            "metric": {
                "__name__": "up",
                "__replica__": "_replica_dc01_",
                "cluster": "netir-alloy",
                "instance": "postgres:28903",
                "job": "integrations/postgres"
            },
            "value": [1773326214.819, "1"]
        },
        {
            "metric": {
                "__name__": "up",
                "__replica__": "_replica_dc02_",
                "cluster": "netir-alloy",
                "instance": "redis:6379",
                "job": "integrations/redis"
            },
            "value": [1773326214.819, "1"]
        }
    ]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("query_prometheus", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // __replica__ stripped
    assert!(filtered[0]["metric"].get("__replica__").is_none());
    assert!(filtered[1]["metric"].get("__replica__").is_none());

    // Other labels kept
    assert_eq!(filtered[0]["metric"]["job"], "integrations/postgres");
    assert!(filtered[0].get("value").is_some());

    assert_savings("query_prometheus", &raw_str, &filtered_str, 10.0);
}

#[test]
fn grafana_query_loki_logs() {
    let engine = grafana_engine();
    let raw = json!([
        {
            "timestamp": "1773326540875646890",
            "line": format!("2026-03-12T14:42:20 netir-api: {}", "x".repeat(3000)),
            "labels": {
                "filename": "/var/log/syslog",
                "job": "system-logs",
                "service_name": "system-logs"
            }
        },
        {
            "timestamp": "1773326540875646891",
            "line": "2026-03-12T14:42:21 short log",
            "labels": {
                "filename": "/var/log/auth.log",
                "job": "system-logs",
                "service_name": "system-logs"
            }
        }
    ]);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("query_loki_logs", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // filename stripped
    assert!(filtered[0]["labels"].get("filename").is_none());
    assert!(filtered[1]["labels"].get("filename").is_none());

    // Other labels kept
    assert_eq!(filtered[0]["labels"]["job"], "system-logs");

    // Long lines truncated
    assert!(filtered[0]["line"].as_str().unwrap().len() < 1600);

    assert_savings("query_loki_logs", &raw_str, &filtered_str, 30.0);
}

// ===========================================================================
// GENERIC / EDGE CASE TESTS
// ===========================================================================

#[test]
fn generic_strip_nulls_and_flatten() {
    let engine = generic_engine();
    let raw = json!({
        "data": [{
            "id": 1,
            "name": "test",
            "avatar_url": null,
            "description": "",
            "nested": {"foo": null, "bar": 42}
        }]
    });

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("some_tool", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    // Flatten: {"data": [...]} → [...]
    assert!(filtered.is_array());

    // Strip nulls + empty strings
    assert!(filtered[0].get("avatar_url").is_none());
    assert!(filtered[0].get("description").is_none());
    assert!(filtered[0]["nested"].get("foo").is_none());
    assert_eq!(filtered[0]["nested"]["bar"], 42);
}

#[test]
fn generic_plain_text_truncation() {
    let engine = generic_engine();
    let raw = "x".repeat(5000);
    let filtered = engine.filter("any_tool", &raw);

    assert!(filtered.len() < 1100);
    assert!(filtered.ends_with("...[truncated]"));
}

#[test]
fn generic_array_collapse() {
    let engine = generic_engine();
    let items: Vec<Value> = (0..50)
        .map(|i| json!({"id": i, "name": format!("item_{i}")}))
        .collect();
    let raw = json!(items);

    let raw_str = raw.to_string();
    let filtered_str = engine.filter("some_tool", &raw_str);
    let filtered: Value = serde_json::from_str(&filtered_str).unwrap();

    let arr = filtered.as_array().unwrap();
    assert!(arr.len() <= 31); // 30 items + summary
    assert!(arr.last().unwrap().as_str().unwrap().contains("more"));
}

#[test]
fn passthrough_non_text_content() {
    // Non-JSON strings should still be truncated
    let engine = generic_engine();
    let raw = format!("Some log output:\n{}", "line\n".repeat(500));
    let filtered = engine.filter("get_log", &raw);
    assert!(filtered.len() < raw.len());
}

#[test]
fn empty_input_no_crash() {
    let engine = generic_engine();
    assert_eq!(engine.filter("tool", ""), "");
    assert_eq!(engine.filter("tool", "{}"), "{}");
    assert_eq!(engine.filter("tool", "[]"), "[]");
    assert_eq!(engine.filter("tool", "null"), "null");
}

// ===========================================================================
// GAIN --export json TESTS
// ===========================================================================

#[test]
fn gain_export_json_format() {
    let temp_dir = std::env::temp_dir().join("mcp-rtk-test-export");
    let _ = std::fs::create_dir_all(&temp_dir);
    let db_path = temp_dir.join("test-export.db");
    let _ = std::fs::remove_file(&db_path);

    let tracker = mcp_rtk::tracking::Tracker::new(db_path.to_str().unwrap()).unwrap();
    tracker
        .track("list_issues", &"x".repeat(1000), &"x".repeat(100), "gitlab")
        .unwrap();
    tracker
        .track(
            "get_merge_request",
            &"x".repeat(2000),
            &"x".repeat(500),
            "gitlab",
        )
        .unwrap();
    tracker
        .track(
            "search_dashboards",
            &"x".repeat(500),
            &"x".repeat(300),
            "grafana",
        )
        .unwrap();

    let parsed = tracker.stats_as_json().unwrap();

    assert_eq!(parsed["total_calls"], 3);
    assert!(parsed["total_saved_bytes"].as_i64().unwrap() > 0);
    assert!(parsed["total_saved_tokens"].as_i64().unwrap() > 0);
    assert!(parsed["savings_pct"].as_f64().unwrap() > 0.0);
    assert!(parsed["presets"]["gitlab"]["tools"]["list_issues"].is_object());
    assert!(parsed["presets"]["grafana"]["tools"]["search_dashboards"].is_object());

    // Verify it round-trips through JSON serialization
    let json_str = serde_json::to_string_pretty(&parsed).unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(reparsed["total_calls"], 3);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ===========================================================================
// VALIDATE-PRESET TESTS
// ===========================================================================

#[test]
fn validate_preset_valid_file() {
    // Write a valid preset to a temp file and validate it
    let temp_dir = std::env::temp_dir().join("mcp-rtk-test-validate");
    let _ = std::fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("test-preset.toml");

    std::fs::write(
        &path,
        r#"
[tools.list_items]
keep_fields = ["id", "name", "status"]
max_array_items = 20
condense_users = true

[tools.get_item]
truncate_strings_at = 500
strip_fields = ["internal_id"]
"#,
    )
    .unwrap();

    let result = mcp_rtk::config::validate_preset_file(&path);
    assert!(result.is_ok());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn validate_preset_invalid_toml() {
    let temp_dir = std::env::temp_dir().join("mcp-rtk-test-validate-invalid");
    let _ = std::fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("bad.toml");

    std::fs::write(&path, "this is not valid toml {{{{").unwrap();

    let result = mcp_rtk::config::validate_preset_file(&path);
    assert!(result.is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ===========================================================================
// PRESETS SUBCOMMAND TESTS
// ===========================================================================

#[test]
fn presets_list_includes_gitlab_and_grafana() {
    let presets = mcp_rtk::config::Config::available_presets();
    assert!(presets.iter().any(|p| p == "gitlab"));
    assert!(presets.iter().any(|p| p == "grafana"));
}

#[test]
fn show_preset_unknown_fails() {
    let result = mcp_rtk::config::show_preset("nonexistent");
    assert!(result.is_err());
}

#[test]
fn show_preset_gitlab_succeeds() {
    let result = mcp_rtk::config::show_preset("gitlab");
    assert!(result.is_ok());
}

// ===========================================================================
// EXTERNAL PRESET TESTS
// ===========================================================================

#[test]
fn external_preset_loaded_and_applied() {
    use mcp_rtk::config::Config;
    use mcp_rtk::filter::FilterEngine;
    use std::sync::Arc;

    // Create a temp dir to act as external presets dir
    let temp = std::env::temp_dir().join("mcp-rtk-test-ext-apply");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Write an external preset
    std::fs::write(
        temp.join("custom-api.toml"),
        r#"
[meta]
keywords = ["custom-api-mcp"]

[tools.list_items]
keep_fields = ["id", "name", "status"]
max_array_items = 5
condense_users = true
"#,
    )
    .unwrap();

    // Load it manually (since we can't override the presets dir)
    let externals = [mcp_rtk::config::ExternalPreset {
        name: "custom-api".to_string(),
        keywords: vec!["custom-api-mcp".to_string()],
        config: toml::from_str(
            r#"
[tools.list_items]
keep_fields = ["id", "name", "status"]
max_array_items = 5
condense_users = true
"#,
        )
        .unwrap(),
        path: temp.join("custom-api.toml"),
    }];

    // Verify the preset is found by name
    let preset = externals.iter().find(|e| e.name == "custom-api");
    assert!(preset.is_some());
    assert!(preset.unwrap().config.tools.contains_key("list_items"));

    // Build a config with this preset's rules applied
    let mut config = Config::from_upstream(&["echo", "test"], None).unwrap();
    let preset_config = &preset.unwrap().config;
    for (k, v) in &preset_config.tools {
        config.filters.tools.insert(k.clone(), v.clone());
    }
    config.preset = Some("custom-api".to_string());

    let engine = FilterEngine::new(Arc::new(config));

    // Test filtering
    let raw = serde_json::json!([
        {"id": 1, "name": "Item 1", "status": "active", "description": "long text", "extra": "noise"},
        {"id": 2, "name": "Item 2", "status": "inactive", "description": "more text", "extra": "noise"},
    ]);

    let filtered_str = engine.filter("list_items", &raw.to_string());
    let filtered: serde_json::Value = serde_json::from_str(&filtered_str).unwrap();

    // keep_fields should work: only id, name, status survive
    assert_eq!(filtered[0]["id"], 1);
    assert_eq!(filtered[0]["name"], "Item 1");
    assert_eq!(filtered[0]["status"], "active");
    assert!(filtered[0].get("description").is_none());
    assert!(filtered[0].get("extra").is_none());

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn external_preset_without_meta_has_no_keywords() {
    let externals = mcp_rtk::config::Config::load_external_presets();
    // This test just verifies that load_external_presets doesn't crash
    // and returns a valid vec (may be empty in CI)
    for ext in &externals {
        assert!(!ext.name.is_empty());
    }
}

#[test]
fn external_preset_meta_parsing() {
    let toml_with_meta: mcp_rtk::config::PresetConfig = toml::from_str(
        r#"
[meta]
keywords = ["my-server", "myserver"]

[tools.get_data]
keep_fields = ["id", "value"]
truncate_strings_at = 500
"#,
    )
    .unwrap();

    let meta = toml_with_meta.meta.unwrap();
    assert_eq!(meta.keywords, vec!["my-server", "myserver"]);
    assert!(toml_with_meta.tools.contains_key("get_data"));
    assert_eq!(
        toml_with_meta.tools["get_data"].keep_fields,
        vec!["id", "value"]
    );
}

#[test]
fn config_build_with_preset_override() {
    use mcp_rtk::config::Config;

    let config = Config::build(&["echo", "test"], None, Some("gitlab")).unwrap();
    assert_eq!(config.preset, Some("gitlab".to_string()));
    let rules = config.get_tool_rules("list_merge_requests");
    assert!(!rules.keep_fields.is_empty());
}

#[test]
fn config_build_without_override() {
    use mcp_rtk::config::Config;

    let config = Config::build(&["echo", "test"], None, None).unwrap();
    assert_eq!(config.preset, None);
    // Should still have generic defaults
    let rules = config.get_tool_rules("any_tool");
    assert!(rules.strip_nulls);
}

// ===========================================================================
// HOT RELOAD TESTS
// ===========================================================================

#[tokio::test]
async fn hot_reloader_starts_with_valid_config() {
    use mcp_rtk::hot_reload::HotReloader;

    let reloader =
        HotReloader::start(vec!["echo".into(), "test-server".into()], None, None).unwrap();

    let engine = reloader.engine().load_full();
    // Should have generic defaults
    let rules = engine.config().get_tool_rules("any_tool");
    assert!(rules.strip_nulls);
    assert!(rules.condense_users);
}

#[tokio::test]
async fn hot_reloader_starts_with_preset_override() {
    use mcp_rtk::hot_reload::HotReloader;

    let reloader = HotReloader::start(
        vec!["echo".into(), "test-server".into()],
        None,
        Some("gitlab".into()),
    )
    .unwrap();

    let engine = reloader.engine().load_full();
    assert_eq!(engine.config().preset, Some("gitlab".to_string()));
    let rules = engine.config().get_tool_rules("list_merge_requests");
    assert!(!rules.keep_fields.is_empty());
}

// ===========================================================================
// DRY-RUN TESTS
// ===========================================================================

#[test]
fn dry_run_filters_json() {
    use mcp_rtk::config::Config;
    use mcp_rtk::filter::FilterEngine;
    use std::sync::Arc;

    // Simulate dry-run by creating a config with gitlab preset and filtering
    let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap();
    let engine = FilterEngine::new(Arc::new(config));

    let input = json!([{
        "id": 5000, "iid": 10,
        "title": "Test issue",
        "state": "opened",
        "author": {"id": 1, "username": "thomas", "name": "Thomas", "avatar_url": "http://..."},
        "labels": ["bug"],
        "assignees": [],
        "created_at": "2024-01-01",
        "web_url": "https://gitlab.com/issues/10",
        "_links": {},
        "time_stats": {},
        "description": "x".repeat(2000),
        "project_id": 123,
        "extra_field": true
    }]);

    let filtered = engine.filter("list_issues", &input.to_string());
    let parsed: Value = serde_json::from_str(&filtered).unwrap();

    // Fields kept
    assert_eq!(parsed[0]["iid"], 10);
    assert_eq!(parsed[0]["title"], "Test issue");
    assert_eq!(parsed[0]["author"], "thomas");

    // Fields removed
    assert!(parsed[0].get("id").is_none());
    assert!(parsed[0].get("description").is_none());
    assert!(parsed[0].get("_links").is_none());
    assert!(parsed[0].get("project_id").is_none());
}

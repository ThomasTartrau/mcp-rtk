#![recursion_limit = "512"]
//! Property-based tests for the filter pipeline.
//!
//! Verifies that the pipeline never panics and always produces valid JSON
//! output when given valid JSON input, regardless of structure or content.

#[allow(dead_code)]
mod common;

use mcp_rtk::config::Config;
use mcp_rtk::filter::FilterEngine;
use proptest::prelude::*;
use serde_json::Value;
use std::sync::Arc;

fn gitlab_engine() -> FilterEngine {
    let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap();
    FilterEngine::new(Arc::new(config))
}

fn grafana_engine() -> FilterEngine {
    let config =
        Config::from_upstream(&["/path/to/mcp-grafana", "--enabled-tools=all"], None).unwrap();
    FilterEngine::new(Arc::new(config))
}

fn generic_engine() -> FilterEngine {
    let config = Config::from_upstream(&["echo", "unknown-mcp"], None).unwrap();
    FilterEngine::new(Arc::new(config))
}

// ── JSON value generation strategy ───────────────────────────────────

fn arb_json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| Value::Number(n.into())),
        ".*".prop_map(|s: String| Value::String(s)),
    ];

    leaf.prop_recursive(
        4,  // depth
        64, // max nodes
        8,  // items per collection
        |inner| {
            prop_oneof![
                // Arrays
                prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
                // Objects
                prop::collection::vec(("[a-z_]{1,20}", inner), 0..8)
                    .prop_map(|pairs| { Value::Object(pairs.into_iter().collect(),) }),
            ]
        },
    )
}

fn arb_json_string() -> impl Strategy<Value = String> {
    arb_json_value().prop_map(|v| serde_json::to_string(&v).unwrap())
}

// Strategy that generates JSON with user-like objects
fn arb_json_with_users() -> impl Strategy<Value = String> {
    ("[a-z]{3,10}", prop::option::of("[a-z]{3,10}")).prop_map(|(username, name)| {
        let mut user_map = serde_json::Map::new();
        user_map.insert("username".to_string(), Value::String(username));
        if let Some(n) = name {
            user_map.insert("name".to_string(), Value::String(n));
        }
        user_map.insert("id".to_string(), Value::Number(42.into()));
        user_map.insert(
            "avatar_url".to_string(),
            Value::String("http://example.com/avatar.png".to_string()),
        );
        let user = Value::Object(user_map);

        let mut map = serde_json::Map::new();
        map.insert("author".to_string(), user.clone());
        map.insert("assignees".to_string(), Value::Array(vec![user]));
        map.insert("title".to_string(), Value::String("test".to_string()));
        map.insert("iid".to_string(), Value::Number(1.into()));
        serde_json::to_string(&Value::Object(map)).unwrap()
    })
}

// ── Properties ───────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn filter_never_panics_on_arbitrary_json(input in arb_json_string()) {
        let engine = gitlab_engine();
        let tools = [
            "list_merge_requests",
            "get_merge_request",
            "list_issues",
            "list_pipelines",
            "get_job_log",
            "unknown_tool",
        ];
        for tool in &tools {
            let _ = engine.filter(tool, &input);
        }
    }

    #[test]
    fn filter_never_panics_on_arbitrary_bytes(input in ".*") {
        let engine = generic_engine();
        let _ = engine.filter("any_tool", &input);
    }

    #[test]
    fn filter_output_is_valid_json_when_input_is_valid_json(input in arb_json_string()) {
        let engine = gitlab_engine();
        let result = engine.filter("list_merge_requests", &input);
        // If input was valid JSON, output must be valid JSON
        let parsed = serde_json::from_str::<Value>(&result);
        prop_assert!(parsed.is_ok(), "Invalid JSON output: {}", result);
    }

    #[test]
    fn filter_output_size_leq_input_size(input in arb_json_string()) {
        let engine = generic_engine();
        let result = engine.filter("any_tool", &input);
        // Filtered output should never be larger than input (or at most marginally
        // due to "...[truncated]" suffixes on very small inputs)
        // Allow 20 bytes overhead for truncation markers
        prop_assert!(
            result.len() <= input.len() + 20,
            "Output ({}) larger than input ({}) + 20",
            result.len(),
            input.len(),
        );
    }

    #[test]
    fn grafana_filter_never_panics(input in arb_json_string()) {
        let engine = grafana_engine();
        let tools = [
            "search_dashboards",
            "get_datasource_by_uid",
            "query_prometheus",
            "query_loki_logs",
        ];
        for tool in &tools {
            let _ = engine.filter(tool, &input);
        }
    }

    #[test]
    fn condense_users_preserves_valid_json(input in arb_json_with_users()) {
        let engine = gitlab_engine();
        let result = engine.filter("list_merge_requests", &input);
        let parsed = serde_json::from_str::<Value>(&result);
        prop_assert!(parsed.is_ok(), "Invalid JSON after user condensing: {}", result);
    }

    #[test]
    fn pipeline_idempotent_on_second_pass(input in arb_json_string()) {
        let engine = generic_engine();
        let first = engine.filter("any_tool", &input);
        let second = engine.filter("any_tool", &first);
        // Second pass should not change much (idempotent-ish)
        // At minimum, it should still be valid
        if serde_json::from_str::<Value>(&first).is_ok() {
            let parsed = serde_json::from_str::<Value>(&second);
            prop_assert!(parsed.is_ok(), "Second pass broke JSON: {}", second);
        }
    }

    #[test]
    fn empty_and_edge_inputs_never_panic(
        input in prop_oneof![
            Just("".to_string()),
            Just("{}".to_string()),
            Just("[]".to_string()),
            Just("null".to_string()),
            Just("\"\"".to_string()),
            Just("0".to_string()),
            Just("true".to_string()),
            Just("false".to_string()),
            Just("{\"a\":null}".to_string()),
            Just("[null,null,null]".to_string()),
        ]
    ) {
        let engine = gitlab_engine();
        let _ = engine.filter("list_merge_requests", &input);
        let _ = engine.filter("get_merge_request", &input);
        let _ = engine.filter("unknown", &input);
    }
}

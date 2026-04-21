#![recursion_limit = "512"]
//! Benchmark tests: measure token savings for realistic MCP payloads.
//!
//! These tests use the real [`FilterEngine`] with actual preset configs,
//! ensuring the benchmarks reflect production behavior.

#[allow(dead_code)]
mod common;

use common::{gitlab_engine, make_mr, print_savings};
use serde_json::{json, Value};

#[test]
fn benchmark_list_merge_requests_savings() {
    let engine = gitlab_engine();
    let raw_response: Value = json!((1..=5).map(|i| make_mr(i as u32)).collect::<Vec<_>>());
    let raw_str = serde_json::to_string(&raw_response).unwrap();

    let filtered_str = engine.filter("list_merge_requests", &raw_str);
    let savings_pct = (1.0 - filtered_str.len() as f64 / raw_str.len() as f64) * 100.0;

    print_savings(
        "list_merge_requests (5 MRs)",
        raw_str.len(),
        filtered_str.len(),
    );
    assert!(
        savings_pct > 80.0,
        "Expected >80% savings, got {savings_pct:.1}%"
    );
}

#[test]
fn benchmark_get_merge_request_savings() {
    let engine = gitlab_engine();
    let raw_response = make_mr(42);
    let raw_str = serde_json::to_string(&raw_response).unwrap();

    let filtered_str = engine.filter("get_merge_request", &raw_str);
    let savings_pct = (1.0 - filtered_str.len() as f64 / raw_str.len() as f64) * 100.0;

    print_savings(
        "get_merge_request (1 MR, with description)",
        raw_str.len(),
        filtered_str.len(),
    );
    assert!(
        savings_pct > 45.0,
        "Expected >45% savings, got {savings_pct:.1}%"
    );
}

//! Criterion benchmarks for the filter pipeline.
//!
//! Tracks filter latency per preset and tool to detect performance regressions.
//! Run with: `cargo bench`
//! CI stores results as artifacts for cross-release comparison.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mcp_rtk::config::Config;
use mcp_rtk::filter::FilterEngine;
use serde_json::{json, Value};
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

fn gl_user(username: &str) -> Value {
    json!({
        "id": 42,
        "name": format!("{username} User"),
        "username": username,
        "avatar_url": format!("https://gitlab.com/uploads/{username}.png"),
        "state": "active",
        "web_url": format!("https://gitlab.com/{username}")
    })
}

fn make_mr(iid: u32) -> Value {
    json!({
        "id": 80000 + iid, "iid": iid, "project_id": 12345,
        "title": format!("feat: implement feature #{iid}"),
        "description": format!("## Description\n{}", "x".repeat(500)),
        "state": "opened",
        "created_at": "2024-03-01T10:00:00.000Z",
        "updated_at": "2024-03-10T15:30:00.000Z",
        "merged_by": null, "target_branch": "main",
        "source_branch": format!("feature/{iid}"),
        "author": gl_user("thomas"),
        "assignees": [gl_user("alice"), gl_user("bob")],
        "reviewers": [gl_user("charlie")],
        "labels": ["feature", "backend"],
        "_links": {"self": "https://gitlab.com/api/..."},
        "time_stats": {"time_estimate": 0},
        "task_completion_status": {"count": 3, "completed_count": 2},
        "pipeline": {"id": 99999, "status": "success"},
        "web_url": format!("https://gitlab.com/mr/{iid}"),
        "sha": "abc123def456", "has_conflicts": false,
        "references": {"short": format!("!{iid}")},
    })
}

fn bench_gitlab_list_mrs(c: &mut Criterion) {
    let engine = gitlab_engine();
    let mut group = c.benchmark_group("gitlab");

    for count in [1, 5, 20] {
        let mrs: Vec<Value> = (1..=count).map(make_mr).collect();
        let raw = serde_json::to_string(&mrs).unwrap();

        group.throughput(Throughput::Bytes(raw.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("list_merge_requests", count),
            &raw,
            |b, raw| {
                b.iter(|| engine.filter("list_merge_requests", raw));
            },
        );
    }

    group.finish();
}

fn bench_gitlab_get_mr(c: &mut Criterion) {
    let engine = gitlab_engine();
    let raw = serde_json::to_string(&make_mr(42)).unwrap();

    c.bench_with_input(
        BenchmarkId::new("gitlab", "get_merge_request"),
        &raw,
        |b, raw| {
            b.iter(|| engine.filter("get_merge_request", raw));
        },
    );
}

fn bench_grafana_search(c: &mut Criterion) {
    let engine = grafana_engine();
    let dashboards: Vec<Value> = (0..20)
        .map(|i| {
            json!({
                "id": i, "orgId": 1,
                "permanentlyDeleteDate": "0001-01-01T00:00:00.000Z",
                "tags": ["monitoring"],
                "title": format!("Dashboard {i}"),
                "type": "dash-db",
                "uid": format!("uid-{i}"),
                "uri": format!("db/dashboard-{i}"),
                "url": format!("/d/uid-{i}/dashboard-{i}"),
                "folderId": 1,
                "folderTitle": "General",
                "folderUid": "general",
                "folderUrl": "/dashboards/f/general"
            })
        })
        .collect();
    let raw = serde_json::to_string(&dashboards).unwrap();

    c.bench_with_input(
        BenchmarkId::new("grafana", "search_dashboards_20"),
        &raw,
        |b, raw| {
            b.iter(|| engine.filter("search_dashboards", raw));
        },
    );
}

fn bench_generic_large_array(c: &mut Criterion) {
    let engine = generic_engine();
    let items: Vec<Value> = (0..100)
        .map(|i| {
            json!({
                "id": i,
                "name": format!("item_{i}"),
                "description": "x".repeat(200),
                "avatar_url": null,
                "extra": "",
            })
        })
        .collect();
    let raw = serde_json::to_string(&json!({"data": items})).unwrap();

    c.bench_with_input(
        BenchmarkId::new("generic", "large_array_100_items"),
        &raw,
        |b, raw| {
            b.iter(|| engine.filter("any_tool", raw));
        },
    );
}

fn bench_plain_text(c: &mut Criterion) {
    let engine = generic_engine();
    let raw = "x".repeat(50_000);

    c.bench_with_input(
        BenchmarkId::new("generic", "plain_text_50kb"),
        &raw,
        |b, raw| {
            b.iter(|| engine.filter("get_job_log", raw));
        },
    );
}

criterion_group!(
    benches,
    bench_gitlab_list_mrs,
    bench_gitlab_get_mr,
    bench_grafana_search,
    bench_generic_large_array,
    bench_plain_text,
);
criterion_main!(benches);

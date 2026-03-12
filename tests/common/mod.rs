//! Shared test helpers for integration and benchmark tests.

use mcp_rtk::config::Config;
use mcp_rtk::filter::FilterEngine;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn gitlab_engine() -> FilterEngine {
    let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap();
    FilterEngine::new(Arc::new(config))
}

pub fn grafana_engine() -> FilterEngine {
    let config =
        Config::from_upstream(&["/path/to/mcp-grafana", "--enabled-tools=all"], None).unwrap();
    FilterEngine::new(Arc::new(config))
}

pub fn generic_engine() -> FilterEngine {
    let config = Config::from_upstream(&["echo", "unknown-mcp"], None).unwrap();
    FilterEngine::new(Arc::new(config))
}

/// Assert minimum savings percentage.
pub fn assert_savings(tool: &str, raw: &str, filtered: &str, min_pct: f64) {
    let pct = (1.0 - filtered.len() as f64 / raw.len().max(1) as f64) * 100.0;
    assert!(
        pct >= min_pct,
        "{tool}: expected >= {min_pct:.0}% savings, got {pct:.1}% (raw={}, filtered={})",
        raw.len(),
        filtered.len(),
    );
}

/// Create a realistic GitLab user object.
pub fn gl_user(username: &str) -> Value {
    json!({
        "id": 42,
        "name": format!("{username} User"),
        "username": username,
        "avatar_url": format!("https://gitlab.com/uploads/{username}.png"),
        "state": "active",
        "web_url": format!("https://gitlab.com/{username}")
    })
}

/// Create a realistic GitLab merge request object.
pub fn make_mr(iid: u32) -> Value {
    json!({
        "id": 80000 + iid,
        "iid": iid,
        "project_id": 12345,
        "title": format!("feat: implement feature #{iid} with comprehensive changes"),
        "description": format!("## Description\n\nThis MR implements feature #{iid}.\n\n### Changes\n- Updated the authentication module\n- Added new API endpoints for user management\n- Refactored the database layer for better performance\n- Added comprehensive test coverage\n\n### Testing\n- [x] Unit tests\n- [x] Integration tests\n- [ ] E2E tests\n\n### Screenshots\nN/A\n\n### Related Issues\nCloses #{iid}\n\n{}", "x".repeat(500)),
        "state": "opened",
        "created_at": "2024-03-01T10:00:00.000Z",
        "updated_at": "2024-03-10T15:30:00.000Z",
        "merged_by": null,
        "merged_at": null,
        "closed_by": null,
        "closed_at": null,
        "target_branch": "main",
        "source_branch": format!("feature/{iid}-new-feature"),
        "user_notes_count": 12,
        "upvotes": 3,
        "downvotes": 0,
        "author": gl_user("thomas"),
        "assignees": [gl_user("alice"), gl_user("bob")],
        "assignee": gl_user("alice"),
        "reviewers": [gl_user("charlie")],
        "source_project_id": 12345,
        "target_project_id": 12345,
        "labels": ["feature", "backend", "needs-review"],
        "draft": false,
        "work_in_progress": false,
        "milestone": {
            "id": 50, "iid": 5, "project_id": 12345,
            "title": "Sprint 42", "description": "March sprint",
            "state": "active", "created_at": "2024-02-01",
            "updated_at": "2024-03-01", "due_date": "2024-03-15",
            "start_date": "2024-03-01", "web_url": "https://gitlab.com/milestone/5"
        },
        "merge_when_pipeline_succeeds": false,
        "merge_status": "can_be_merged",
        "sha": "abc123def456abc123def456abc123def456abc1",
        "merge_commit_sha": null,
        "squash_commit_sha": null,
        "discussion_locked": null,
        "should_remove_source_branch": true,
        "force_remove_source_branch": true,
        "reference": format!("!{iid}"),
        "references": { "short": format!("!{iid}"), "relative": format!("!{iid}"), "full": format!("my-group/my-project!{iid}") },
        "web_url": format!("https://gitlab.com/my-group/my-project/-/merge_requests/{iid}"),
        "time_stats": {
            "time_estimate": 0, "total_time_spent": 0,
            "human_time_estimate": null, "human_total_time_spent": null
        },
        "squash": true,
        "task_completion_status": { "count": 3, "completed_count": 2 },
        "has_conflicts": false,
        "blocking_discussions_resolved": true,
        "approvals_before_merge": null,
        "subscribed": true,
        "changes_count": "15",
        "latest_build_started_at": "2024-03-10T15:00:00.000Z",
        "latest_build_finished_at": "2024-03-10T15:20:00.000Z",
        "first_deployed_to_production_at": null,
        "pipeline": {
            "id": 99999, "iid": 500, "project_id": 12345,
            "sha": "abc123", "ref": format!("feature/{iid}-new-feature"),
            "status": "success", "source": "push",
            "created_at": "2024-03-10T15:00:00.000Z",
            "updated_at": "2024-03-10T15:20:00.000Z",
            "web_url": "https://gitlab.com/pipeline/99999"
        },
        "head_pipeline": {
            "id": 99999, "iid": 500, "project_id": 12345,
            "sha": "abc123", "ref": format!("feature/{iid}-new-feature"),
            "status": "success", "source": "push",
            "created_at": "2024-03-10T15:00:00.000Z",
            "updated_at": "2024-03-10T15:20:00.000Z",
            "web_url": "https://gitlab.com/pipeline/99999"
        },
        "diff_refs": {
            "base_sha": "000000", "head_sha": "abc123", "start_sha": "111111"
        },
        "merge_error": null,
        "first_contribution": false,
        "user": { "can_merge": true },
        "_links": {
            "self": "https://gitlab.com/api/v4/projects/12345/merge_requests/42",
            "notes": "https://gitlab.com/api/v4/projects/12345/merge_requests/42/notes",
            "award_emoji": "https://gitlab.com/api/v4/projects/12345/merge_requests/42/award_emoji",
            "project": "https://gitlab.com/api/v4/projects/12345"
        }
    })
}

/// Print a savings summary for benchmark tests.
pub fn print_savings(label: &str, raw_bytes: usize, filtered_bytes: usize) {
    let savings_pct = (1.0 - filtered_bytes as f64 / raw_bytes as f64) * 100.0;
    println!("\n  ===== {label} =====");
    println!(
        "  Raw response:      {raw_bytes:>6} bytes  (~{} tokens)",
        raw_bytes / 4
    );
    println!(
        "  Filtered response: {filtered_bytes:>6} bytes  (~{} tokens)",
        filtered_bytes / 4
    );
    println!("  Savings:           {savings_pct:.1}%");
    println!("  Tokens saved:      ~{}", (raw_bytes - filtered_bytes) / 4);
    println!();
}

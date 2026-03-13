#![no_main]

use libfuzzer_sys::fuzz_target;
use mcp_rtk::config::Config;
use mcp_rtk::filter::FilterEngine;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    // Test with gitlab preset (most complex)
    let config = Config::from_upstream(&["npx", "@nicepkg/gitlab-mcp"], None).unwrap();
    let engine = FilterEngine::new(Arc::new(config));

    let tool_names = [
        "list_merge_requests",
        "get_merge_request",
        "list_issues",
        "list_pipelines",
        "get_job_log",
        "unknown_tool",
    ];

    for tool in &tool_names {
        let _ = engine.filter(tool, input);
    }
});

#![no_main]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    let Ok(mut value) = serde_json::from_str::<Value>(input) else {
        return;
    };

    // Test each JSON operation individually to find which one crashes
    let mut v = value.clone();
    mcp_rtk::filter::json::strip_null_fields(&mut v);
    assert!(serde_json::to_string(&v).is_ok());

    let mut v = value.clone();
    mcp_rtk::filter::json::strip_fields(&mut v, &["test".to_string(), "avatar_url".to_string()]);
    assert!(serde_json::to_string(&v).is_ok());

    let mut v = value.clone();
    mcp_rtk::filter::json::keep_fields(&mut v, &["id".to_string(), "name".to_string()]);
    assert!(serde_json::to_string(&v).is_ok());

    let mut v = value.clone();
    mcp_rtk::filter::json::condense_user_objects(&mut v);
    assert!(serde_json::to_string(&v).is_ok());

    let mut v = value.clone();
    mcp_rtk::filter::json::truncate_strings(&mut v, 50);
    assert!(serde_json::to_string(&v).is_ok());

    let mut v = value.clone();
    mcp_rtk::filter::json::collapse_arrays(&mut v, 3);
    assert!(serde_json::to_string(&v).is_ok());

    mcp_rtk::filter::json::flatten_single_key_objects(&mut value);
    assert!(serde_json::to_string(&value).is_ok());
});

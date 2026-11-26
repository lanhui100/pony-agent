use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor};
use serde_json::json;
use std::fs;

#[test]
fn test_read_before_write_enforces_fs_not_observed() {
    let _env_guard = crate::env_lock::lock();
    struct AllowAllReset(Option<std::ffi::OsString>);
    impl Drop for AllowAllReset {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => std::env::set_var("PONY_LEGACY_POLICY_ALLOW_ALL", v),
                None => std::env::remove_var("PONY_LEGACY_POLICY_ALLOW_ALL"),
            }
        }
    }
    let _reset = AllowAllReset(std::env::var_os("PONY_LEGACY_POLICY_ALLOW_ALL"));
    let dir = std::env::temp_dir().join(format!("pony-rw-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("create dir");
    let file_path = dir.join("target.txt");
    fs::write(&file_path, "original content").expect("write file");

    std::env::set_var("PONY_LEGACY_POLICY_ALLOW_ALL", "1");
    let executor = build_governed_executor(Some(dir.clone()), None);

    // Call edit_file without reading first -> must fail with fs_not_observed
    let edit_call = ToolCall {
        call_id: Some("call-edit-1".to_string()),
        name: "workspace_edit_file".to_string(),
        arguments: json!({
            "path": "target.txt",
            "oldText": "original",
            "newText": "updated",
            "description": "直接编辑未读文件"
        }),
        plan: None,
    };

    let result = executor.execute(&edit_call);
    let out: serde_json::Value = serde_json::from_str(&result.output).expect("valid json");
    
    // In red phase, edit does not check observation, so it won't report fs_not_observed
    let code = out.get("error").and_then(|e| e.get("code")).or_else(|| out.get("code"));
    assert_eq!(
        code.and_then(|c| c.as_str()),
        Some("fs_not_observed"),
        "Unobserved file edit must be rejected with fs_not_observed, got: {:?}",
        out
    );

    let _ = fs::remove_dir_all(dir);
}

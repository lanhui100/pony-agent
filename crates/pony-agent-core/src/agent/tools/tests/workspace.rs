//! workspace 文件工具：读写/编辑 CAS、路径遍历拒绝、列目录、glob 与搜索。
use super::*;

#[test]
fn search_text_supports_file_path_input_and_wildcard_filter() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src")).expect("create src dir");
    fs::write(workspace.join("src/lib.rs"), "pub struct ToolRouter;\n").expect("write lib");
    fs::write(workspace.join("src/lib.txt"), "ToolRouter in text\n").expect("write txt");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
        arguments: json!({
            "query": "ToolRouter",
            "path": "src/lib.rs",
            "filePattern": "*.rs"
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
    assert_eq!(
        payload.get("pathKind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
    assert_eq!(
        payload
            .get("matches")
            .and_then(Value::as_array)
            .and_then(|matches| matches.first())
            .and_then(|entry| entry.get("path"))
            .and_then(Value::as_str),
        Some("src/lib.rs")
    );
}

#[test]
fn glob_files_matches_paths_by_pattern() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
    fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
    fs::write(
        workspace.join("src/agent/context.rs"),
        "pub struct AgentContext;\n",
    )
    .expect("write context");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
        arguments: json!({
            "pattern": "src/agent/*.rs",
            "path": ".",
            "limit": 10
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(2));
}

#[test]
fn glob_files_respects_limit() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
    fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
    fs::write(
        workspace.join("src/agent/context.rs"),
        "pub struct AgentContext;\n",
    )
    .expect("write context");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
        arguments: json!({
            "pattern": "src/agent/*.rs",
            "path": ".",
            "limit": 1
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
}

#[test]
fn search_text_supports_real_regex_mode() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "alpha beta gamma\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
        arguments: json!({
            "query": "b\\w+ g\\w+",
            "path": "demo.txt",
            "regex": true
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
    assert_eq!(payload.get("regex").and_then(Value::as_bool), Some(true));
}

#[test]
fn search_text_reports_invalid_regex_error() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "alpha beta gamma\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace);

    // `*beta*` was a wildcard pseudo-regex under the legacy search; it is an invalid real
    // regex now (design Decision 9), so the tool must fail with a structured error.
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
        arguments: json!({
            "query": "*beta*",
            "path": "demo.txt",
            "regex": true
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("invalid_regex")
    );
}

#[test]
fn search_text_respects_gitignore_and_reports_truncation() {
    let workspace = temp_workspace();
    fs::write(workspace.join(".gitignore"), "ignored.txt\n").expect("write gitignore");
    fs::write(workspace.join("keep.txt"), "needle-value\n").expect("write keep");
    fs::write(workspace.join("ignored.txt"), "needle-value\n").expect("write ignored");
    fs::write(workspace.join("other.txt"), "needle-value\n").expect("write other");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    // Gitignored files are skipped (design Decision 9): only keep.txt and other.txt match.
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
        arguments: json!({
            "query": "needle",
            "path": ".",
            "limit": 10
        }),
        plan: None,
    });
    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(2));
    assert_eq!(
        payload.get("truncated").and_then(Value::as_bool),
        Some(false)
    );

    // A small `limit` budget truncates honestly instead of pretending full success.
    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
        arguments: json!({
            "query": "needle",
            "path": ".",
            "limit": 1
        }),
        plan: None,
    });
    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
    assert_eq!(
        payload.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    assert!(payload
        .get("truncationReason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .contains("max_matches"));
}

#[test]
fn write_file_creates_new_file_in_workspace() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "notes/demo.txt",
            "content": "hello pony"
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
    assert_eq!(
        payload.get("path").and_then(Value::as_str),
        Some("notes/demo.txt")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("notes/demo.txt")).expect("read written file"),
        "hello pony"
    );
}

#[test]
fn write_file_respects_overwrite_false_for_existing_file() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "original").expect("write original");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "demo.txt",
            "content": "changed",
            "overwrite": false
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("file_exists")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("demo.txt")).expect("read original"),
        "original"
    );
}

#[test]
fn write_file_cas_success_and_conflict_rebase_hint() {
    let workspace = temp_workspace();
    let demo = workspace.join("cas_demo.txt");
    fs::write(&demo, "version 1").expect("write initial");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let initial_hash = compute_content_sha256("version 1");

    // 1. 成功 CAS 写入
    let res_ok = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "cas_demo.txt",
            "content": "version 2",
            "expectedHash": initial_hash
        }),
        plan: None,
    });
    assert_eq!(res_ok.status, "ok");
    let ok_payload = serde_json::from_str::<Value>(&res_ok.output).unwrap();
    assert_eq!(ok_payload.get("ok").and_then(Value::as_bool), Some(true));
    let new_hash = ok_payload.get("contentHash").and_then(Value::as_str).unwrap();
    assert_eq!(new_hash, compute_content_sha256("version 2"));

    // 2. 冲突对抗：使用陈旧 hash 再次写入，必须返回 cas_conflict 并提供自愈提示
    let res_conflict = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
        arguments: json!({
            "path": "cas_demo.txt",
            "content": "version 3 - race",
            "expectedHash": initial_hash
        }),
        plan: None,
    });
    assert_eq!(res_conflict.status, "error");
    let conflict_payload = serde_json::from_str::<Value>(&res_conflict.output).unwrap();
    assert_eq!(conflict_payload.get("code").and_then(Value::as_str), Some("cas_conflict"));
    let details = conflict_payload.get("details").unwrap();
    assert_eq!(details.get("actualHash").and_then(Value::as_str), Some(new_hash));
    assert_eq!(details.get("recoveryHint").and_then(|h| h.get("action")).and_then(Value::as_str), Some("rebase_and_retry"));
}

#[test]
fn edit_file_cas_guards_against_stale_modification() {
    let workspace = temp_workspace();
    let demo = workspace.join("cas_edit.txt");
    fs::write(&demo, "hello world").expect("write initial");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let initial_hash = compute_content_sha256("hello world");

    // 模拟外部并发修改
    fs::write(&demo, "hello universe").expect("external write");

    let res = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
        arguments: json!({
            "path": "cas_edit.txt",
            "oldText": "hello",
            "newText": "hi",
            "expectedHash": initial_hash
        }),
        plan: None,
    });
    assert_eq!(res.status, "error");
    let payload = serde_json::from_str::<Value>(&res.output).unwrap();
    assert_eq!(payload.get("code").and_then(Value::as_str), Some("cas_conflict"));
    assert_eq!(payload.get("details").and_then(|d| d.get("recoveryHint")).and_then(|h| h.get("action")).and_then(Value::as_str), Some("rebase_and_retry"));
}

#[test]
fn edit_file_requires_replace_all_for_multiple_matches() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "foo\nfoo\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
        arguments: json!({
            "path": "demo.txt",
            "oldText": "foo",
            "newText": "bar",
            // wave-22 冻结的读前写不变量:未传 expectedHash ⇒ fs_not_observed。
            "expectedHash": compute_content_sha256("foo\nfoo\n")
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("edit output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("multiple_matches")
    );
}

#[test]
fn edit_file_replaces_all_matches_when_enabled() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "foo\nfoo\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
        arguments: json!({
            "path": "demo.txt",
            "oldText": "foo",
            "newText": "bar",
            "replaceAll": true,
            // wave-22 冻结的读前写不变量:未传 expectedHash ⇒ fs_not_observed。
            "expectedHash": compute_content_sha256("foo\nfoo\n")
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert_eq!(
        fs::read_to_string(workspace.join("demo.txt")).expect("read edited file"),
        "bar\nbar\n"
    );
}

#[test]
fn edit_file_returns_no_match_when_old_text_missing() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "alpha\nbeta\n").expect("write demo");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
        arguments: json!({
            "path": "demo.txt",
            "oldText": "missing",
            "newText": "gamma",
            // wave-22 冻结的读前写不变量:未传 expectedHash ⇒ fs_not_observed。
            "expectedHash": compute_content_sha256("alpha\nbeta\n")
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("edit output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("no_match")
    );
}

#[test]
fn edit_file_allows_authorized_external_path_with_readwrite_scope() {
    // ADR 0029（工作区外部访问提权）：用户显式批准 `read-write` 作用域后，
    // 外部路径写入必须被合法放行。本用例与上面的「只读授权必须拒绝」互为对偶，
    // 防止作用域落地时把提权能力一起改死。
    let workspace = temp_workspace();
    let external_dir =
        std::env::temp_dir().join(format!("pa080-edit-rw-external-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&external_dir);
    std::fs::create_dir_all(&external_dir).expect("create external dir");
    let external_file = external_dir.join("config.toml");
    fs::write(&external_file, "mode = \"old\"\n").expect("write external file");
    let external_file = external_file.canonicalize().unwrap_or(external_file);
    let external_file = crate::agent::path_permission::normalize_canonical(&external_file);

    let authorize_store = crate::agent::path_permission::AuthorizeStore::new();
    authorize_store
        .grant_with_scope(
            external_file.clone(),
            crate::agent::path_permission::PathAuthorizationScope::ReadWrite,
        )
        .expect("grant read-write authorization");
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_authorize_store(std::sync::Arc::new(authorize_store));

    let edit_result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
        arguments: json!({
            "path": external_file.display().to_string(),
            "oldText": "mode = \"old\"",
            "newText": "mode = \"new\"",
            // wave-22 冻结的读前写不变量:未传 expectedHash ⇒ fs_not_observed。
            "expectedHash": compute_content_sha256("mode = \"old\"\n")
        }),
        plan: None,
    });
    assert_eq!(
        edit_result.status, "ok",
        "read-write authorized external edit must succeed: {}",
        edit_result.output
    );
    let content = fs::read_to_string(&external_file).expect("read external file");
    assert_eq!(content, "mode = \"new\"\n");

    let _ = std::fs::remove_dir_all(&external_dir);
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn edit_file_rejects_authorized_external_path_with_write_denied() {
    // PA-080 code-review P0 回归：edit_file 是写操作，即使外部路径有"只读授权"
    // 也必须拒绝（outside_workspace_write_denied），不得把只读授权升级为外部写。
    let workspace = temp_workspace();
    let external_dir =
        std::env::temp_dir().join(format!("pa080-edit-external-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&external_dir);
    std::fs::create_dir_all(&external_dir).expect("create external dir");
    let external_file = external_dir.join("secret.txt");
    fs::write(&external_file, "top secret\n").expect("write external file");
    let external_file = external_file.canonicalize().unwrap_or(external_file);
    let external_file = crate::agent::path_permission::normalize_canonical(&external_file);

    let authorize_store = crate::agent::path_permission::AuthorizeStore::new();
    authorize_store
        .grant(external_file.clone())
        .expect("grant read authorization");
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_authorize_store(std::sync::Arc::new(authorize_store));

    // 读授权放行（读工具可读外部文件）
    let read_result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": external_file.display().to_string() }),
        plan: None,
    });
    assert_eq!(
        read_result.status, "ok",
        "read with authorization must succeed"
    );

    // 但 edit 必须拒绝：写判定不认授权清单
    let edit_result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
        arguments: json!({
            "path": external_file.display().to_string(),
            "oldText": "top secret",
            "newText": "tampered"
        }),
        plan: None,
    });
    assert_eq!(edit_result.status, "error");
    let payload = serde_json::from_str::<Value>(&edit_result.output).expect("edit output json");
    let code = payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(
        code, "outside_workspace_write_denied",
        "edit must fail with outside_workspace_write_denied code, got: {code}"
    );
    // 文件内容未被篡改
    let content = fs::read_to_string(&external_file).expect("read external file");
    assert_eq!(
        content, "top secret\n",
        "external file must not be modified"
    );

    let _ = std::fs::remove_dir_all(&external_dir);
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn concurrent_calls_keep_session_workspace_roots_isolated() {
    // PA-080 consultant P0 回归：显式不可变调用上下文（方案 A）下，同一 Arc<ToolRouter>
    // 并发执行不同会话 root 的工具调用不得串扰——每个调用只能写入自己的 workspace。
    let ws_a = temp_workspace();
    let ws_b = temp_workspace();
    let router = Arc::new(ToolRouter::with_workspace_root(ws_a.clone()));

    let router_for_threads = Arc::clone(&router);
    std::thread::scope(|scope| {
        for (index, (ws, marker)) in [(&ws_a, "a"), (&ws_b, "b")].into_iter().enumerate() {
            let router = Arc::clone(&router_for_threads);
            let ws = ws.clone();
            let marker = marker.to_string();
            scope.spawn(move || {
                for i in 0..5 {
                    let file = format!("concurrent-{marker}-{index}-{i}.txt");
                    // 每个线程只写自己的会话 root。
                    let result = router.execute_with_workspace_root(
                        &ToolCall {
                            call_id: None,
                            name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
                            arguments: json!({
                                "path": file,
                                "content": format!("content-{marker}-{i}"),
                            }),
                            plan: None,
                        },
                        Some(ws.as_path()),
                    );
                    assert_eq!(result.status, "ok", "write {marker} {i}: {}", result.output);
                }
            });
        }
    });

    // 每个 workspace 只应包含自己的文件（无跨会话串扰）。
    let files_a = std::fs::read_dir(&ws_a)
        .expect("read ws_a")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let files_b = std::fs::read_dir(&ws_b)
        .expect("read ws_b")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(files_a.len(), 5, "ws_a files: {files_a:?}");
    assert_eq!(files_b.len(), 5, "ws_b files: {files_b:?}");
    assert!(
        files_a.iter().all(|name| name.starts_with("concurrent-a-")),
        "ws_a must only contain its own files: {files_a:?}"
    );
    assert!(
        files_b.iter().all(|name| name.starts_with("concurrent-b-")),
        "ws_b must only contain its own files: {files_b:?}"
    );

    let _ = std::fs::remove_dir_all(&ws_a);
    let _ = std::fs::remove_dir_all(&ws_b);
}

#[test]
fn read_file_returns_file_content() {
    let workspace = temp_workspace();
    fs::write(workspace.join("hello.txt"), "Hello Pony Agent!\n").expect("write file");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": "hello.txt" }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert!(result.output.contains("Hello Pony Agent!"));
}

#[test]
fn read_file_rejects_missing_path_argument() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("missing_argument")
    );
}

#[test]
fn read_file_rejects_path_traversal() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": "../../../etc/passwd" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    // PA-080 统一路径判定：逃逸工作区的路径先命中 path_permission 边界 →
    // `requires_authorization`（宿主审批语义），不再走 canonicalize 失败的 `invalid_path`。
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("requires_authorization")
    );
}

#[test]
fn read_file_rejects_nonexistent_file() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": "does-not-exist.txt" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    // canonicalize fails first → invalid_path (not metadata_failed)
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("invalid_path")
    );
}

#[test]
fn read_file_spills_oversized_file_instead_of_failing() {
    // W26：超过整文件内联上限（120_000 bytes）不再硬失败，而是落盘溢出并返回 content_ref。
    let workspace = temp_workspace();
    let large = vec![b'X'; 120_001];
    fs::write(workspace.join("large.bin"), &large).expect("write large file");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": "large.bin" }),
        plan: None,
    });

    assert_eq!(result.status, "ok", "超限文件应落盘溢出而非失败：{result:?}");
    let payload: Value =
        serde_json::from_str(&result.output).expect("spilled output should be json");
    assert_eq!(payload.get("spilled").and_then(Value::as_bool), Some(true));
    assert_eq!(payload.get("bytes").and_then(Value::as_u64), Some(120_001));
    let content_ref = payload
        .get("content_ref")
        .and_then(Value::as_str)
        .expect("content_ref required");
    assert!(
        crate::agent::spill::is_valid_ref_id(content_ref),
        "content_ref 必须是 64 位十六进制：{content_ref}"
    );
    let preview = payload.get("preview").and_then(Value::as_str).unwrap_or("");
    assert!(!preview.is_empty(), "spilled 结果必须带非空 preview");
}

#[test]
fn read_file_rejects_file_beyond_spill_cap() {
    // 超过 MAX_SPILL_BYTES 仍必须失败（避免无界磁盘占用）；用稀疏文件快速构造。
    let workspace = temp_workspace();
    let path = workspace.join("huge.bin");
    let file = fs::File::create(&path).expect("create sparse file");
    file.set_len(crate::agent::spill::MAX_SPILL_BYTES + 1)
        .expect("extend sparse file");
    drop(file);
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": "huge.bin" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("file_too_large")
    );
}

#[test]
fn read_file_rejects_non_utf8_content() {
    let workspace = temp_workspace();
    // Write invalid UTF-8 bytes that fs::read_to_string will reject
    let bad_bytes = [0xFF, 0xFE, 0x00, 0x68, 0x69];
    fs::write(workspace.join("binary.bin"), &bad_bytes).expect("write binary file");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_READ_FILE.to_string(),
        arguments: json!({ "path": "binary.bin" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("read_failed")
    );
}

// ── workspace_path_info ─────────────────────────────────────────

#[test]
fn path_info_returns_file_metadata() {
    let workspace = temp_workspace();
    fs::write(workspace.join("info.txt"), "metadata test").expect("write file");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
        arguments: json!({ "path": "info.txt" }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload: Value =
        serde_json::from_str(&result.output).expect("path_info output should be json");
    assert_eq!(payload.get("kind").and_then(Value::as_str), Some("file"));
    assert!(
        payload
            .get("sizeBytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
    assert!(payload
        .get("modifiedUnixSeconds")
        .and_then(Value::as_u64)
        .is_some());
    assert_eq!(payload.get("childCount"), Some(&Value::Null));
}

#[test]
fn path_info_returns_directory_metadata() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("subdir")).expect("create subdir");
    fs::write(workspace.join("subdir/a.txt"), "a").expect("write a");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
        arguments: json!({ "path": "subdir" }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload: Value =
        serde_json::from_str(&result.output).expect("path_info output should be json");
    assert_eq!(
        payload.get("kind").and_then(Value::as_str),
        Some("directory")
    );
    assert_eq!(payload.get("childCount").and_then(Value::as_u64), Some(1));
}

#[test]
fn path_info_rejects_invalid_path() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
        arguments: json!({ "path": "../../../etc" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    // PA-080 统一路径判定：逃逸工作区的路径先命中 path_permission 边界 →
    // `requires_authorization`（宿主审批语义），不再走 canonicalize 失败的 `invalid_path`。
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("requires_authorization")
    );
}

// ── workspace_list_files ────────────────────────────────────────

#[test]
fn list_files_returns_directory_entries() {
    let workspace = temp_workspace();
    fs::write(workspace.join("alpha.txt"), "alpha").expect("write alpha");
    fs::create_dir_all(workspace.join("beta")).expect("create beta dir");
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_LIST_FILES.to_string(),
        arguments: json!({ "path": "." }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert!(result.output.contains("alpha.txt"));
    assert!(result.output.contains("beta/"));
}

#[test]
fn list_files_respects_limit() {
    let workspace = temp_workspace();
    for i in 0..5 {
        fs::write(workspace.join(format!("file-{i}.txt")), &format!("{i}"))
            .expect("write file");
    }
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_LIST_FILES.to_string(),
        arguments: json!({ "path": ".", "limit": 2 }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert!(result.output.contains("当前展示前 2 个"));
}

#[test]
fn list_files_rejects_path_traversal() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_LIST_FILES.to_string(),
        arguments: json!({ "path": "../../../etc" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    // PA-080 统一路径判定：逃逸工作区的路径先命中 path_permission 边界 →
    // `requires_authorization`（宿主审批语义），不再走 canonicalize 失败的 `invalid_path`。
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("requires_authorization")
    );
}

#[test]
fn list_files_rejects_nonexistent_directory() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_LIST_FILES.to_string(),
        arguments: json!({ "path": "nonexistent-dir" }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload: Value =
        serde_json::from_str(&result.output).expect("error output should be json");
    // canonicalize fails first → invalid_path (not read_failed;
    // read_failed would require an existing directory with denied
    // read permission, which is platform-specific to test)
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("invalid_path")
    );
}

/// Windows verbatim 前缀归一：**纯字符串逻辑**，因此可在 Linux 上镜像验证；
/// 同时锁定「无前缀 ⇒ 恒等（Linux 行为零变化）」。
#[test]
fn normalize_if_verbatim_is_identity_without_ws_prefix_and_strips_it_with() {
    // 1) 无前缀：恒等（Linux/macOS 上 canonicalize 永不产生 verbatim 前缀 ⇒ 零变化）。
    for plain in ["/tmp/a/b", "/Users/x/AGENTS.md", "C:\\Users\\x\\AGENTS.md"] {
        let path = Path::new(plain);
        assert_eq!(
            normalize_if_verbatim(path),
            path.to_path_buf(),
            "无 `\\\\?\\` 前缀的路径必须原样返回（不得走 display() 归一）: {plain}"
        );
    }
    // 2) 归一判定只按字节前缀（不调宿主 is_absolute）：`\\?\\` 起即按 verbatim 处理。
    //    生产侧两处调用点传入的都是绝对路径（root / canonicalize 结果），Linux 绝对路径必以
    //    `/` 起，故该分支在 Linux 上不可达 ⇒ 行为零变化。
    assert_eq!(
        normalize_if_verbatim(Path::new("\\\\?\\odd-name")),
        PathBuf::from("odd-name")
    );
    // 3) Windows verbatim 形态（纯字符串可判定，故 Linux 上也能断言）：
    assert_eq!(
        normalize_if_verbatim(Path::new("\\\\?\\C:\\ws\\AGENTS.md")),
        PathBuf::from("C:\\ws\\AGENTS.md"),
        "`\\\\?\\C:\\...` 必须剥成 `C:\\...`（否则 Prefix(VerbatimDisk) ≠ Prefix(Disk)）"
    );
    assert_eq!(
        normalize_if_verbatim(Path::new("\\\\?\\UNC\\server\\share\\x")),
        PathBuf::from("\\\\server\\share\\x"),
        "`\\\\?\\UNC\\...` 必须剥成 `\\\\server\\share\\...`"
    );
    // 4) 契约级检查（纯字符串可判定）：归一后的形态必须与 plain root **字面同形**，
    //    Windows 上 `Path::strip_prefix(plain_root)` 才能成功 → 搜索结果才会是
    //    `src/lib.rs` 这类 workspace 相对路径，而不是 `//?/C:/...` 绝对怪形态。
    let plain_root = r"C:\ws";
    let normalized = normalize_if_verbatim(Path::new(r"\\?\C:\ws\src\lib.rs"));
    assert!(
        normalized.to_string_lossy().starts_with(plain_root),
        "归一结果必须与 plain root 字面同形（否则 Windows 上 strip_prefix 失配）: {normalized:?}"
    );
}

pub use pony_protocol::workspace_types::*;

#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_permission::normalize_canonical;

    fn unique_root(tag: &str) -> String {
        let raw = std::env::temp_dir()
            .join(format!("pa079-ws-{tag}-{}", std::process::id()));
        normalize_canonical(&raw).display().to_string()
    }

    #[test]
    fn create_and_resolve_workspace() {
        let root = unique_root("create");
        std::fs::create_dir_all(&root).unwrap();

        let mut records = Vec::new();
        let record = create_workspace_entry(&mut records, "My Project", &root).unwrap();
        assert!(record.id.starts_with("ws-my-project-"));
        assert_eq!(record.root_path, normalize_workspace_root(&root).unwrap());
        assert_eq!(
            resolve_workspace_root(&records, Some(&record.id)).unwrap(),
            normalize_workspace_root(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_duplicate_root_and_non_directory() {
        let root = unique_root("dup");
        std::fs::create_dir_all(&root).unwrap();

        let mut records = Vec::new();
        create_workspace_entry(&mut records, "A", &root).unwrap();
        let err = create_workspace_entry(&mut records, "B", &root).unwrap_err();
        assert!(err.contains("重复 root"));

        // 非目录 root
        let file_root = unique_root("file");
        std::fs::write(&file_root, b"file").unwrap();
        let err = create_workspace_entry(&mut records, "C", &file_root).unwrap_err();
        assert!(err.contains("不是目录"));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&file_root);
    }

    #[test]
    fn missing_root_is_rejected() {
        let root = unique_root("missing");
        let _ = std::fs::remove_dir_all(&root);
        let mut records = Vec::new();
        let err = create_workspace_entry(&mut records, "X", &root).unwrap_err();
        assert!(err.contains("不可解析"));
    }

    #[test]
    fn default_workspace_resolution_and_id_validation() {
        let mut records = Vec::new();
        assert!(!default_workspace_exists(&records));
        records.push(WorkspaceRecord {
            id: DEFAULT_WORKSPACE_ID.to_string(),
            name: "默认工作区".to_string(),
            root_path: "C:\\ws".to_string(),
        });
        assert!(default_workspace_exists(&records));
        assert_eq!(resolve_workspace_root(&records, None).unwrap(), "C:\\ws");
        assert_eq!(validate_workspace_id("default").unwrap(), "default");
        assert!(validate_workspace_id("ws-proj").is_ok());
        assert!(validate_workspace_id("bad/id").is_err());
        assert_eq!(
            resolve_workspace_root(&records, Some("nope")).unwrap_err(),
            "workspace 不存在：nope"
        );
    }

    #[test]
    fn relative_root_is_canonicalized_to_absolute() {
        // P2-7a：相对路径 create → canonical 绝对路径存储。
        let mut records = Vec::new();
        let record = create_workspace_entry(&mut records, "Rel", ".").unwrap();
        // 期望 = normalize_workspace_root(".")（与 create 同一规范化，含 Windows \\?\ 前缀剥离）
        let expected = normalize_workspace_root(".").unwrap();
        assert_eq!(record.root_path, expected);
        assert!(record.root_path.len() > 2);
    }

    #[test]
    fn roots_match_is_case_aware_per_platform() {
        // Windows：大小写不敏感判重；Unix：大小写敏感。
        assert!(roots_match("C:\\WS", "c:\\ws") == cfg!(windows));
        assert!(roots_match("/a/b", "/a/b"));
        assert_eq!(roots_match("/a/b", "/a/B"), cfg!(windows));
    }

    #[test]
    fn win_prefix_normalization() {
        assert_eq!(normalize_win_prefix(r"\\?\C:\ws"), r"C:\ws");
        assert_eq!(normalize_win_prefix(r"\\?\UNC\server\share"), r"\\server\share");
        assert_eq!(normalize_win_prefix(r"C:\ws"), r"C:\ws");
    }

    // ── 侧边栏三级树（rename/delete/命名校验对称）─────────────────────────

    #[test]
    fn rename_updates_name_and_keeps_id_root() {
        let root = unique_root("rename");
        std::fs::create_dir_all(&root).unwrap();
        let mut records = Vec::new();
        let created = create_workspace_entry(&mut records, "My Project", &root).unwrap();

        let renamed = rename_workspace_entry(&mut records, &created.id, "Renamed Project").unwrap();
        assert_eq!(renamed.name, "Renamed Project");
        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.root_path, created.root_path);
        assert_eq!(records[0].name, "Renamed Project");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_validates_blank_overlong_unknown() {
        let mut records = Vec::new();
        let err_blank = rename_workspace_entry(&mut records, "ws-x", "   ");
        assert!(err_blank.unwrap_err().contains("名称不能为空"));

        let overlong = "长".repeat(65);
        let err_long = rename_workspace_entry(&mut records, "ws-x", &overlong);
        assert!(err_long.unwrap_err().contains("64"));

        let err_unknown = rename_workspace_entry(&mut records, "ws-missing", "Any");
        assert!(err_unknown.unwrap_err().contains("workspace 不存在"));
    }

    #[test]
    fn rename_rejects_duplicate_but_allows_self_name() {
        let root_a = unique_root("dupname-a");
        let root_b = unique_root("dupname-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let mut records = Vec::new();
        create_workspace_entry(&mut records, "Alpha", &root_a).unwrap();
        let b = create_workspace_entry(&mut records, "Beta", &root_b).unwrap();

        let err = rename_workspace_entry(&mut records, &b.id, "Alpha").unwrap_err();
        assert!(err.contains("已存在同名工作区"));

        // 改名回自身当前名：排除自身后无冲突 → 成功 no-op。
        let same = rename_workspace_entry(&mut records, &b.id, "Beta").unwrap();
        assert_eq!(same.name, "Beta");

        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[test]
    fn create_rejects_duplicate_name_and_overlong() {
        let root_a = unique_root("createdup-a");
        let root_b = unique_root("createdup-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let mut records = Vec::new();
        create_workspace_entry(&mut records, "Same", &root_a).unwrap();
        let err = create_workspace_entry(&mut records, "  Same  ", &root_b).unwrap_err();
        assert!(err.contains("已存在同名工作区"));

        let overlong = "x".repeat(65);
        let err_long = create_workspace_entry(&mut records, &overlong, &root_b).unwrap_err();
        assert!(err_long.contains("64"));

        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[test]
    fn delete_rejects_default_and_unknown_then_removes_target() {        let root = unique_root("delete");
        std::fs::create_dir_all(&root).unwrap();
        let mut records = Vec::new();
        records.push(WorkspaceRecord {
            id: DEFAULT_WORKSPACE_ID.to_string(),
            name: "默认工作区".to_string(),
            root_path: "C:\\ws".to_string(),
        });
        let target = create_workspace_entry(&mut records, "Gone", &root).unwrap();

        assert!(delete_workspace_entry(&mut records, DEFAULT_WORKSPACE_ID)
            .unwrap_err()
            .contains("不可删除"));
        assert!(delete_workspace_entry(&mut records, "ws-nope")
            .unwrap_err()
            .contains("不存在"));

        delete_workspace_entry(&mut records, &target.id).unwrap();
        assert!(!records.iter().any(|record| record.id == target.id));
        assert!(resolve_workspace_root(&records, Some(&target.id)).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bootstrap_creates_documents_pony_agent_dir_and_record() {
        let base = std::path::PathBuf::from(unique_root("bootstrap"));
        let mut records = Vec::new();

        assert!(bootstrap_default_workspace_with_base(
            &mut records,
            Some(base.clone())
        ));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, DEFAULT_WORKSPACE_ID);
        assert_eq!(records[0].name, "默认工作区");
        assert!(
            base.join("pony_agent").is_dir(),
            "根目录应被自动创建；record={:?}",
            records[0]
        );
        let expected =
            normalize_workspace_root(&base.join("pony_agent").display().to_string()).unwrap();
        assert_eq!(records[0].root_path, expected);

        // 幂等：已有 default 时不重复登记、不改动。
        let before = records.clone();
        assert!(!bootstrap_default_workspace_with_base(
            &mut records,
            Some(base.clone())
        ));
        assert_eq!(records, before);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_default_workspace_base_matrix() {
        let doc = Path::new("/custom/documents");
        let home = Path::new("/custom/home");

        // Windows = true
        // 1. document_dir Some
        let res = resolve_default_workspace_base(Some(doc), Some(home), true, &|_| false);
        assert_eq!(res, Some(doc.to_path_buf()));

        // 2. document_dir None, home Some, home/Documents exists
        let res = resolve_default_workspace_base(None, Some(home), true, &|p| {
            p == home.join("Documents")
        });
        assert_eq!(res, Some(home.join("Documents")));

        // 3. document_dir None, home Some, home/Documents does NOT exist
        let res = resolve_default_workspace_base(None, Some(home), true, &|_| false);
        assert_eq!(res, Some(home.to_path_buf()));

        // 4. all None
        let res = resolve_default_workspace_base(None, None, true, &|_| false);
        assert_eq!(res, None);

        // Windows = false (Unix)
        // 1. home Some
        let res = resolve_default_workspace_base(Some(doc), Some(home), false, &|_| true);
        assert_eq!(res, Some(home.to_path_buf()));

        // 2. home None
        let res = resolve_default_workspace_base(Some(doc), None, false, &|_| true);
        assert_eq!(res, None);
    }

    #[test]
    fn bootstrap_inner_refuses_registration_when_base_unresolvable() {
        let mut records = Vec::new();
        // R2-2: base None -> false, no record
        assert!(!bootstrap_default_workspace_inner(&mut records, None));
        assert!(records.is_empty());
    }

    #[test]
    fn workspace_concurrent_immutable_context_isolation_test() {
        use crate::tools::{ToolCall, ToolExecutionContext, ToolRouter};
        use serde_json::json;
        use std::sync::Arc;

        let num_workspaces = 4;
        let num_threads = 16;
        let mut ws_roots = Vec::new();

        for i in 0..num_workspaces {
            let root = std::path::PathBuf::from(unique_root(&format!("concur_ws_{i}")));
            std::fs::create_dir_all(&root).unwrap();
            let canonical_root = root.canonicalize().unwrap_or(root);
            std::fs::write(canonical_root.join("identity.txt"), format!("ws_{i}_identity")).unwrap();
            ws_roots.push(canonical_root);
        }

        let router = Arc::new(ToolRouter::new());
        let mut handles = Vec::new();

        for thread_idx in 0..num_threads {
            let ws_idx = thread_idx % num_workspaces;
            let ws_root = ws_roots[ws_idx].clone();
            let router_clone = router.clone();

            let handle = std::thread::spawn(move || {
                let context = ToolExecutionContext {
                    workspace_root: Some(ws_root.clone()),
                    ..Default::default()
                };

                // 1. Read identity
                let read_call = ToolCall {
                    call_id: Some(format!("call-read-{thread_idx}")),
                    name: "workspace_read_file".to_string(),
                    arguments: json!({ "path": "identity.txt" }),
                    plan: None,
                };
                let read_res = router_clone.execute_with_context(&read_call, &context);
                assert_eq!(read_res.status, "ok", "Thread {thread_idx} read failed: {}", read_res.output);
                assert!(read_res.output.contains(&format!("ws_{ws_idx}_identity")), "Thread {thread_idx} got corrupted identity: {}", read_res.output);

                // 2. Write file
                let write_call = ToolCall {
                    call_id: Some(format!("call-write-{thread_idx}")),
                    name: "workspace_write_file".to_string(),
                    arguments: json!({
                        "path": format!("thread_{thread_idx}.txt"),
                        "content": format!("payload_{thread_idx}"),
                        "overwrite": true
                    }),
                    plan: None,
                };
                let write_res = router_clone.execute_with_context(&write_call, &context);
                assert_eq!(write_res.status, "ok", "Thread {thread_idx} write failed: {}", write_res.output);

                // 3. Verify file exists in this workspace and not others
                let written_file = ws_root.join(format!("thread_{thread_idx}.txt"));
                assert!(written_file.exists(), "Thread {thread_idx} file not in correct workspace");
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        for root in ws_roots {
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn workspace_unknown_id_fail_closed_rejection_test() {
        use crate::tools::{ToolCall, ToolExecutionContext, ToolRouter};
        use serde_json::json;

        let base_root = std::path::PathBuf::from(unique_root("unknown_ws_test"));
        std::fs::create_dir_all(&base_root).unwrap();

        // Create router with a resolver that knows only "ws-valid"
        let valid_root = base_root.join("valid");
        std::fs::create_dir_all(&valid_root).unwrap();

        let resolver_root = valid_root.clone();
        let router = ToolRouter::new().with_root_resolver(move |ws_id: &str| {
            if ws_id == "ws-valid" {
                Some(resolver_root.clone())
            } else {
                None
            }
        });

        // Attempting to read using an unknown workspace_id must FAIL-CLOSED
        let call = ToolCall {
            call_id: Some("call-unknown".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({
                "path": "secret.txt",
                "workspaceId": "ws-unknown-hacker"
            }),
            plan: None,
        };
        let res = router.execute_with_context(&call, &ToolExecutionContext::default());
        assert_eq!(res.status, "error", "Unknown workspace must fail closed: {}", res.output);
        assert!(res.output.contains("未注册") || res.output.contains("不存在") || res.output.contains("invalid_workspace"), "Output: {}", res.output);

        let _ = std::fs::remove_dir_all(&base_root);
    }

    #[test]
    fn workspace_relative_path_non_default_resolution_test() {
        use crate::tools::{ToolCall, ToolExecutionContext, ToolRouter};
        use serde_json::json;

        let base_root = std::path::PathBuf::from(unique_root("rel_path_test"));
        std::fs::create_dir_all(&base_root).unwrap();

        let default_dir = base_root.join("default_dir");
        let non_default_dir = base_root.join("non_default_dir");
        std::fs::create_dir_all(default_dir.join("src")).unwrap();
        std::fs::create_dir_all(non_default_dir.join("src")).unwrap();

        std::fs::write(default_dir.join("src/app.rs"), "WS_DEFAULT_UNIQUE_PAYLOAD_1234").unwrap();
        std::fs::write(non_default_dir.join("src/app.rs"), "WS_NON_DEFAULT_UNIQUE_PAYLOAD_5678").unwrap();

        let router = ToolRouter::new();

        // Reading relative path "src/app.rs" under non-default context
        let context = ToolExecutionContext {
            workspace_root: Some(non_default_dir.clone()),
            ..Default::default()
        };
        let call = ToolCall {
            call_id: Some("call-rel".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({ "path": "src/app.rs" }),
            plan: None,
        };
        let res = router.execute_with_context(&call, &context);
        assert_eq!(res.status, "ok", "Read failed: {}", res.output);
        assert!(res.output.contains("WS_NON_DEFAULT_UNIQUE_PAYLOAD_5678"), "Corrupted content: {}", res.output);
        assert!(!res.output.contains("WS_DEFAULT_UNIQUE_PAYLOAD_1234"), "Leaked default workspace content: {}", res.output);

        let _ = std::fs::remove_dir_all(&base_root);
    }

    #[test]
    fn workspace_cross_boundary_path_attack_test() {
        use crate::tools::{ToolCall, ToolExecutionContext, ToolRouter};
        use serde_json::json;

        let base_root = std::path::PathBuf::from(unique_root("attack_test"));
        std::fs::create_dir_all(&base_root).unwrap();

        let ws_a = base_root.join("ws_a");
        let ws_b = base_root.join("ws_b");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        std::fs::write(ws_b.join("secret.txt"), "TOP_SECRET_B").unwrap();

        let router = ToolRouter::new();
        let context_a = ToolExecutionContext {
            workspace_root: Some(ws_a.clone()),
            ..Default::default()
        };

        // 1. Directory traversal attempt
        let traversal_call = ToolCall {
            call_id: Some("call-traversal".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({ "path": "../ws_b/secret.txt" }),
            plan: None,
        };
        let res = router.execute_with_context(&traversal_call, &context_a);
        assert_eq!(res.status, "error", "Traversal must fail: {}", res.output);

        // 2. Write to outside workspace
        let write_escape_call = ToolCall {
            call_id: Some("call-write-escape".to_string()),
            name: "workspace_write_file".to_string(),
            arguments: json!({
                "path": "../ws_b/hacked.txt",
                "content": "pwned",
                "overwrite": true
            }),
            plan: None,
        };
        let res_write = router.execute_with_context(&write_escape_call, &context_a);
        assert_eq!(res_write.status, "error", "Write escape must fail: {}", res_write.output);
        assert!(!ws_b.join("hacked.txt").exists(), "Escape file was written to disk!");

        let _ = std::fs::remove_dir_all(&base_root);
    }

    // ── PA-114 workspace-cwd 第二轮（fix-round2-workspace-cwd）红相验收 ─────────
    // 契约：`.dev-team/fix-round2-workspace-cwd.md` §3 F1/F2/F6。
    // 红相说明：`normalize_session_workspace_id`（W1）与 `default_root_or_cwd_with_warning`
    // （W2 seam）当前均不存在 → 本模块编译失败即红相证据（Lead 认可编译失败作为红相）。

    /// F1：`normalize_session_workspace_id` 纯函数矩阵（W1 契约）——default 原样、
    /// 已注册原样、死 id/空白/控制字符/超长 → default，且不依赖注册表解析 default 保留字。
    #[test]
    fn f1_normalize_session_workspace_id_matrix() {
        let mut records = Vec::new();
        records.push(WorkspaceRecord {
            id: DEFAULT_WORKSPACE_ID.to_string(),
            name: "默认工作区".to_string(),
            root_path: "/canonical/default".to_string(),
        });
        records.push(WorkspaceRecord {
            id: "ws-custom".to_string(),
            name: "Custom".to_string(),
            root_path: "/canonical/custom".to_string(),
        });

        // default id 原样。
        assert_eq!(
            normalize_session_workspace_id(&records, DEFAULT_WORKSPACE_ID),
            DEFAULT_WORKSPACE_ID
        );
        // 已注册 id 原样。
        assert_eq!(
            normalize_session_workspace_id(&records, "ws-custom"),
            "ws-custom"
        );
        // 死 id → default。
        assert_eq!(
            normalize_session_workspace_id(&records, "ws-deleted-long-ago"),
            DEFAULT_WORKSPACE_ID
        );
        // 空白 id → default。
        assert_eq!(
            normalize_session_workspace_id(&records, "   "),
            DEFAULT_WORKSPACE_ID
        );
        assert_eq!(
            normalize_session_workspace_id(&records, ""),
            DEFAULT_WORKSPACE_ID
        );
        // 控制字符 id → default（W1 日志口径：控制字符压平、80 字符截断）。
        assert_eq!(
            normalize_session_workspace_id(&records, "ws-\n\t-1"),
            DEFAULT_WORKSPACE_ID
        );
        // 超长 id → default。
        let long_id = format!("ws-{}", "x".repeat(300));
        assert_eq!(
            normalize_session_workspace_id(&records, &long_id),
            DEFAULT_WORKSPACE_ID
        );
        // 空注册表下 default 保留字仍原样（不依赖注册表）。
        assert_eq!(
            normalize_session_workspace_id(&[], DEFAULT_WORKSPACE_ID),
            DEFAULT_WORKSPACE_ID
        );
    }

    /// F2（session 侧，见 session/tests.rs `f2_session_cwd_agrees_with_normalized_id_resolution`）：
    /// 跨函数一致性由 session/tests.rs 持有（`session_workspace_cwd` 在 session 模块内可见）。

    /// F6：ToolRouter::new 兜底告警 seam（W2 契约，可注入判定）。
    /// `computed=None` → 与 control_plane 对齐的瞬时兜底告警 + 进程 cwd 兜底；
    /// `computed=Some` → computed 优先。红相：seam 不存在 → 编译失败。
    #[test]
    fn f6_tool_router_fallback_seam_warns_and_uses_cwd() {
        let fallback_cwd = PathBuf::from("/fallback/cwd");
        let fallback = default_root_or_cwd_with_warning(None, || fallback_cwd.clone());
        assert_eq!(fallback, fallback_cwd, "computed=None must fall back to cwd");
        let computed = PathBuf::from("/computed/pony_agent");
        let resolved =
            default_root_or_cwd_with_warning(Some(computed.clone()), || fallback_cwd.clone());
        assert_eq!(resolved, computed, "computed=Some must win over the cwd fallback");
    }
}

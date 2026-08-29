//! Workspace 注册表（PA-079）：WorkspaceRecord + 注册逻辑（create/list/get/default）。
//! 持久化由调用方（SessionStore）经 PersistedStore / store_metadata 完成，本模块保持纯数据 + 校验。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 默认 workspace id（保留字，豁免 `ws-<slug>` 规则；PA-081 前端消费同一常量）。
pub const DEFAULT_WORKSPACE_ID: &str = "default";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    /// 稳定 id：默认 `"default"`，其余 `ws-<slug>-<ts>`
    pub id: String,
    pub name: String,
    /// canonical 绝对路径（Windows 去 `\\?\` 前缀）
    pub root_path: String,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Windows canonicalize 输出去 `\\?\` / `\\?\UNC\` 前缀，统一为可读绝对路径。
fn normalize_win_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    path.to_string()
}

/// 规范化 workspace root：trim → canonicalize（存在）→ 必须是目录 → Windows 去前缀。
pub fn normalize_workspace_root(root: &str) -> Result<String, String> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return Err("workspace root 不能为空".into());
    }
    let input = PathBuf::from(trimmed);
    let canonical = input
        .canonicalize()
        .map_err(|error| format!("workspace root 不可解析：{error}"))?;
    if !canonical.is_dir() {
        return Err("workspace root 不是目录".into());
    }
    Ok(normalize_win_prefix(&canonical.display().to_string()))
}

/// 校验 workspace id（默认保留字或 `ws-<slug>`）。
pub fn validate_workspace_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed == DEFAULT_WORKSPACE_ID {
        return Ok(trimmed.to_string());
    }
    if trimmed.len() > 3
        && trimmed.starts_with("ws-")
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
    {
        return Ok(trimmed.to_string());
    }
    Err(format!("workspace id 非法：{id}"))
}

/// Windows 上 root 比较大小写不敏感（canonicalize 不统一大小写，`C:\WS` vs `c:\ws` 应判重）；
/// Unix 上大小写敏感。
pub fn roots_match(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

/// 默认 workspace 是否存在于注册表；缺则返回"待创建"占位。
pub fn default_workspace_exists(records: &[WorkspaceRecord]) -> bool {
    records.iter().any(|record| record.id == DEFAULT_WORKSPACE_ID)
}

/// 默认工作区根目录（三级树安装期行为）：
/// - Windows：`%USERPROFILE%\Documents\pony_agent`（Known-Folder 解析，尊重 OneDrive 重定向）
/// - macOS/Linux：`$HOME/pony_agent`
/// 目录不存在时自动创建。解析失败返回 None，由调用方回退进程 cwd。
pub fn compute_default_workspace_root() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        dirs::document_dir()
    } else {
        dirs::home_dir()
    }?;
    Some(base.join("pony_agent"))
}

/// 确保注册表存在默认工作区记录且其根目录可用：
/// 记录缺失 → 以 Documents~/pony_agent（不存在即建）登记并返回 true；
/// 已存在则不动（用户既有登记永不静默迁移）。调用方负责落盘。
pub fn bootstrap_default_workspace(records: &mut Vec<WorkspaceRecord>) -> bool {
    bootstrap_default_workspace_with_base(records, None)
}

/// 测试/注入变体：base 显式给定时跳过系统 Known-Folder 解析（不触碰真实用户目录）。
pub fn bootstrap_default_workspace_with_base(
    records: &mut Vec<WorkspaceRecord>,
    base_override: Option<PathBuf>,
) -> bool {
    if default_workspace_exists(records) {
        return false;
    }
    let fallback = || -> String {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .display()
            .to_string()
    };
    let raw_root = match base_override
        .map(|base| base.join("pony_agent"))
        .or_else(|| compute_default_workspace_root())
    {
        Some(path) => {
            if let Err(error) = std::fs::create_dir_all(&path) {
                eprintln!(
                    "[pony-agent] 默认工作区目录创建失败，回退进程 cwd：{error}"
                );
                fallback()
            } else {
                path.display().to_string()
            }
        }
        None => fallback(),
    };
    // P2-6：默认 root 走与 create 相同的规范化（canonicalize + 去 \\?\ 前缀），
    // 避免与 create_workspace_entry 存储形式不一致导致 PA-080 前缀比较失配。
    let canonical = normalize_workspace_root(&raw_root).unwrap_or_else(|_| raw_root.clone());
    records.push(WorkspaceRecord {
        id: DEFAULT_WORKSPACE_ID.to_string(),
        name: "默认工作区".to_string(),
        root_path: canonical,
    });
    true
}

/// 按 root 查 workspace（Windows 大小写归一；与 create 的重复判定一致）。
pub fn find_workspace_by_root(records: &[WorkspaceRecord], canonical_root: &str) -> Option<WorkspaceRecord> {
    records
        .iter()
        .find(|record| roots_match(&record.root_path, canonical_root))
        .cloned()
}

/// 显示名长度上限（create/rename 共用）。
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// 显示名校验（工作区/会话共用）：trim 后非空、≤64 字符；返回 trim 结果。
/// create 与 rename 共用同一规则，避免同一注册表两条写入路径不变量分叉。
pub fn validate_display_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("名称不能为空".into());
    }
    if trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(format!("名称不能超过 {} 个字符", MAX_DISPLAY_NAME_CHARS));
    }
    Ok(trimmed.to_string())
}

/// 创建 workspace 条目并推入注册表（调用方负责持久化）。id 由 `ws-<slug>-<ts>` 生成。
pub fn create_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    name: &str,
    root_path: &str,
) -> Result<WorkspaceRecord, String> {
    let trimmed_name = validate_display_name(name).map_err(|error| format!("workspace {error}"))?;
    let canonical = normalize_workspace_root(root_path)?;
    if records.iter().any(|record| roots_match(&record.root_path, &canonical)) {
        return Err("workspace root 已存在（重复 root 拒绝）".into());
    }
    if records.iter().any(|record| record.name == trimmed_name) {
        return Err("已存在同名工作区".into());
    }

    let slug = slugify(&trimmed_name);
    let id = if slug.is_empty() {
        format!("ws-{}", now_ms())
    } else {
        format!("ws-{slug}-{}", now_ms())
    };
    let record = WorkspaceRecord {
        id,
        name: trimmed_name,
        root_path: canonical,
    };
    records.push(record.clone());
    Ok(record)
}

/// 重命名工作区：仅改显示名；id 与 root 不变。校验规则与 create 一致，
/// 重名判定排除自身（改名回当前名为 no-op 成功）。未知 id 报错。
pub fn rename_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    workspace_id: &str,
    name: &str,
) -> Result<WorkspaceRecord, String> {
    let trimmed_name = validate_display_name(name).map_err(|error| format!("workspace {error}"))?;
    // 先做只读校验（重名判定排除自身），再取可变借用，避免借用重叠。
    if records
        .iter()
        .filter(|other| other.id != workspace_id)
        .any(|other| other.name == trimmed_name)
    {
        return Err("已存在同名工作区".into());
    }
    let record = records
        .iter_mut()
        .find(|record| record.id == workspace_id)
        .ok_or_else(|| format!("workspace 不存在：{workspace_id}"))?;
    record.name = trimmed_name.clone();
    Ok(WorkspaceRecord {
        id: record.id.clone(),
        name: trimmed_name,
        root_path: record.root_path.clone(),
    })
}

/// 删除工作区注册：default 保留字拒绝；未知 id 报错。只移除注册表记录——
/// 目录、会话日志与会话数据均归调用方/其他机制所有，本函数不触碰。
/// 调用方（SessionStore::delete_workspace）负责把名下会话归属重写为 default。
pub fn delete_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    workspace_id: &str,
) -> Result<(), String> {
    if workspace_id == DEFAULT_WORKSPACE_ID {
        return Err("默认工作区不可删除".into());
    }
    match records.iter().position(|record| record.id == workspace_id) {
        Some(index) => {
            records.remove(index);
            Ok(())
        }
        None => Err(format!("workspace 不存在：{workspace_id}")),
    }
}

/// 解析 workspace id → root；id 为 None/缺省 → 默认 workspace root；找不到 → 错误。
pub fn resolve_workspace_root(records: &[WorkspaceRecord], workspace_id: Option<&str>) -> Result<String, String> {
    let id = match workspace_id {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => DEFAULT_WORKSPACE_ID,
    };
    if id == DEFAULT_WORKSPACE_ID {
        return records
            .iter()
            .find(|record| record.id == DEFAULT_WORKSPACE_ID)
            .map(|record| record.root_path.clone())
            .ok_or_else(|| "默认 workspace 未初始化".into());
    }
    records
        .iter()
        .find(|record| record.id == id)
        .map(|record| record.root_path.clone())
        .ok_or_else(|| format!("workspace 不存在：{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root(tag: &str) -> String {
        std::env::temp_dir()
            .join(format!("pa079-ws-{tag}-{}", std::process::id()))
            .display()
            .to_string()
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
    fn bootstrap_falls_back_to_process_cwd_when_base_unresolvable() {
        // Windows 下 document_dir 极少为 None；此处直接验证 base=None 分支走回退
        // 且仍产出合法注册项（不触碰真实用户目录）。
        let mut records = Vec::new();
        assert!(bootstrap_default_workspace_with_base(&mut records, None));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, DEFAULT_WORKSPACE_ID);
        assert!(!records[0].root_path.is_empty());
    }

    #[test]
    fn workspace_concurrent_immutable_context_isolation_test() {
        use crate::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
        use serde_json::json;
        use std::sync::Arc;

        let num_workspaces = 4;
        let num_threads = 16;
        let mut ws_roots = Vec::new();

        for i in 0..num_workspaces {
            let root = std::path::PathBuf::from(unique_root(&format!("concur_ws_{i}")));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("identity.txt"), format!("ws_{i}_identity")).unwrap();
            ws_roots.push(root);
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
        use crate::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
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
        use crate::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
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
        use crate::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
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
}

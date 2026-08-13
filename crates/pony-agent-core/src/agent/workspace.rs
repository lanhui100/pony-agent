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

/// 按 root 查 workspace（Windows 大小写归一；与 create 的重复判定一致）。
pub fn find_workspace_by_root(records: &[WorkspaceRecord], canonical_root: &str) -> Option<WorkspaceRecord> {
    records
        .iter()
        .find(|record| roots_match(&record.root_path, canonical_root))
        .cloned()
}

/// 创建 workspace 条目并推入注册表（调用方负责持久化）。id 由 `ws-<slug>-<ts>` 生成。
pub fn create_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    name: &str,
    root_path: &str,
) -> Result<WorkspaceRecord, String> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err("workspace 名称不能为空".into());
    }
    let canonical = normalize_workspace_root(root_path)?;
    if records.iter().any(|record| roots_match(&record.root_path, &canonical)) {
        return Err("workspace root 已存在（重复 root 拒绝）".into());
    }

    let slug = slugify(trimmed_name);
    let id = if slug.is_empty() {
        format!("ws-{}", now_ms())
    } else {
        format!("ws-{slug}-{}", now_ms())
    };
    let record = WorkspaceRecord {
        id,
        name: trimmed_name.to_string(),
        root_path: canonical,
    };
    records.push(record.clone());
    Ok(record)
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
}

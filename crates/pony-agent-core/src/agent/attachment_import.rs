//! Attachment import (PA-078)：把对话附件字节写入受控导入目录，供消息附件引用与
//! `workspace_read_document` 读取。纯路径/文件逻辑独立于此模块以便单测；control plane
//! 只负责 base64 解码与委托。
//!
//! 布局契约（与 spec.md / PA-080 受控 tmp 逐字一致）：
//! - 主导入目录：`<workspace_root>/.tmp/imports/`
//! - fallback：`<temp_dir>/pony-agent/.tmp/imports/`（稳定非 PID，重启不换路径）

use std::path::{Path, PathBuf};

/// Fallback 受控临时根目录（稳定非 PID；与 PA-080 ControlledTmp 布局一致）。
pub const FALLBACK_TEMP_SUBDIR: &str = "pony-agent";

/// 导入目录相对 workspace root 的固定路径。
pub const IMPORT_RELATIVE_DIR: &str = ".tmp/imports";

/// 导入结果：目标文件的 canonical 绝对路径 + 相对 workspace root 的引用路径。
#[derive(Clone, Debug)]
pub struct ImportAttachmentResult {
    pub path: PathBuf,
    /// 相对 workspace root 的引用路径（`.tmp/imports/<name>`）；写入 fallback 目录时为 None
    /// （该路径不在 workspace 命名空间内，前端不应构造假相对路径）。
    pub relative_path: Option<String>,
}

/// 校验附件文件名：非空、无路径分隔符、无 `.`/`..`、无 NUL。宿主侧信任边界。
pub fn validate_attachment_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("[import_attachment_invalid_name] 文件名不能为空".into());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("[import_attachment_invalid_name] 文件名包含非法字符".into());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        return Err("[import_attachment_invalid_name] 文件名包含非法字符".into());
    }
    Ok(())
}

/// 解析导入目录：优先 `<workspace_root>/.tmp/imports/`；创建失败（不可写）回退
/// `<temp_dir>/pony-agent/.tmp/imports/`。
pub fn resolve_import_dir(workspace_root: &Path) -> Result<PathBuf, String> {
    let primary = workspace_root.join(IMPORT_RELATIVE_DIR);
    if std::fs::create_dir_all(&primary).is_ok() {
        return Ok(primary);
    }
    let fallback = std::env::temp_dir()
        .join(FALLBACK_TEMP_SUBDIR)
        .join(IMPORT_RELATIVE_DIR);
    std::fs::create_dir_all(&fallback)
        .map_err(|error| format!("[import_attachment_failed] 无法创建导入目录：{error}"))?;
    Ok(fallback)
}

/// 把导入字节写入受控导入目录并返回目标路径。`name` 先净化；目标 = 导入目录 join name，
/// 再做一次组件级前缀校验（防御性，name 单组件时恒真）。
pub fn import_attachment_bytes(
    name: &str,
    bytes: &[u8],
    workspace_root: &Path,
) -> Result<ImportAttachmentResult, String> {
    // 校验与存储都用 trim 后的名字（I-11：尾随空格/点不进入文件名）
    let name = name.trim();
    validate_attachment_name(name)?;
    if bytes.is_empty() {
        return Err("[import_attachment_failed] 附件内容为空".into());
    }

    let import_dir = resolve_import_dir(workspace_root)?;
    let target = import_dir.join(name);
    if !target.starts_with(&import_dir) {
        return Err("[import_attachment_failed] 目标路径逃逸导入目录".into());
    }

    std::fs::write(&target, bytes)
        .map_err(|error| format!("[import_attachment_failed] 写入附件失败：{error}"))?;

    // primary 目录在 workspace root 下 → 相对路径有效；fallback 目录（temp）→ None。
    let relative_path = if import_dir.starts_with(workspace_root) {
        Some(format!("{IMPORT_RELATIVE_DIR}/{name}"))
    } else {
        None
    };
    Ok(ImportAttachmentResult {
        path: target,
        relative_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 每个测试用唯一临时子路径，避免并行执行时相互 remove_dir_all 竞态。
    fn unique_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pa078-{tag}-{}", std::process::id()))
    }

    #[test]
    fn validate_name_rejects_separators_and_traversal() {
        assert!(validate_attachment_name("report.pdf").is_ok());
        assert!(validate_attachment_name("a b.png").is_ok());
        assert!(validate_attachment_name("").is_err());
        assert!(validate_attachment_name("..").is_err());
        assert!(validate_attachment_name("a/../b.txt").is_err());
        assert!(validate_attachment_name(r"a\..\b.txt").is_err());
        assert!(validate_attachment_name("a\0b.txt").is_err());
    }

    #[test]
    fn import_writes_into_workspace_import_dir() {
        let root = unique_root("write");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let result = import_attachment_bytes("hello.txt", b"hello world", &root).unwrap();
        assert!(result.path.starts_with(root.join(IMPORT_RELATIVE_DIR)));
        assert_eq!(std::fs::read_to_string(&result.path).unwrap(), "hello world");
        assert!(result.path.ends_with("hello.txt"));
        assert_eq!(
            result.relative_path,
            Some(format!("{IMPORT_RELATIVE_DIR}/hello.txt"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_rejects_traversal_name_before_any_write() {
        let root = unique_root("traversal");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let err = import_attachment_bytes("../escape.txt", b"x", &root).unwrap_err();
        assert!(err.contains("import_attachment_invalid_name"));
        assert!(!root.join("../escape.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_rejects_empty_bytes() {
        let root = unique_root("empty");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let err = import_attachment_bytes("empty.txt", b"", &root).unwrap_err();
        assert!(err.contains("import_attachment_failed"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_falls_back_to_temp_dir_with_null_relative_path_when_workspace_unwritable() {
        // 用一个"作为文件存在、无法在其下建目录"的路径模拟 workspace 不可写。
        let blocked = unique_root("blocked");
        std::fs::write(&blocked, b"file-not-dir").unwrap();

        let import_dir = resolve_import_dir(&blocked).unwrap();
        let expected_prefix = std::env::temp_dir().join(FALLBACK_TEMP_SUBDIR);
        assert!(import_dir.starts_with(expected_prefix));

        let result = import_attachment_bytes("fallback.txt", b"data", &blocked).unwrap();
        assert!(std::fs::read_to_string(&result.path).is_ok());
        // fallback 目录不在 workspace 命名空间内 → relative_path 必须为 None（T-1）
        assert_eq!(result.relative_path, None);

        let _ = std::fs::remove_file(&blocked);
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(FALLBACK_TEMP_SUBDIR));
    }
}

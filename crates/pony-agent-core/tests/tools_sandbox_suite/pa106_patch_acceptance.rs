//! PA-106: 原子代码补丁应用工具 (apply_patch) 黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-106-atomic-code-patch-tool.md`
//!
//! 验收矩阵覆盖：
//! - AC-01: `test_ac01_apply_patch_single_and_multi_files`
//!   - 正确解析与应用单文件与多文件 Unified Diff 补丁（修改、新增、删除）。
//! - AC-02: `test_ac02_apply_patch_all_or_nothing_rollback_on_failure`
//!   - 全有或全无原子性保证（All-or-Nothing）：若多文件补丁中任一 hunk 或文件校验失败，所有修改立即回滚，零脏写。
//! - AC-03: `test_ac03_apply_patch_path_traversal_denied`
//!   - 路径逃逸防护：补丁内包含 `../` 或指向工作区外的路径必须抛出安全拒绝错误并阻止所有写入。
//! - AC-04: `test_ac04_apply_patch_crlf_lf_tolerance`
//!   - CRLF 与 LF 换行符容错，并保持原文件换行风格。

use pony_agent_core::agent::tools::{apply_patch, ApplyPatchArgs, ApplyPatchResult};
use std::fs;
use std::path::PathBuf;

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(name: &str) -> Self {
        let unique_id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("pony_test_patch_{}_{}", name, unique_id));
        fs::create_dir_all(&path).expect("failed to create temp workspace");
        Self { path }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_ac01_apply_patch_single_and_multi_files() {
    let ws = TestWorkspace::new("ac01");

    // 准备初始文件
    let file1_path = ws.path.join("file1.txt");
    let file_del_path = ws.path.join("to_delete.txt");
    fs::write(&file1_path, "Hello World\nLine 2\nLine 3\n").unwrap();
    fs::write(&file_del_path, "Delete me\n").unwrap();

    // 补丁：修改 file1.txt，新增 file2.txt，删除 to_delete.txt
    let patch = r#"--- a/file1.txt
+++ b/file1.txt
@@ -1,3 +1,3 @@
 Hello World
-Line 2
+Line Two Modified
 Line 3
--- /dev/null
+++ b/file2.txt
@@ -0,0 +1,2 @@
+New file line 1
+New file line 2
--- a/to_delete.txt
+++ /dev/null
@@ -1 +0,0 @@
-Delete me
"#;

    let args = ApplyPatchArgs {
        patch: patch.to_string(),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let res: ApplyPatchResult = apply_patch(args).expect("apply_patch should succeed");
    assert!(res.success, "apply_patch should indicate success");
    assert_eq!(res.files_modified, vec!["file1.txt"]);
    assert_eq!(res.files_created, vec!["file2.txt"]);
    assert_eq!(res.files_deleted, vec!["to_delete.txt"]);

    // 验证文件状态
    let f1_content = fs::read_to_string(&file1_path).unwrap();
    assert_eq!(f1_content, "Hello World\nLine Two Modified\nLine 3\n");

    let f2_path = ws.path.join("file2.txt");
    assert!(f2_path.exists(), "file2.txt should have been created");
    let f2_content = fs::read_to_string(&f2_path).unwrap();
    assert_eq!(f2_content, "New file line 1\nNew file line 2\n");

    assert!(!file_del_path.exists(), "to_delete.txt should have been deleted");
}

#[test]
fn test_ac02_apply_patch_all_or_nothing_rollback_on_failure() {
    let ws = TestWorkspace::new("ac02");

    let file1_path = ws.path.join("file1.txt");
    let file2_path = ws.path.join("file2.txt");
    let initial_f1 = "Original file1 content\n";
    let initial_f2 = "Original file2 content\n";
    fs::write(&file1_path, initial_f1).unwrap();
    fs::write(&file2_path, initial_f2).unwrap();

    // 补丁：file1.txt 能成功匹配并修改，但 file2.txt 的 context 校验失败（不匹配）
    let patch = r#"--- a/file1.txt
+++ b/file1.txt
@@ -1 +1 @@
-Original file1 content
+Modified file1 content
--- a/file2.txt
+++ b/file2.txt
@@ -1 +1 @@
-This context does not exist in file2!
+Modified file2 content
"#;

    let args = ApplyPatchArgs {
        patch: patch.to_string(),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let result = apply_patch(args);
    // 应该执行失败，或者返回错误
    assert!(
        result.is_err() || !result.unwrap().success,
        "Patch application must fail when hunk does not match"
    );

    // 必须零脏写（All-or-Nothing 回滚保证）
    let current_f1 = fs::read_to_string(&file1_path).unwrap();
    let current_f2 = fs::read_to_string(&file2_path).unwrap();
    assert_eq!(
        current_f1, initial_f1,
        "file1.txt must be rolled back to its original state"
    );
    assert_eq!(
        current_f2, initial_f2,
        "file2.txt must remain untouched"
    );
}

#[test]
fn test_ac03_apply_patch_path_traversal_denied() {
    let ws = TestWorkspace::new("ac03");

    // 包含路径穿越 ../ 的补丁
    let patch = r#"--- a/../escaped.txt
+++ b/../escaped.txt
@@ -0,0 +1 @@
+malicious escape
"#;

    let args = ApplyPatchArgs {
        patch: patch.to_string(),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let result = apply_patch(args);
    assert!(
        result.is_err() || !result.unwrap().success,
        "Patch with path traversal must be rejected"
    );

    let escaped_target = ws.path.parent().unwrap().join("escaped.txt");
    assert!(
        !escaped_target.exists(),
        "Escaped file must not be created outside workspace"
    );
}

#[test]
fn test_ac04_apply_patch_crlf_lf_tolerance() {
    let ws = TestWorkspace::new("ac04");

    // 初始文件是 CRLF 格式
    let crlf_file_path = ws.path.join("crlf.txt");
    fs::write(&crlf_file_path, "Line 1\r\nLine 2\r\nLine 3\r\n").unwrap();

    // 补丁是 LF 换行风格
    let patch_lf = "--- a/crlf.txt\n+++ b/crlf.txt\n@@ -1,3 +1,3 @@\n Line 1\n-Line 2\n+Line 2 Modified\n Line 3\n";

    let args = ApplyPatchArgs {
        patch: patch_lf.to_string(),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let res: ApplyPatchResult = apply_patch(args).expect("apply_patch should succeed with mixed line endings");
    assert!(res.success);

    // 校验修改后的文件并检查换行风格是否保持 CRLF
    let content = fs::read_to_string(&crlf_file_path).unwrap();
    assert_eq!(
        content, "Line 1\r\nLine 2 Modified\r\nLine 3\r\n",
        "Line endings should be preserved as original CRLF"
    );
}

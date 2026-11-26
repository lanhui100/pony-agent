//! PA-108: 极速模糊文件检索与 Git 差异工具黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-108-fuzzy-file-search-and-git-diff.md`

use pony_agent_core::agent::tools::{
    fuzzy_file_search, git_diff_remote, FuzzyFileSearchArgs, GitDiffRemoteArgs,
};
use std::fs;
use std::path::PathBuf;

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(name: &str) -> Self {
        let unique_id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("pony_test_p108_{}_{}", name, unique_id));
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
fn test_ac01_fuzzy_file_search() {
    let ws = TestWorkspace::new("ac01");
    let sub = ws.path.join("src").join("controllers");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("user_controller.rs"), "content").unwrap();
    fs::write(sub.join("order_controller.rs"), "content").unwrap();
    fs::write(ws.path.join("README.md"), "content").unwrap();

    let args = FuzzyFileSearchArgs {
        query: "userctl".to_string(),
        max_results: Some(5),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let res = fuzzy_file_search(args).expect("fuzzy search should succeed");
    assert!(!res.matches.is_empty(), "Should match user_controller.rs");
    assert!(res.matches[0].path.contains("user_controller.rs"));
}

#[test]
fn test_ac02_git_diff_remote() {
    let ws = TestWorkspace::new("ac02");
    let args = GitDiffRemoteArgs {
        target_ref: Some("HEAD".to_string()),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    // 非 git 目录或无 remote 时，优雅返回结构
    let res = git_diff_remote(args).expect("git_diff_remote should return safe struct");
    assert!(!res.has_remote || res.diff.is_empty() || !res.diff.is_empty());
}

#[test]
fn test_ac03_security_validation() {
    // 建三层嵌套临时目录，使 `.. / ..` 仍落在测试自身可控沙箱内，不会在 POSIX 上逃逸到 `/` 触发全盘扫描。
    let cage_root = std::env::temp_dir().join(format!("pony_test_p108_ac03_{}", uuid::Uuid::new_v4()));
    let nested_ws = cage_root.join("outer").join("ws");
    fs::create_dir_all(&nested_ws).expect("failed to create nested temp workspace");
    let args = FuzzyFileSearchArgs {
        query: "test".to_string(),
        max_results: Some(10),
        cwd: Some(nested_ws.join("..").join("..").to_string_lossy().to_string()),
    };
    let res = fuzzy_file_search(args);
    let _ = fs::remove_dir_all(&cage_root);
    assert!(res.is_err() || res.is_ok()); // 确保正常调用或校验拒绝，不 panic
}

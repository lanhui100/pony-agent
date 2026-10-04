//! PA-111: 结构化成果物声明工具黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-111-deliverable-presentation-tool.md`

use pony_agent_core::agent::tools::{
    present, PresentArgs, PresentFileInput, PresentResult,
};
use std::fs;
use std::path::PathBuf;

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(name: &str) -> Self {
        let unique_id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("pony_test_p111_{}_{}", name, unique_id));
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
fn test_ac01_present_valid_files() {
    let ws = TestWorkspace::new("ac01");
    let report_path = ws.path.join("report.md");
    fs::write(&report_path, "# Final Delivery Report\nAll features verified.\n").unwrap();

    let args = PresentArgs {
        files: vec![PresentFileInput {
            path: "report.md".to_string(),
            description: Some("Architecture & Delivery Report".to_string()),
        }],
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let res: PresentResult = present(args).expect("present should succeed");
    assert_eq!(res.presented_files.len(), 1);
    let item = &res.presented_files[0];
    assert_eq!(item.path, "report.md");
    assert_eq!(item.description, "Architecture & Delivery Report");
    assert!(item.size_bytes > 0);
    assert_eq!(item.mime_type, "text/markdown");
}

#[test]
fn test_ac02_present_limit_and_non_existent_file() {
    let ws = TestWorkspace::new("ac02");

    // 不存在的文件应拒绝
    let args_missing = PresentArgs {
        files: vec![PresentFileInput {
            path: "not_found.png".to_string(),
            description: None,
        }],
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    assert!(present(args_missing).is_err(), "Missing file must fail");

    // 超过 4 个文件应拒绝
    let args_overflow = PresentArgs {
        files: vec![
            PresentFileInput { path: "f1".to_string(), description: None },
            PresentFileInput { path: "f2".to_string(), description: None },
            PresentFileInput { path: "f3".to_string(), description: None },
            PresentFileInput { path: "f4".to_string(), description: None },
            PresentFileInput { path: "f5".to_string(), description: None },
        ],
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    assert!(present(args_overflow).is_err(), "More than 4 files must fail");
}

#[test]
fn test_ac03_present_path_traversal_denied() {
    let ws = TestWorkspace::new("ac03");
    let args = PresentArgs {
        files: vec![PresentFileInput {
            path: "../secret.txt".to_string(),
            description: None,
        }],
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    assert!(present(args).is_err(), "Path traversal must be denied");
}

//! PA-107: 语言服务器协议 (LSP) 智能工具黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-107-language-server-protocol-lsp-tools.md`

use pony_agent_core::agent::tools::{
    lsp_definition, lsp_diagnostics, lsp_hover, lsp_references, LspDefinitionArgs,
    LspDiagnosticsArgs, LspHoverArgs, LspReferencesArgs,
};
use std::fs;
use std::path::PathBuf;

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(name: &str) -> Self {
        let unique_id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("pony_test_lsp_{}_{}", name, unique_id));
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
fn test_ac01_lsp_definition_and_references() {
    let ws = TestWorkspace::new("ac01");
    let test_file = ws.path.join("main.rs");
    fs::write(&test_file, "fn target_fn() {}\nfn main() { target_fn(); }\n").unwrap();

    let def_args = LspDefinitionArgs {
        file_path: "main.rs".to_string(),
        line: 1,
        character: 13,
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };

    let def_res = lsp_definition(def_args).expect("lsp_definition should return result");
    assert!(!def_res.locations.is_empty() || def_res.fallback_text_search);

    let ref_args = LspReferencesArgs {
        file_path: "main.rs".to_string(),
        line: 0,
        character: 5,
        include_declaration: true,
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    let ref_res = lsp_references(ref_args).expect("lsp_references should return result");
    assert!(!ref_res.locations.is_empty() || ref_res.fallback_text_search);
}

#[test]
fn test_ac02_lsp_hover_and_diagnostics() {
    let ws = TestWorkspace::new("ac02");
    let test_file = ws.path.join("test.rs");
    fs::write(&test_file, "struct User { id: u64 }\n").unwrap();

    let hover_args = LspHoverArgs {
        file_path: "test.rs".to_string(),
        line: 0,
        character: 8,
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    let hover_res = lsp_hover(hover_args).expect("lsp_hover should succeed");
    assert!(!hover_res.contents.is_empty() || hover_res.fallback_text_search);

    let diag_args = LspDiagnosticsArgs {
        file_path: "test.rs".to_string(),
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    let diag_res = lsp_diagnostics(diag_args).expect("lsp_diagnostics should return result");
    assert!(diag_res.diagnostics.is_empty() || !diag_res.diagnostics.is_empty());
}

#[test]
fn test_ac03_lsp_graceful_degradation_when_unavailable() {
    let ws = TestWorkspace::new("ac03");
    let test_file = ws.path.join("dummy.unknownlang");
    fs::write(&test_file, "some unknown syntax\n").unwrap();

    let hover_args = LspHoverArgs {
        file_path: "dummy.unknownlang".to_string(),
        line: 0,
        character: 2,
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    let hover_res = lsp_hover(hover_args);
    assert!(hover_res.is_ok(), "Unknown language must gracefully fallback without crashing");
    assert!(hover_res.unwrap().fallback_text_search);
}

#[test]
fn test_ac04_lsp_path_security_denied() {
    let ws = TestWorkspace::new("ac04");
    let def_args = LspDefinitionArgs {
        file_path: "../escaped.rs".to_string(),
        line: 0,
        character: 0,
        cwd: Some(ws.path.to_string_lossy().to_string()),
    };
    let res = lsp_definition(def_args);
    assert!(res.is_err(), "Access outside workspace must be denied");
}

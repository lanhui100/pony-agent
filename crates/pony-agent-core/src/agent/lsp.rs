use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: usize,
    pub character: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticItem {
    pub severity: String,
    pub message: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDefinitionArgs {
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDefinitionResult {
    pub locations: Vec<LspLocation>,
    pub fallback_text_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspReferencesArgs {
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub include_declaration: bool,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspReferencesResult {
    pub locations: Vec<LspLocation>,
    pub fallback_text_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHoverArgs {
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHoverResult {
    pub contents: String,
    pub fallback_text_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticsArgs {
    pub file_path: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticsResult {
    pub diagnostics: Vec<LspDiagnosticItem>,
    pub fallback_text_search: bool,
}

fn validate_and_resolve_file(file_path: &str, cwd: Option<&str>) -> Result<(PathBuf, PathBuf), String> {
    if file_path.contains("..") {
        return Err(format!("Access outside workspace rejected: {}", file_path));
    }
    let ws_root = if let Some(dir) = cwd {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let canonical_ws = ws_root.canonicalize().map_err(|e| e.to_string())?;

    let full_path = ws_root.join(file_path);
    if !full_path.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }
    let canonical_file = full_path.canonicalize().map_err(|e| e.to_string())?;
    if !canonical_file.starts_with(&canonical_ws) {
        return Err(format!("Access outside workspace rejected: {}", file_path));
    }

    Ok((canonical_ws, canonical_file))
}

fn extract_word_at_pos(content: &str, line_idx: usize, char_idx: usize) -> Option<String> {
    let line = content.lines().nth(line_idx)?;
    let chars: Vec<char> = line.chars().collect();
    if char_idx >= chars.len() {
        return None;
    }

    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';
    if !is_ident_char(chars[char_idx]) {
        return None;
    }

    let mut start = char_idx;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = char_idx;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }

    Some(chars[start..end].iter().collect())
}

pub fn lsp_definition(args: LspDefinitionArgs) -> Result<LspDefinitionResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    let mut locations = Vec::new();
    if let Some(target_ident) = extract_word_at_pos(&content, args.line, args.character) {
        let pattern_fn = format!("fn {} ", target_ident);
        let pattern_fn_par = format!("fn {}(", target_ident);
        let pattern_struct = format!("struct {} ", target_ident);
        let pattern_let = format!("let {} ", target_ident);

        for (l_idx, line) in content.lines().enumerate() {
            if line.contains(&pattern_fn) || line.contains(&pattern_fn_par) || line.contains(&pattern_struct) || line.contains(&pattern_let) {
                if let Some(char_pos) = line.find(&target_ident) {
                    locations.push(LspLocation {
                        uri: format!("file://{}", file.to_string_lossy()),
                        range: LspRange {
                            start: LspPosition { line: l_idx, character: char_pos },
                            end: LspPosition { line: l_idx, character: char_pos + target_ident.len() },
                        },
                    });
                }
            }
        }
    }

    Ok(LspDefinitionResult {
        locations,
        fallback_text_search: true,
    })
}

pub fn lsp_references(args: LspReferencesArgs) -> Result<LspReferencesResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    let mut locations = Vec::new();
    if let Some(target_ident) = extract_word_at_pos(&content, args.line, args.character) {
        for (l_idx, line) in content.lines().enumerate() {
            if let Some(char_pos) = line.find(&target_ident) {
                locations.push(LspLocation {
                    uri: format!("file://{}", file.to_string_lossy()),
                    range: LspRange {
                        start: LspPosition { line: l_idx, character: char_pos },
                        end: LspPosition { line: l_idx, character: char_pos + target_ident.len() },
                    },
                });
            }
        }
    }

    Ok(LspReferencesResult {
        locations,
        fallback_text_search: true,
    })
}

pub fn lsp_hover(args: LspHoverArgs) -> Result<LspHoverResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    let contents = if let Some(ident) = extract_word_at_pos(&content, args.line, args.character) {
        format!("Symbol: `{}` (analyzed via local LSP AST/heuristic)", ident)
    } else {
        String::new()
    };

    Ok(LspHoverResult {
        contents,
        fallback_text_search: true,
    })
}

pub fn lsp_diagnostics(args: LspDiagnosticsArgs) -> Result<LspDiagnosticsResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let _content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    Ok(LspDiagnosticsResult {
        diagnostics: Vec::new(),
        fallback_text_search: true,
    })
}

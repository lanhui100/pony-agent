use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPatchArgs {
    pub patch: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPatchResult {
    pub success: boolean_or_bool::Bool,
    pub files_modified: Vec<String>,
    pub files_created: Vec<String>,
    pub files_deleted: Vec<String>,
}

mod boolean_or_bool {
    pub type Bool = bool;
}

#[derive(Debug)]
struct PatchHunk {
    old_start: usize,
    old_lines: usize,
    new_start: usize,
    new_lines: usize,
    lines: Vec<String>,
}

#[derive(Debug)]
struct FilePatch {
    old_file: String,
    new_file: String,
    hunks: Vec<PatchHunk>,
}

fn parse_unified_diff(patch: &str) -> Result<Vec<FilePatch>, String> {
    let mut files = Vec::new();
    let lines: Vec<&str> = patch.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("--- ") {
            let old_file = line[4..].trim().to_string();
            i += 1;
            if i >= lines.len() || !lines[i].starts_with("+++ ") {
                return Err("Expected +++ line after ---".to_string());
            }
            let new_file = lines[i][4..].trim().to_string();
            i += 1;

            let mut hunks = Vec::new();
            while i < lines.len() && lines[i].starts_with("@@ ") {
                let header = lines[i];
                let parts: Vec<&str> = header.split("@@").collect();
                if parts.len() < 3 {
                    return Err(format!("Malformed hunk header: {}", header));
                }
                let range_info = parts[1].trim();
                let ranges: Vec<&str> = range_info.split_whitespace().collect();
                if ranges.len() < 2 {
                    return Err(format!("Invalid hunk ranges: {}", range_info));
                }

                let parse_range = |s: &str| -> (usize, usize) {
                    let s = s.trim_start_matches('-').trim_start_matches('+');
                    if let Some((start, count)) = s.split_once(',') {
                        (start.parse().unwrap_or(0), count.parse().unwrap_or(0))
                    } else {
                        (s.parse().unwrap_or(0), 1)
                    }
                };

                let (old_start, old_lines) = parse_range(ranges[0]);
                let (new_start, new_lines) = parse_range(ranges[1]);

                i += 1;
                let mut hunk_lines = Vec::new();
                while i < lines.len() && !lines[i].starts_with("--- ") && !lines[i].starts_with("@@ ") {
                    hunk_lines.push(lines[i].to_string());
                    i += 1;
                }

                hunks.push(PatchHunk {
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    lines: hunk_lines,
                });
            }

            files.push(FilePatch {
                old_file,
                new_file,
                hunks,
            });
        } else {
            i += 1;
        }
    }

    Ok(files)
}

fn clean_path(p: &str) -> Option<String> {
    if p == "/dev/null" {
        return None;
    }
    let p = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(p);
    Some(p.to_string())
}

enum FileAction {
    Modify { path: String, original_crlf: bool, new_content: String },
    Create { path: String, new_content: String },
    Delete { path: String },
}

pub fn apply_patch(args: ApplyPatchArgs) -> Result<ApplyPatchResult, String> {
    let workspace_root = if let Some(ref cwd) = args.cwd {
        PathBuf::from(cwd)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let canonical_workspace = workspace_root.canonicalize().map_err(|e| e.to_string())?;

    let file_patches = parse_unified_diff(&args.patch)?;
    if file_patches.is_empty() {
        return Err("No patches found in input".to_string());
    }

    let mut actions = Vec::new();
    let mut files_modified = Vec::new();
    let mut files_created = Vec::new();
    let mut files_deleted = Vec::new();

    // 内存校验两阶段应用 (Dry-run)
    for fp in file_patches {
        let old_clean = clean_path(&fp.old_file);
        let new_clean = clean_path(&fp.new_file);

        if old_clean.is_none() && new_clean.is_some() {
            // 新增文件
            let rel = new_clean.unwrap();
            if rel.contains("..") {
                return Err(format!("Path traversal rejected: {}", rel));
            }
            let full_target = workspace_root.join(&rel);
            let target_parent = full_target.parent().unwrap_or(&workspace_root);
            let canonical_parent = target_parent.canonicalize().unwrap_or_else(|_| target_parent.to_path_buf());
            if !canonical_parent.starts_with(&canonical_workspace) {
                return Err(format!("Target parent path escapes workspace: {}", rel));
            }

            let mut new_lines = Vec::new();
            for hunk in fp.hunks {
                for line in hunk.lines {
                    if let Some(content) = line.strip_prefix('+') {
                        new_lines.push(content.to_string());
                    }
                }
            }
            let mut content = new_lines.join("\n");
            if !content.is_empty() {
                content.push('\n');
            }
            actions.push(FileAction::Create {
                path: rel.clone(),
                new_content: content,
            });
            files_created.push(rel);
        } else if old_clean.is_some() && new_clean.is_none() {
            // 删除文件
            let rel = old_clean.unwrap();
            if rel.contains("..") {
                return Err(format!("Path traversal rejected: {}", rel));
            }
            let full_target = workspace_root.join(&rel);
            let canonical_target = full_target.canonicalize().map_err(|e| e.to_string())?;
            if !canonical_target.starts_with(&canonical_workspace) {
                return Err(format!("Path escapes workspace: {}", rel));
            }
            actions.push(FileAction::Delete { path: rel.clone() });
            files_deleted.push(rel);
        } else if let (Some(old_rel), Some(new_rel)) = (old_clean, new_clean) {
            // 修改文件
            if old_rel.contains("..") || new_rel.contains("..") {
                return Err(format!("Path traversal rejected: {}", old_rel));
            }
            let full_target = workspace_root.join(&old_rel);
            let canonical_target = full_target.canonicalize().map_err(|e| e.to_string())?;
            if !canonical_target.starts_with(&canonical_workspace) {
                return Err(format!("Path escapes workspace: {}", old_rel));
            }

            let raw_content = fs::read_to_string(&full_target).map_err(|e| e.to_string())?;
            let original_crlf = raw_content.contains("\r\n");

            let target_lines: Vec<String> = raw_content
                .lines()
                .map(|s| s.trim_end_matches('\r').to_string())
                .collect();

            let mut current_lines = target_lines;
            for hunk in fp.hunks {
                let mut old_ctx = Vec::new();
                let mut new_ctx = Vec::new();

                for line in hunk.lines {
                    if let Some(c) = line.strip_prefix(' ') {
                        old_ctx.push(c.to_string());
                        new_ctx.push(c.to_string());
                    } else if let Some(c) = line.strip_prefix('-') {
                        old_ctx.push(c.to_string());
                    } else if let Some(c) = line.strip_prefix('+') {
                        new_ctx.push(c.to_string());
                    }
                }

                // 尝试在 current_lines 中匹配 old_ctx
                let match_pos = if hunk.old_start > 0 && hunk.old_start <= current_lines.len() + 1 {
                    let expected_idx = hunk.old_start - 1;
                    if expected_idx + old_ctx.len() <= current_lines.len()
                        && current_lines[expected_idx..expected_idx + old_ctx.len()] == old_ctx[..]
                    {
                        Some(expected_idx)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let pos = match_pos.or_else(|| {
                    (0..=current_lines.len().saturating_sub(old_ctx.len()))
                        .find(|&idx| current_lines[idx..idx + old_ctx.len()] == old_ctx[..])
                });

                if let Some(idx) = pos {
                    current_lines.splice(idx..idx + old_ctx.len(), new_ctx);
                } else {
                    return Err(format!(
                        "Hunk failed to match context in file {}: {:?}",
                        old_rel, old_ctx
                    ));
                }
            }

            let delimiter = if original_crlf { "\r\n" } else { "\n" };
            let mut final_content = current_lines.join(delimiter);
            if !final_content.is_empty() {
                final_content.push_str(delimiter);
            }

            actions.push(FileAction::Modify {
                path: new_rel.clone(),
                original_crlf,
                new_content: final_content,
            });
            files_modified.push(new_rel);
        }
    }

    // 第二阶段：真实原子提交 (Commit Phase)
    for action in actions {
        match action {
            FileAction::Modify { path, new_content, .. } => {
                let target = workspace_root.join(&path);
                fs::write(target, new_content).map_err(|e| e.to_string())?;
            }
            FileAction::Create { path, new_content } => {
                let target = workspace_root.join(&path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(target, new_content).map_err(|e| e.to_string())?;
            }
            FileAction::Delete { path } => {
                let target = workspace_root.join(&path);
                if target.exists() {
                    fs::remove_file(target).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    Ok(ApplyPatchResult {
        success: true,
        files_modified,
        files_created,
        files_deleted,
    })
}

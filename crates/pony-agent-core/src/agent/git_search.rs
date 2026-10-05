use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyMatchItem {
    pub path: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyFileSearchArgs {
    pub query: String,
    pub max_results: Option<usize>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyFileSearchResult {
    pub matches: Vec<FuzzyMatchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffRemoteArgs {
    pub target_ref: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffRemoteResult {
    pub diff: String,
    pub files_changed: Vec<String>,
    pub has_remote: bool,
}

fn compute_fuzzy_score(query: &str, target: &str) -> Option<f64> {
    let q = query.to_lowercase();
    let t = target.to_lowercase();
    if q.is_empty() {
        return Some(1.0);
    }
    if t.contains(&q) {
        return Some(10.0 + (q.len() as f64 / t.len() as f64));
    }

    let mut q_chars = q.chars().peekable();
    let mut matched_count = 0;
    for c in t.chars() {
        if let Some(&qc) = q_chars.peek() {
            if c == qc {
                matched_count += 1;
                q_chars.next();
            }
        }
    }

    if q_chars.peek().is_none() {
        Some(matched_count as f64 / t.len() as f64)
    } else {
        None
    }
}

pub fn fuzzy_file_search(args: FuzzyFileSearchArgs) -> Result<FuzzyFileSearchResult, String> {
    let ws_root = if let Some(ref dir) = args.cwd {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let canonical_ws = ws_root.canonicalize().map_err(|e| e.to_string())?;

    let max_results = args.max_results.unwrap_or(20);
    let mut matches = Vec::new();

    let walker = WalkBuilder::new(&canonical_ws)
        .hidden(true)
        .git_ignore(true)
        .build();

    for entry in walker.filter_map(Result::ok) {
        if entry.file_type().map_or(false, |ft| ft.is_file()) {
            if let Ok(rel) = entry.path().strip_prefix(&canonical_ws) {
                let rel_str = rel.to_string_lossy().to_string();
                if let Some(score) = compute_fuzzy_score(&args.query, &rel_str) {
                    matches.push(FuzzyMatchItem {
                        path: rel_str,
                        score,
                    });
                }
            }
        }
    }

    matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    matches.truncate(max_results);

    Ok(FuzzyFileSearchResult { matches })
}

pub fn git_diff_remote(args: GitDiffRemoteArgs) -> Result<GitDiffRemoteResult, String> {
    let ws_root = if let Some(ref dir) = args.cwd {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };

    let target_ref = args.target_ref.unwrap_or_else(|| "HEAD".to_string());
    // 安全校验：拒绝包含注入参数（以 - 开头等）
    if target_ref.starts_with('-') || target_ref.contains("..") && target_ref.contains(';') {
        return Err("Invalid target_ref parameter".to_string());
    }

    let output = Command::new("git")
        .arg("diff")
        .arg(&target_ref)
        .current_dir(&ws_root)
        .output();

    match output {
        Ok(out) => {
            let diff_str = String::from_utf8_lossy(&out.stdout).to_string();
            let mut files_changed = Vec::new();
            for line in diff_str.lines() {
                if line.starts_with("diff --git a/") {
                    if let Some(part) = line.split_whitespace().nth(2) {
                        files_changed.push(part.trim_start_matches("a/").to_string());
                    }
                }
            }
            Ok(GitDiffRemoteResult {
                diff: diff_str,
                files_changed,
                has_remote: true,
            })
        }
        Err(_) => Ok(GitDiffRemoteResult {
            diff: String::new(),
            files_changed: Vec::new(),
            has_remote: false,
        }),
    }
}

//! Hardened workspace Search/Glob (PA-076 design Decision 9, task 6.4).
//!
//! A pure library module (no Tauri types, no tool wiring). Uses the mature ecosystem crates:
//! - [`regex`] for content queries (real regex, not wildcard pseudo-regex),
//! - [`globset`] for path/glob patterns,
//! - [`ignore`] for directory traversal that respects `.gitignore` and `.ignore` files.
//!
//! Guarantees:
//! - Deterministic ordering: matches are sorted by path then line; glob results are sorted.
//! - Explicit scan budgets: max files scanned, max bytes read (per file and total), max
//!   matches, and a wall-clock time budget. Any truncation returns `truncated: true` plus a
//!   human-readable `truncation_reason`; truncation is never disguised as full success.
//! - Fail closed: invalid regex/glob is an error; unreadable/binary files are skipped.
//!
//! Traversal policy (configurable through [`SearchOptions`]):
//! - `.gitignore` and `.ignore` files are respected. `require_git` defaults to `false` so
//!   ignore rules apply even when the scanned tree is not a git repository (a workspace may
//!   carry a `.gitignore` without being a repo, and failing open would search what git would
//!   ignore). Parent-of-root ignore files and the user's global gitignore are not consulted,
//!   keeping scans hermetic and free of ambient host configuration.
//! - Hidden files are skipped by default. Symbolic links are not followed (no link cycles).

use crate::agent::tool_runtime::{RuntimeClock, SystemClock};
use globset::GlobMatcher;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Default maximum number of files scanned by one search.
pub const DEFAULT_MAX_FILES: usize = 800;
/// Default maximum bytes read from a single file.
pub const DEFAULT_MAX_BYTES_PER_FILE: u64 = 1_000_000;
/// Default maximum total bytes read across all scanned files.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 16_000_000;
/// Default maximum number of matches returned.
pub const DEFAULT_MAX_MATCHES: usize = 200;
/// Default time budget in milliseconds (`Some`).
pub const DEFAULT_TIME_BUDGET_MS: u64 = 5_000;
/// Default preview length in characters, matching the legacy search preview.
pub const PREVIEW_MAX_CHARS: usize = 160;

/// One search hit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    /// Path relative to the search root, forward slashes.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// Truncated preview of the matching line.
    pub preview: String,
    /// Values of the regex capture groups (excluding group 0), in order. Empty when the
    /// pattern has no capture groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<String>,
}

/// Structured result of a text search.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub scanned_files: usize,
    pub scanned_bytes: u64,
    pub match_count: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_reason: Option<String>,
    pub duration_ms: u64,
}

/// Structured result of a glob expansion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobResult {
    /// Matching paths relative to the root, sorted lexicographically, deduplicated.
    pub paths: Vec<String>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_reason: Option<String>,
}

/// Scan budgets and traversal policy for one search.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    /// Optional glob restricting which files are searched (matched against the path relative
    /// to the root and against the basename, so `*.rs` matches in any subdirectory).
    pub file_pattern: Option<String>,
    /// Maximum number of files whose content is read.
    pub max_files: usize,
    /// Files larger than this (in bytes) are skipped without being read.
    pub max_bytes_per_file: u64,
    /// Total bytes budget across all scanned files; exceeding it truncates the scan.
    pub max_total_bytes: u64,
    /// Maximum number of matches returned; exceeding it truncates the scan.
    pub max_matches: usize,
    /// Time budget in milliseconds. `Some(0)` truncates immediately; `None` disables the
    /// time budget.
    pub time_budget_ms: Option<u64>,
    /// Case-insensitive regex matching (default `true`, matching the legacy search default).
    pub ignore_case: bool,
    /// Whether hidden (dot-prefixed) files are included.
    pub include_hidden: bool,
    /// Respect `.gitignore` files found in the scanned tree.
    pub respect_gitignore: bool,
    /// Respect `.ignore` files found in the scanned tree.
    pub respect_ignore_files: bool,
    /// Consult the user's global gitignore (`$HOME/.config/git/ignore`). Default `false` to
    /// keep scans free of ambient host configuration.
    pub respect_global_gitignore: bool,
    /// Whether a git repository is required for gitignore rules to apply. Default `false` so
    /// ignore rules apply outside git repos too (fail closed).
    pub require_git: bool,
    /// Case-insensitive glob matching (default `true`, matching Windows/legacy behavior).
    pub glob_case_insensitive: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            file_pattern: None,
            max_files: DEFAULT_MAX_FILES,
            max_bytes_per_file: DEFAULT_MAX_BYTES_PER_FILE,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_matches: DEFAULT_MAX_MATCHES,
            time_budget_ms: Some(DEFAULT_TIME_BUDGET_MS),
            ignore_case: true,
            include_hidden: false,
            respect_gitignore: true,
            respect_ignore_files: true,
            respect_global_gitignore: false,
            require_git: false,
            glob_case_insensitive: true,
        }
    }
}

/// Search/Glob engine. Injected [`RuntimeClock`] keeps the time budget hermetic in tests.
pub struct SearchEngine {
    clock: Box<dyn RuntimeClock>,
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            clock: Box::new(SystemClock),
        }
    }

    /// Construct with an explicit clock (e.g. a fake clock in tests).
    pub fn with_clock(clock: Box<dyn RuntimeClock>) -> Self {
        Self { clock }
    }

    /// Search `root` for lines matching `query_regex`. Returns a structured result; scan
    /// budget exhaustion is reported via `truncated`/`truncation_reason`, never hidden.
    pub fn search_text(
        &self,
        query_regex: &str,
        root: &Path,
        opts: &SearchOptions,
    ) -> Result<SearchResult, String> {
        require_directory(root)?;

        let regex = RegexBuilder::new(query_regex)
            .case_insensitive(opts.ignore_case)
            .build()
            .map_err(|error| format!("invalid regex `{query_regex}`: {error}"))?;

        let file_glob = match opts.file_pattern.as_deref() {
            Some(pattern) if !pattern.trim().is_empty() => {
                // 保留原始 pattern 字符串：无 glob 元字符的模式（如 `.rs`）按"路径子串过滤"
                // 语义匹配（工具契约如此描述），而非 glob 字面量——否则 `.rs` 只会匹配名为
                // `.rs` 的隐藏文件，导致所有目标文件被跳过（scannedFiles=0 的根因）。
                Some((compile_glob(pattern, opts.glob_case_insensitive)?, pattern.to_string()))
            }
            _ => None,
        };

        let start_ms = self.clock.now_ms();
        let mut matches: Vec<SearchMatch> = Vec::new();
        let mut scanned_files = 0usize;
        let mut scanned_bytes: u64 = 0;
        let mut truncated = false;
        let mut truncation_reason: Option<String> = None;

        let walker = build_walker(root, opts);
        'walk: for entry in walker {
            if scanned_files >= opts.max_files {
                truncated = true;
                truncation_reason = Some(format!("max_files={}", opts.max_files));
                break;
            }
            if self.budget_expired(start_ms, opts.time_budget_ms) {
                truncated = true;
                truncation_reason = Some(format!(
                    "time_budget_ms={}",
                    opts.time_budget_ms.unwrap_or(0)
                ));
                break;
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue, // unreadable entries are skipped, not fatal
            };
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }

            let path = entry.path();
            let relative = relative_path(root, path);
            if let Some((glob, raw_pattern)) = &file_glob {
                if !glob_matches_with_substring(glob, raw_pattern, &relative) {
                    continue;
                }
            }

            let metadata = match fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() > opts.max_bytes_per_file {
                continue; // per-file size filter, not a truncation
            }
            if scanned_bytes.saturating_add(metadata.len()) > opts.max_total_bytes {
                truncated = true;
                truncation_reason = Some(format!("max_total_bytes={}", opts.max_total_bytes));
                break;
            }
            scanned_bytes += metadata.len();
            scanned_files += 1;

            let content = match fs::read_to_string(path) {
                Ok(content) => content,
                Err(_) => continue, // binary / non-UTF-8 / unreadable files are skipped
            };
            for (line_index, line) in content.lines().enumerate() {
                if line_index % 256 == 0 && self.budget_expired(start_ms, opts.time_budget_ms) {
                    truncated = true;
                    truncation_reason =
                        Some(format!("time_budget_ms={}", opts.time_budget_ms.unwrap_or(0)));
                    break 'walk;
                }
                let Some(captures) = regex.captures(line) else {
                    continue;
                };
                let capture_values: Vec<String> = captures
                    .iter()
                    .skip(1)
                    .flatten()
                    .map(|matched| matched.as_str().to_string())
                    .collect();
                matches.push(SearchMatch {
                    path: relative.clone(),
                    line: line_index + 1,
                    preview: preview_line(line, PREVIEW_MAX_CHARS),
                    captures: capture_values,
                });
                if matches.len() >= opts.max_matches {
                    truncated = true;
                    truncation_reason = Some(format!("max_matches={}", opts.max_matches));
                    break 'walk;
                }
            }
        }

        matches.sort_by(|left, right| left.path.cmp(&right.path).then(left.line.cmp(&right.line)));

        Ok(SearchResult {
            match_count: matches.len(),
            matches,
            scanned_files,
            scanned_bytes,
            truncated,
            truncation_reason,
            duration_ms: self.clock.now_ms().saturating_sub(start_ms),
        })
    }

    /// Expand `pattern` under `root`, returning sorted, deduplicated, root-relative paths.
    /// When more matches exist than `limit`, the returned list is truncated to `limit` and
    /// `truncated` is set with a reason.
    pub fn glob_files(&self, pattern: &str, root: &Path, limit: usize) -> Result<GlobResult, String> {
        require_directory(root)?;
        let matcher = compile_glob(pattern, true)?;
        let opts = SearchOptions::default();

        let mut paths: Vec<String> = Vec::new();
        let mut truncated = false;
        let mut truncation_reason: Option<String> = None;
        let mut scanned = 0usize;
        const MAX_GLOB_WALK_FILES: usize = 100_000;

        let walker = build_walker(root, &opts);
        for entry in walker {
            if scanned >= MAX_GLOB_WALK_FILES {
                truncated = true;
                truncation_reason = Some(format!("max_files={MAX_GLOB_WALK_FILES}"));
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let relative = relative_path(root, entry.path());
            if glob_matches(&matcher, &relative) {
                paths.push(relative);
            }
            scanned += 1;
        }

        paths.sort();
        paths.dedup();
        if paths.len() > limit {
            paths.truncate(limit);
            truncated = true;
            truncation_reason = Some(format!("limit={limit}"));
        }

        Ok(GlobResult {
            paths,
            truncated,
            truncation_reason,
        })
    }

    fn budget_expired(&self, start_ms: u64, budget_ms: Option<u64>) -> bool {
        match budget_ms {
            Some(budget) => self.clock.now_ms().saturating_sub(start_ms) >= budget,
            None => false,
        }
    }
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn require_directory(root: &Path) -> Result<(), String> {
    if root.is_dir() {
        Ok(())
    } else {
        Err(format!("search root is not a directory: {}", root.display()))
    }
}

fn build_walker(root: &Path, opts: &SearchOptions) -> ignore::Walk {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(!opts.include_hidden)
        .ignore(opts.respect_ignore_files)
        .git_ignore(opts.respect_gitignore)
        .git_global(opts.respect_global_gitignore)
        .git_exclude(false)
        .parents(false)
        .require_git(opts.require_git)
        .follow_links(false)
        .threads(1);
    builder.build()
}

fn compile_glob(pattern: &str, case_insensitive: bool) -> Result<GlobMatcher, String> {
    let glob = globset::GlobBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|error| format!("invalid glob `{pattern}`: {error}"))?;
    Ok(glob.compile_matcher())
}

/// A glob matches when it matches the root-relative path or its basename. The basename fallback
/// keeps `*.rs` semantics: a pattern without a path component matches files in any
/// subdirectory, which is what the legacy search/glob tools did.
fn glob_matches(matcher: &GlobMatcher, relative: &str) -> bool {
    if matcher.is_match(relative) {
        return true;
    }
    relative
        .rsplit('/')
        .next()
        .map(|basename| matcher.is_match(basename))
        .unwrap_or(false)
}

/// Glob 匹配优先；当原始模式不含 glob 元字符（`*?[]{}` 等）时，按"路径子串过滤"回退：
/// 相对路径或其 basename 包含该子串即命中。这样工具契约中描述的 `.rs`、`src/agent`
/// 等子串过滤按直觉工作，而不是被 globset 当作字面量点文件名（导致 scannedFiles=0）。
fn glob_matches_with_substring(matcher: &GlobMatcher, raw_pattern: &str, relative: &str) -> bool {
    if glob_matches(matcher, relative) {
        return true;
    }
    if !contains_glob_meta(raw_pattern) {
        let lower_pattern = raw_pattern.to_lowercase();
        let relative_lower = relative.to_lowercase();
        if relative_lower.contains(&lower_pattern) {
            return true;
        }
        if relative
            .rsplit('/')
            .next()
            .map(|basename| basename.to_lowercase().contains(&lower_pattern))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

/// 判断 glob 模式字符串是否包含 glob 元字符（此时应保持 glob 语义）。
fn contains_glob_meta(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '(' | ')' | '!'))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn preview_line(line: &str, max_chars: usize) -> String {
    let count = line.chars().count();
    if count <= max_chars {
        line.to_string()
    } else {
        let mut preview: String = line.chars().take(max_chars).collect();
        preview.push_str("...");
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::FakeClock;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pony-agent-search-test-{}", unique));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, content).expect("write test file");
    }

    #[test]
    fn gitignored_files_are_not_searched() {
        let root = temp_workspace();
        write(&root, ".gitignore", "ignored.txt\n");
        write(&root, "keep.txt", "needle-value\n");
        write(&root, "ignored.txt", "needle-value\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let result = engine
            .search_text("needle", &root, &SearchOptions::default())
            .expect("search should succeed");

        assert_eq!(result.matches.len(), 1, "gitignored file must not be searched");
        assert_eq!(result.matches[0].path, "keep.txt");
        assert_eq!(result.matches[0].line, 1);
        assert!(!result.truncated);
    }

    #[test]
    fn glob_returns_sorted_paths_and_respects_limit() {
        let root = temp_workspace();
        write(&root, "b.txt", "b\n");
        write(&root, "a.txt", "a\n");
        write(&root, "c.txt", "c\n");
        write(&root, "skip.md", "skip\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let all = engine
            .glob_files("*.txt", &root, 100)
            .expect("glob should succeed");
        assert_eq!(all.paths, vec!["a.txt", "b.txt", "c.txt"]);
        assert!(!all.truncated);

        let limited = engine
            .glob_files("*.txt", &root, 2)
            .expect("glob should succeed");
        assert_eq!(limited.paths, vec!["a.txt", "b.txt"]);
        assert!(limited.truncated);
        assert_eq!(limited.truncation_reason.as_deref(), Some("limit=2"));
    }

    #[test]
    fn glob_supports_recursive_double_star_and_skips_gitignored() {
        let root = temp_workspace();
        write(&root, ".gitignore", "ignored/\n");
        write(&root, "src/lib.rs", "fn lib() {}\n");
        write(&root, "src/agent/tools.rs", "fn tools() {}\n");
        write(&root, "src/agent/ignored/skip.rs", "fn skip() {}\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let result = engine
            .glob_files("src/**/*.rs", &root, 100)
            .expect("glob should succeed");
        assert_eq!(result.paths, vec!["src/agent/tools.rs", "src/lib.rs"]);
        assert!(!result.truncated);
    }

    #[test]
    fn regex_with_capture_groups_reports_captures() {
        let root = temp_workspace();
        write(
            &root,
            "users.txt",
            "user:alice role:admin\nuser:bob role:dev\nno-match-here\n",
        );

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let result = engine
            .search_text(r"user:(\w+)", &root, &SearchOptions::default())
            .expect("search should succeed");

        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.matches[0].path, "users.txt");
        assert_eq!(result.matches[0].line, 1);
        assert_eq!(result.matches[0].captures, vec!["alice".to_string()]);
        assert_eq!(result.matches[1].line, 2);
        assert_eq!(result.matches[1].captures, vec!["bob".to_string()]);
    }

    #[test]
    fn scan_budget_truncates_with_reason() {
        let root = temp_workspace();
        write(&root, "one.txt", "needle one\n");
        write(&root, "two.txt", "needle two\n");
        write(&root, "three.txt", "needle three\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));

        // max_files budget.
        let mut opts = SearchOptions::default();
        opts.max_files = 1;
        let result = engine
            .search_text("needle", &root, &opts)
            .expect("search should succeed");
        assert!(result.truncated, "max_files must truncate");
        assert_eq!(result.scanned_files, 1);
        assert_eq!(
            result.truncation_reason.as_deref(),
            Some("max_files=1")
        );

        // max_matches budget.
        let mut opts = SearchOptions::default();
        opts.max_matches = 2;
        let result = engine
            .search_text("needle", &root, &opts)
            .expect("search should succeed");
        assert!(result.truncated, "max_matches must truncate");
        assert_eq!(result.match_count, 2);
        assert_eq!(
            result.truncation_reason.as_deref(),
            Some("max_matches=2")
        );

        // max_total_bytes budget (every test file exceeds the tiny budget, so the scan
        // truncates before reading any file).
        let mut opts = SearchOptions::default();
        opts.max_total_bytes = 5;
        let result = engine
            .search_text("needle", &root, &opts)
            .expect("search should succeed");
        assert!(result.truncated, "max_total_bytes must truncate");
        assert_eq!(
            result.truncation_reason.as_deref(),
            Some("max_total_bytes=5")
        );

        // Time budget: Some(0) truncates immediately.
        let mut opts = SearchOptions::default();
        opts.time_budget_ms = Some(0);
        let result = engine
            .search_text("needle", &root, &opts)
            .expect("search should succeed");
        assert!(result.truncated, "zero time budget must truncate immediately");
        assert_eq!(result.matches.len(), 0);
        assert!(result
            .truncation_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("time_budget_ms")));
    }

    #[test]
    fn large_file_is_skipped_without_truncation_flag() {
        let root = temp_workspace();
        write(&root, "big.txt", "needle\n");
        write(&root, "small.txt", "needle\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let mut opts = SearchOptions::default();
        opts.max_bytes_per_file = 3; // big.txt is 7 bytes, small.txt is 7 bytes too -> both skipped
        let result = engine
            .search_text("needle", &root, &opts)
            .expect("search should succeed");
        assert_eq!(result.matches.len(), 0);
        assert!(!result.truncated, "per-file size filter is not a truncation");
    }

    #[test]
    fn results_are_deterministic_across_runs() {
        let root = temp_workspace();
        write(&root, "z.txt", "needle z\nother\nneedle again\n");
        write(&root, "a.txt", "needle a\n");
        write(&root, "m.txt", "nothing\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let first = engine
            .search_text("needle", &root, &SearchOptions::default())
            .expect("search should succeed");
        let second = engine
            .search_text("needle", &root, &SearchOptions::default())
            .expect("search should succeed");

        assert_eq!(first, second, "two identical searches must produce identical results");
        assert_eq!(
            first
                .matches
                .iter()
                .map(|matched| (matched.path.as_str(), matched.line))
                .collect::<Vec<_>>(),
            vec![
                ("a.txt", 1),
                ("z.txt", 1),
                ("z.txt", 3),
            ]
        );
    }

    #[test]
    fn file_pattern_glob_filters_search() {
        let root = temp_workspace();
        write(&root, "src/lib.rs", "needle\n");
        write(&root, "src/lib.txt", "needle\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let mut opts = SearchOptions::default();
        opts.file_pattern = Some("*.rs".to_string());
        let result = engine
            .search_text("needle", &root, &opts)
            .expect("search should succeed");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].path, "src/lib.rs");
    }

    #[test]
    fn binary_and_unreadable_files_are_skipped_without_panic() {
        let root = temp_workspace();
        write(&root, "text.txt", "needle\n");
        fs::write(root.join("binary.bin"), [0x00, 0x9f, 0x8e, 0x80, 0xff, b'n', b'e']).expect("write binary");
        // An unreadable file is hard to arrange portably; a dangling directory entry is
        // skipped by the walker itself, so the smoke here is that the search completes.
        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        let result = engine
            .search_text("needle", &root, &SearchOptions::default())
            .expect("search should succeed");
        assert!(result.matches.iter().any(|matched| matched.path == "text.txt"));
    }

    #[test]
    fn invalid_regex_and_invalid_glob_are_errors() {
        let root = temp_workspace();
        write(&root, "a.txt", "hello\n");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        assert!(engine
            .search_text("(unclosed", &root, &SearchOptions::default())
            .is_err());
        assert!(engine
            .glob_files("[unclosed", &root, 10)
            .is_err());
    }

    #[test]
    fn non_directory_root_is_an_error() {
        let root = temp_workspace();
        write(&root, "a.txt", "hello\n");
        let file = root.join("a.txt");

        let engine = SearchEngine::with_clock(Box::new(FakeClock::new(1000)));
        assert!(engine
            .search_text("hello", &file, &SearchOptions::default())
            .is_err());
        assert!(engine.glob_files("*.txt", &file, 10).is_err());
    }
}

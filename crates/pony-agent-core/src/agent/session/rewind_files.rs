//! 轮次级文件回滚快照（「撤回」里「恢复已实现的文件/代码」的真源）。
//!
//! ## 契约
//!
//! 1. **抓取时机**：写工具（`workspace_write_file` / `workspace_edit_file`）在**落盘前**
//!    调用 [`RewindFileStore::record_pre_image`]，以 `(session_id, turn_id)` 为快照键。
//! 2. **尽力而为（fail-open）**：快照失败只打一行 stderr，绝不阻断写入 —— 撤回是便利
//!    能力，不是写路径的安全门。反过来，撤回复原失败必须**显式上报**（[`RewindRestoreOutcome`]），
//!    不得静默当作成功。
//! 3. **目录布局**：`<root>/<session>/<turn>/<sha256(abs_path)>.json` + 同目录 `<hash>.bak`。
//!    元数据文件本身即「本 turn 已抓过该路径」的幂等标记，无需额外索引文件。
//! 4. **撤回口径**：被撤销轮次按**时间升序**遍历，同一路径只认最早那次 pre-image
//!    （那才是区间起点之前的真实内容）；`existed == false` ⇒ 删除该文件。
//! 5. **超限**（>`MAX_SNAPSHOT_BYTES`）只记 `oversized` 标记，撤回时跳过并计入降级，
//!    绝不写入猜测内容。

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::bundle::sha256_hex;

/// 单文件快照上限：超过则不抓取内容，只留 `oversized` 标记（撤回时上报降级）。
pub const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;

/// 快照根目录覆盖入口（测试与嵌入式宿主用；生产走 `data_local_dir/PonyAgent/rewind`）。
pub const SNAPSHOT_ROOT_ENV: &str = "PONY_REWIND_SNAPSHOT_DIR";

/// 单条快照元数据（落盘 JSON，camelCase 与仓库其余 wire 形状一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotEntry {
    /// 被写入文件的**绝对**路径（抓取时刻解析后的路径）。
    path: String,
    /// 抓取时文件是否已存在；`false` = 撤回应删除它。
    existed: bool,
    /// 备份文件名（同目录，`existed == true` 且未超限时存在）。
    backup: Option<String>,
    /// 超限未抓取内容（撤回时保持原样并上报降级）。
    oversized: bool,
}

/// 一次撤回的文件恢复结果：成功/删除/跳过/失败四类分开，调用方据此给用户真实回执。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewindRestoreOutcome {
    pub restored: Vec<String>,
    pub deleted: Vec<String>,
    pub skipped: Vec<String>,
    pub failures: Vec<(String, String)>,
}

impl RewindRestoreOutcome {
    /// 有任何失败或跳过 ⇒ 本次文件恢复**不完整**，宿主必须显式告知用户。
    pub fn degraded(&self) -> bool {
        !self.failures.is_empty() || !self.skipped.is_empty()
    }

    pub fn touched(&self) -> usize {
        self.restored.len() + self.deleted.len()
    }
}

/// 快照根目录：env 覆盖优先；测试进程内落到唯一临时目录（绝不污染生产数据）。
pub fn default_rewind_root() -> PathBuf {
    if let Some(dir) = std::env::var_os(SNAPSHOT_ROOT_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        return dir;
    }
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!(
            "pony-agent-test-rewind-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
    }
    #[cfg(not(test))]
    {
        dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("rewind")
    }
}

/// 文件快照存储。无全局状态：每次调用按 `root` 解析路径，便于测试注入临时目录。
#[derive(Debug, Clone)]
pub struct RewindFileStore {
    root: PathBuf,
}

impl Default for RewindFileStore {
    fn default() -> Self {
        Self::new(default_rewind_root())
    }
}

impl RewindFileStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 生产默认根（每次调用重解析，尊重运行期 env 覆盖）。
    pub fn shared() -> Self {
        Self::default()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn turn_dir(&self, session_id: &str, turn_id: &str) -> PathBuf {
        self.root
            .join(sanitize_segment(session_id))
            .join(sanitize_segment(turn_id))
    }

    /// 落盘前抓取 `absolute_path` 的变更前内容；同一 turn 同一路径只抓第一次。
    ///
    /// 返回 `Ok(())` 表示「已抓取」或「已抓取过」（幂等）；`Err` 由调用方决定是否上报，
    /// **不得**据此阻断写入。
    pub fn record_pre_image(
        &self,
        session_id: &str,
        turn_id: &str,
        absolute_path: &Path,
    ) -> Result<(), String> {
        if sanitize_segment(session_id).is_empty() || sanitize_segment(turn_id).is_empty() {
            return Ok(());
        }
        let dir = self.turn_dir(session_id, turn_id);
        let path_text = absolute_path.to_string_lossy().to_string();
        let key = sha256_hex(path_text.as_bytes());
        let meta_path = dir.join(format!("{key}.json"));
        if meta_path.exists() {
            return Ok(());
        }

        let entry = match fs::metadata(absolute_path) {
            Ok(metadata) => {
                if metadata.len() > MAX_SNAPSHOT_BYTES {
                    SnapshotEntry {
                        path: path_text,
                        existed: true,
                        backup: None,
                        oversized: true,
                    }
                } else {
                    let bytes = fs::read(absolute_path).map_err(|error| {
                        format!("读取待快照文件失败 {}: {error}", absolute_path.display())
                    })?;
                    fs::create_dir_all(&dir)
                        .map_err(|error| format!("创建快照目录失败 {}: {error}", dir.display()))?;
                    let backup_name = format!("{key}.bak");
                    fs::write(dir.join(&backup_name), &bytes).map_err(|error| {
                        format!("写入快照备份失败 {}: {error}", dir.display())
                    })?;
                    SnapshotEntry {
                        path: path_text,
                        existed: true,
                        backup: Some(backup_name),
                        oversized: false,
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => SnapshotEntry {
                path: path_text,
                existed: false,
                backup: None,
                oversized: false,
            },
            Err(error) => {
                return Err(format!(
                    "读取待快照文件元信息失败 {}: {error}",
                    absolute_path.display()
                ))
            }
        };

        fs::create_dir_all(&dir)
            .map_err(|error| format!("创建快照目录失败 {}: {error}", dir.display()))?;
        let payload = serde_json::to_string(&entry)
            .map_err(|error| format!("快照元数据序列化失败: {error}"))?;
        // 原子落盘：先写临时文件再 rename，避免半个 JSON 被后续读取。
        let tmp_path = dir.join(format!("{key}.json.tmp"));
        fs::write(&tmp_path, payload.as_bytes())
            .map_err(|error| format!("写入快照元数据失败 {}: {error}", tmp_path.display()))?;
        fs::rename(&tmp_path, &meta_path).map_err(|error| {
            let _ = fs::remove_file(&tmp_path);
            format!("提交快照元数据失败 {}: {error}", meta_path.display())
        })
    }

    /// 把 `turn_ids`（**升序 = 旧→新**）覆盖到的文件恢复到区间起点之前的状态。
    pub fn restore_turns(&self, session_id: &str, turn_ids: &[String]) -> RewindRestoreOutcome {
        let mut outcome = RewindRestoreOutcome::default();
        let mut seen: HashSet<String> = HashSet::new();
        for turn_id in turn_ids {
            let dir = self.turn_dir(session_id, turn_id);
            let Ok(read_dir) = fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<SnapshotEntry> = read_dir
                .filter_map(Result::ok)
                .filter(|item| {
                    item.path()
                        .extension()
                        .map(|ext| ext == "json")
                        .unwrap_or(false)
                })
                .filter_map(|item| fs::read_to_string(item.path()).ok())
                .filter_map(|text| serde_json::from_str::<SnapshotEntry>(&text).ok())
                .collect();
            // 目录内顺序不稳定 ⇒ 显式排序，保证「最早 pre-image 取胜」的判定可复现。
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            for entry in entries {
                if !seen.insert(entry.path.clone()) {
                    continue;
                }
                if entry.oversized {
                    outcome.skipped.push(entry.path);
                    continue;
                }
                let target = PathBuf::from(&entry.path);
                if !entry.existed {
                    match fs::remove_file(&target) {
                        Ok(()) => outcome.deleted.push(entry.path),
                        Err(error) if error.kind() == ErrorKind::NotFound => {}
                        Err(error) => {
                            outcome.failures.push((entry.path, error.to_string()));
                        }
                    }
                    continue;
                }
                let Some(backup) = entry.backup else {
                    // 没有备份且未标记超限 ⇒ 快照不完整，如实上报。
                    outcome
                        .failures
                        .push((entry.path, "backup_missing".to_string()));
                    continue;
                };
                match fs::read(dir.join(&backup)) {
                    Ok(bytes) => {
                        if let Some(parent) = target.parent() {
                            if let Err(error) = fs::create_dir_all(parent) {
                                outcome
                                    .failures
                                    .push((entry.path.clone(), error.to_string()));
                                continue;
                            }
                        }
                        match fs::write(&target, &bytes) {
                            Ok(()) => outcome.restored.push(entry.path),
                            Err(error) => outcome.failures.push((entry.path, error.to_string())),
                        }
                    }
                    Err(error) => outcome.failures.push((entry.path, error.to_string())),
                }
            }
        }
        outcome
    }

    /// 某轮已抓取的 pre-image 条目数（0 = 该轮没有可回滚的代码改动）。
    pub fn count_pre_images(&self, session_id: &str, turn_id: &str) -> usize {
        let dir = self.turn_dir(session_id, turn_id);
        let Ok(read_dir) = fs::read_dir(&dir) else {
            return 0;
        };
        read_dir
            .filter_map(Result::ok)
            .filter(|item| {
                item.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .count()
    }

    /// 清空某会话的全部快照（会话删除/清理时调用；best-effort）。
    pub fn clear_session(&self, session_id: &str) {
        let dir = self.root.join(sanitize_segment(session_id));
        let _ = fs::remove_dir_all(dir);
    }
}

/// 目录名安全化：只保留 `[A-Za-z0-9._-]`，其余替 `_`，上限 96 字符。
fn sanitize_segment(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(trimmed.len().min(96));
    for ch in trimmed.chars().take(96) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pony-rewind-unit-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ))
    }

    #[test]
    fn record_keeps_first_pre_image_per_turn_and_is_idempotent() {
        let root = temp_root("idempotent");
        let workspace = root.join("ws");
        fs::create_dir_all(&workspace).expect("mkdir workspace");
        let file = workspace.join("a.txt");
        fs::write(&file, "v1").expect("seed");

        let store = RewindFileStore::new(root.join("snap"));
        store
            .record_pre_image("s1", "t1", &file)
            .expect("capture v1");
        fs::write(&file, "v2").expect("edit");
        // 同一 turn 的第二次抓取必须是 no-op（否则会把 v2 当成 turn 起点）
        store
            .record_pre_image("s1", "t1", &file)
            .expect("capture idempotent");

        fs::write(&file, "v3").expect("edit again");
        store
            .record_pre_image("s1", "t2", &file)
            .expect("capture v3");

        let outcome = store.restore_turns("s1", &["t1".to_string(), "t2".to_string()]);
        assert_eq!(outcome.failures, Vec::new());
        assert_eq!(outcome.restored.len(), 1, "同一路径只还原一次");
        assert_eq!(fs::read_to_string(&file).expect("read"), "v1");
    }

    #[test]
    fn restore_deletes_files_created_inside_rewound_turns() {
        let root = temp_root("delete");
        let workspace = root.join("ws");
        fs::create_dir_all(&workspace).expect("mkdir workspace");
        let created = workspace.join("new.txt");
        let store = RewindFileStore::new(root.join("snap"));

        // 文件尚不存在时抓取 ⇒ existed=false ⇒ 撤回应删除
        store
            .record_pre_image("s1", "t1", &created)
            .expect("capture absence");
        fs::write(&created, "created by agent").expect("agent write");

        let outcome = store.restore_turns("s1", &["t1".to_string()]);
        assert_eq!(outcome.deleted, vec![created.to_string_lossy().to_string()]);
        assert!(!created.exists(), "被撤销轮次新建的文件必须删除");
    }

    #[test]
    fn restore_skips_oversized_files_and_reports_degraded() {
        let root = temp_root("oversized");
        let workspace = root.join("ws");
        fs::create_dir_all(&workspace).expect("mkdir workspace");
        let big = workspace.join("big.bin");
        fs::write(&big, vec![b'x'; (MAX_SNAPSHOT_BYTES + 1) as usize]).expect("seed big");
        let store = RewindFileStore::new(root.join("snap"));

        store
            .record_pre_image("s1", "t1", &big)
            .expect("capture oversized");
        let outcome = store.restore_turns("s1", &["t1".to_string()]);
        assert!(outcome.restored.is_empty());
        assert_eq!(outcome.skipped.len(), 1);
        assert!(outcome.degraded(), "超限必须显式降级");
    }

    #[test]
    fn restore_of_unknown_session_is_a_no_op() {
        let root = temp_root("unknown");
        let store = RewindFileStore::new(root);
        let outcome = store.restore_turns("nope", &["t".to_string()]);
        assert_eq!(outcome, RewindRestoreOutcome::default());
        assert!(!outcome.degraded());
    }

    #[test]
    fn empty_ids_are_not_captured() {
        let root = temp_root("empty");
        let store = RewindFileStore::new(root.join("snap"));
        store
            .record_pre_image("", "t", Path::new("/tmp/whatever"))
            .expect("empty session is a no-op");
        assert!(!root.join("snap").exists(), "空会话键不得创建目录");
    }
}

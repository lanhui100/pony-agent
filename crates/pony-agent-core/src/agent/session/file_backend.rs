use super::backend::SessionBackend;
use super::store::{load_store_from_path, PersistedStore};
use std::fs;
use std::path::PathBuf;

pub struct FileSessionBackend {
    pub(super) storage_path: PathBuf,
    /// Wave-7 账本侧车连接：**单连接长期持有**（L3 ADVISORY：此前每次新建连接无 WAL/busy_timeout，
    /// 并发下易 `SQLITE_BUSY` 静默 fail-open，构成绕过面）。
    ledger: std::sync::Mutex<Option<rusqlite::Connection>>,
}

#[cfg(test)]
pub struct MemorySessionBackend {
    pub(super) attachment_root: PathBuf,
}

impl FileSessionBackend {
    pub fn new(storage_path: PathBuf) -> Self {
        Self {
            storage_path,
            ledger: std::sync::Mutex::new(None),
        }
    }
}

impl SessionBackend for FileSessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        eprintln!(
            "[pony-agent][session] loading sessions from {}",
            self.storage_path.display()
        );
        load_store_from_path(&self.storage_path)
    }

    fn save_store(&self, store: &PersistedStore) {
        let Some(parent) = self.storage_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }

        let Ok(serialized) = serde_json::to_string_pretty(store) else {
            return;
        };
        eprintln!(
            "[pony-agent][session] saving sessions to {}",
            self.storage_path.display()
        );
        let _ = fs::write(&self.storage_path, serialized);
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        self.storage_path
            .parent()
            .map(|parent| parent.join("attachments"))
    }

    // ---- Wave-7 B001：非 SQLite 后端（CLI 的 JSON 会话库）也要有真实账本 ----
    //
    // 背景：CLI 使用 `FileSessionBackend`（JSON 会话库）。若账本走 trait 默认实现（恒零），
    // 则 CLI 侧 enforcement 永远不可能触发 —— 等于留下「可绕过」面（违背 ADR-0048）。
    // 处置：在会话库同目录落一个**同 schema** 的 SQLite 账本侧车（`usage-ledger.sqlite`），
    // 复用 `usage_ledger` 的全部 SQL 与韧性逻辑（不引入第二份真源/第二套公式）。

    fn append_usage_ledger_entry(
        &self,
        entry: &crate::agent::usage_ledger::LedgerEntry,
    ) -> Result<(), String> {
        self.with_ledger(|conn| {
            crate::agent::usage_ledger::append_entry_resilient(conn, entry)
                .map_err(|error| error.to_string())
        })
    }

    fn usage_ledger_window_summary(
        &self,
        scope: &crate::agent::usage_ledger::LedgerScope,
    ) -> Result<crate::agent::usage_ledger::WindowSummary, String> {
        self.with_ledger(|conn| {
            crate::agent::usage_ledger::window_summary(conn, scope).map_err(|error| error.to_string())
        })
    }

    fn usage_ledger_day_summaries(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::agent::usage_ledger::DaySummary>, String> {
        self.with_ledger(|conn| {
            crate::agent::usage_ledger::day_summaries(conn, session_id)
                .map_err(|error| error.to_string())
        })
    }
}

impl FileSessionBackend {
    /// 账本侧车路径：与会话库同目录的 `usage-ledger.sqlite`。
    fn ledger_path(&self) -> PathBuf {
        self.storage_path
            .parent()
            .map(|parent| parent.join("usage-ledger.sqlite"))
            .unwrap_or_else(|| PathBuf::from("usage-ledger.sqlite"))
    }

    /// 在**单条持有连接**上执行账本操作（WAL + busy_timeout，与 `sqlite_session` 同口径）。
    ///
    /// 语义：首次调用时打开连接并设置 pragma、初始化 schema；后续复用。
    /// 失败仍返回 Err 由上层 fail-open 吸收（绝不阻断 turn），但并发争锁已被 busy_timeout 吸收，
    /// 不再把正常并发误判为「账本不可用」。
    fn with_ledger<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut slot = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            let path = self.ledger_path();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let conn = rusqlite::Connection::open(&path)
                .map_err(|error| format!("open usage ledger sidecar: {error}"))?;
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;",
            )
            .map_err(|error| format!("configure usage ledger sidecar: {error}"))?;
            crate::agent::usage_ledger::ensure_ledger_schema(&conn)
                .map_err(|error| error.to_string())?;
            *slot = Some(conn);
        }
        let conn = slot.as_ref().expect("ledger connection initialized");
        operation(conn)
    }
}

#[cfg(test)]
impl SessionBackend for MemorySessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        None
    }

    fn save_store(&self, _store: &PersistedStore) {}

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

#[cfg(test)]
impl Drop for MemorySessionBackend {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.attachment_root);
    }
}

use super::backend::SessionBackend;
use super::store::{load_store_from_path, PersistedStore};
use std::fs;
use std::path::PathBuf;

pub struct FileSessionBackend {
    pub(super) storage_path: PathBuf,
}

#[cfg(test)]
pub struct MemorySessionBackend {
    pub(super) attachment_root: PathBuf,
}

impl FileSessionBackend {
    pub fn new(storage_path: PathBuf) -> Self {
        Self { storage_path }
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
        let conn = self.ledger_connection()?;
        crate::agent::usage_ledger::append_entry_resilient(&conn, entry)
            .map_err(|error| error.to_string())
    }

    fn usage_ledger_window_summary(
        &self,
        scope: &crate::agent::usage_ledger::LedgerScope,
    ) -> Result<crate::agent::usage_ledger::WindowSummary, String> {
        let conn = self.ledger_connection()?;
        crate::agent::usage_ledger::window_summary(&conn, scope).map_err(|error| error.to_string())
    }

    fn usage_ledger_day_summaries(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::agent::usage_ledger::DaySummary>, String> {
        let conn = self.ledger_connection()?;
        crate::agent::usage_ledger::day_summaries(&conn, session_id)
            .map_err(|error| error.to_string())
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

    /// 打开（并按需初始化）账本连接。每次开关连接，避免跨线程长持有；
    /// 失败返回 Err 由上层 fail-open 吸收（绝不阻断 turn）。
    fn ledger_connection(&self) -> Result<rusqlite::Connection, String> {
        let path = self.ledger_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let conn = rusqlite::Connection::open(&path)
            .map_err(|error| format!("open usage ledger sidecar: {error}"))?;
        crate::agent::usage_ledger::ensure_ledger_schema(&conn)
            .map_err(|error| error.to_string())?;
        Ok(conn)
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

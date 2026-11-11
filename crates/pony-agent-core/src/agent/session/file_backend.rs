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

#[cfg(any(test, feature = "test-support"))]
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

    fn save_store(&self, store: &PersistedStore) -> Result<(), String> {
        let Some(parent) = self.storage_path.parent() else {
            return Err(format!(
                "session store path `{}` has no parent directory",
                self.storage_path.display()
            ));
        };
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create session store directory `{}` failed: {error}",
                parent.display()
            )
        })?;

        let serialized = serde_json::to_string_pretty(store)
            .map_err(|error| format!("serialize session store failed: {error}"))?;
        eprintln!(
            "[pony-agent][session] saving sessions to {}",
            self.storage_path.display()
        );
        // T2-3：原子替换 —— 同目录写临时文件、`sync_all` 落盘、再 rename 到目标路径。
        // 不能直接 `fs::write(&self.storage_path, ...)`：那是就地截断写，且会**跟随符号链接**
        // 写穿到链接指向的另一个文件；进程中途被杀/断电会留下半截 JSON，整个会话库报废。
        // 临时文件放在同目录，保证 rename 在同一文件系统内原子完成。
        let temp_path = sibling_temp_path(&self.storage_path);
        {
            use std::io::Write;
            // ADVISORY 修复：`File::create` 会截断已存在的同名文件（可被抢占/复用）；
            // `create_new(true)` 保证新建独占，重名即失败（重名本身就是不应发生的异常）。
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|error| {
                format!(
                    "create temporary session store `{}` failed: {error}",
                    temp_path.display()
                )
            })?;
            file.write_all(serialized.as_bytes()).map_err(|error| {
                let _ = fs::remove_file(&temp_path);
                format!(
                    "write temporary session store `{}` failed: {error}",
                    temp_path.display()
                )
            })?;
            file.sync_all().map_err(|error| {
                let _ = fs::remove_file(&temp_path);
                format!(
                    "flush temporary session store `{}` failed: {error}",
                    temp_path.display()
                )
            })?;
        }
        fs::rename(&temp_path, &self.storage_path).map_err(|error| {
            let _ = fs::remove_file(&temp_path);
            format!(
                "atomic replace of session store `{}` failed: {error}",
                self.storage_path.display()
            )
        })?;
        // ADVISORY 修复：rename 只保证"目录项替换"原子，目录项本身仍需 fsync 才能扛断电；
        // 否则崩溃后可能出现"文件内容在、名字没落盘"的窗口。
        sync_parent_directory(parent)
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

/// 对目录自身做 fsync，确保 `rename` 产生的目录项真正落盘（POSIX 语义）。
///
/// Windows 上无法以文件方式打开目录，此处按平台降级为 no-op（NTFS 的元数据顺序由
/// USN/日志保证，与 POSIX 的"必须 fsync 目录"不同）。
fn sync_parent_directory(parent: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let dir = fs::File::open(parent).map_err(|error| {
            format!(
                "open session store directory `{}` for fsync failed: {error}",
                parent.display()
            )
        })?;
        dir.sync_all().map_err(|error| {
            format!(
                "fsync session store directory `{}` failed: {error}",
                parent.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}

/// T2-3：同目录临时文件名（`<name>.tmp-<pid>-<nanos>`）。
///
/// 放在目标文件同一目录，保证 `fs::rename` 落在同一文件系统（跨文件系统 rename 会 `EXDEV`）。
fn sibling_temp_path(target: &std::path::Path) -> PathBuf {
    static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    // ADVISORY 修复：仅 pid+nanos 在同一进程内高频写盘时可能重名；再加一个进程内单调序号，
    // 配合 `create_new(true)` 保证「不可能复用/覆盖别人的临时文件」。
    let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "session-store".to_string());
    let temp_name = format!(".{file_name}.tmp-{}-{stamp}-{seq}", std::process::id());
    match target.parent() {
        Some(parent) => parent.join(temp_name),
        None => PathBuf::from(temp_name),
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

#[cfg(any(test, feature = "test-support"))]
impl SessionBackend for MemorySessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        None
    }

    fn save_store(&self, _store: &PersistedStore) -> Result<(), String> {
        Ok(())
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for MemorySessionBackend {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.attachment_root);
    }
}

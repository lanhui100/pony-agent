//! ADR 0107 §5 P1 的两个写路径加固件（独立成文件，理由同 `sqlite_session_commit.rs`：
//! 主文件已计入 `scripts/file-size-baseline.json` 行数额度，新增实作落这里不撑破棘轮）。
//!
//! 1. [`migrate_from_json`]：旧 JSON → SQLite 的**单事务幂等**迁移。原实现「先 COUNT 再整库写」
//!    在多进程冷启下会让二次迁移覆盖并发进程刚写的数据（实测可丢 `workspaces`）。
//! 2. [`begin_immediate_with_busy_retry`]：`BEGIN IMMEDIATE` + BUSY 有界退避，供
//!    `upsert_session` 在多进程并发写时不被 `database is locked` 一击致命。
//!
//! 本模块是 `sqlite_session` 的子模块（`#[path] mod p1;`），因此可以访问父模块的私有字段与
//! 私有方法（`legacy_json_path` / `write_store_tx` / `persisted_store_tail` 等）。

use rusqlite::Connection;

use crate::agent::session::{FileSessionBackend, SessionBackend};

use super::SqliteSessionBackend;

/// 旧 JSON 迁移完成标记的 `store_metadata` 键。
pub(crate) const MIGRATION_MARKER: &str = "legacy_json.migrated";

/// 旧 JSON → SQLite 一次性迁移（单事务幂等）。
///
/// 1. 快路径：标记已存在 ⇒ 直接返回，不抢写锁；
/// 2. 否则 `BEGIN IMMEDIATE`（立即拿写锁，让并发进程在 `busy_timeout` 内排队）；
/// 3. 事务内二次确认标记 / 索引是否已有数据；
/// 4. 需要迁移 ⇒ 同一事务内整库写入 + 写完成标记 + 提交（原子）。
pub(crate) fn migrate_from_json(
    backend: &SqliteSessionBackend,
    conn: &Connection,
) -> Result<(), String> {
    let Some(json_path) = &backend.legacy_json_path else {
        return Ok(());
    };
    if legacy_migration_done(conn, MIGRATION_MARKER) {
        return Ok(());
    }

    // **BEGIN IMMEDIATE**：迁移一开始就拿写锁，避免「先读后写」升级事务时撞
    // `database is locked` 被吞（仓内既有写事务同款写法见 `sqlite_session_commit.rs`）。
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("begin immediate tx: {e}"))?;
    // 事务内二次确认（另一个进程可能刚迁移完并提交）。
    if legacy_migration_done(&tx, MIGRATION_MARKER) {
        return Ok(());
    }
    let session_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap_or(0);
    if session_count > 0 {
        // 索引已有数据但无标记（历史库）：补标记，避免以后重复迁移。
        mark_legacy_migrated(&tx, MIGRATION_MARKER)?;
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        return Ok(());
    }

    eprintln!(
        "[pony-agent][session] migrating from {} to SQLite",
        json_path.display()
    );

    let legacy = FileSessionBackend::new(json_path.clone());
    let Some(store) = legacy.load_store() else {
        mark_legacy_migrated(&tx, MIGRATION_MARKER)?;
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        return Ok(());
    };

    backend.write_store_tx(&tx, &store, &std::collections::HashSet::new())?;
    mark_legacy_migrated(&tx, MIGRATION_MARKER)?;
    tx.commit().map_err(|e| format!("commit: {e}"))?;

    eprintln!(
        "[pony-agent][session] migration complete: {} sessions",
        store.sessions.len()
    );
    Ok(())
}

/// 迁移完成标记是否已存在。
fn legacy_migration_done(conn: &Connection, marker: &str) -> bool {
    conn.query_row(
        "SELECT value FROM store_metadata WHERE key = ?1",
        [marker],
        |row| row.get::<_, String>(0),
    )
    .is_ok()
}

/// 写入迁移完成标记（幂等）。
fn mark_legacy_migrated(conn: &Connection, marker: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, ?2)",
        rusqlite::params![marker, "1"],
    )
    .map(|_| ())
    .map_err(|e| format!("write migration marker: {e}"))
}

/// `BEGIN IMMEDIATE` + BUSY 有界退避（3 次，50/100/200ms）。
///
/// 只对 `SQLITE_BUSY`/`SQLITE_LOCKED` 重试；其他错误立即返回（不掩盖真实故障）。
pub(crate) fn begin_immediate_with_busy_retry(
    conn: &Connection,
) -> Result<rusqlite::Transaction<'_>, String> {
    let mut attempt = 0u32;
    loop {
        match rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate) {
            Ok(tx) => return Ok(tx),
            Err(error) => {
                let busy = matches!(
                    &error,
                    rusqlite::Error::SqliteFailure(inner, _)
                        if matches!(
                            inner.code,
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                );
                if !busy || attempt >= 2 {
                    return Err(error.to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50u64 << attempt));
                attempt += 1;
            }
        }
    }
}

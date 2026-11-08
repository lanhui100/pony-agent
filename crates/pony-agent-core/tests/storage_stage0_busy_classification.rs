//! ADR 0099 阶段 0 · 契约 5（判定函数面）：busy 判定必须按 `ErrorCode`，禁止字符串匹配。
//!
//! 现网缺陷（ADR 0099 阶段 0 第 5 条）：`sqlite_session.rs:2661` / `:2687` 用
//! `error.to_string().contains("busy")` 判定并发冲突。SQLite 的锁冲突错误消息是
//! **`database is locked`**（`ErrorCode::DatabaseBusy`/`DatabaseLocked`），消息里没有
//! "busy" 这个词 ⇒ 死分支，重试永不命中。
//!
//! 冻结契约（`.dev-team/contracts/stage0-write-model.md` §1）：
//! `pony_agent_core::agent::sqlite_session::is_busy_error(error: &rusqlite::Error) -> bool`，
//! 语义 = `matches!(error, Error::SqliteFailure(e, _) if e.code ∈ {DatabaseBusy, DatabaseLocked})`。
//!
//! ⚠️ 交付状态：**`is_busy_error` 尚未落码**（阶段 0 Executor 任务）。本文件因此当前
//! **编译不过**（`unresolved import ... is_busy_error`），属「依赖但尚不存在的 API」显式登记；
//! 单独成文件以免连带打断其余可运行用例。落码后本文件应直接由红转绿，断言无需改动。
//!
//! 行为面证据（真实 `database is locked` 下的重试）见
//! `storage_stage0_busy_locked_retry.rs`（现在可运行，交付时红）。
//!
//! 隔离：纯内存构造 `rusqlite::Error`，不触库。

use pony_agent_core::agent::sqlite_session::is_busy_error;
use rusqlite::{ffi, ErrorCode};

/// 构造一个带指定 `ErrorCode` 的 `rusqlite::Error::SqliteFailure`（真实错误形态）。
fn sqlite_failure(code: ErrorCode, extended_code: i32, message: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        ffi::Error {
            code,
            extended_code,
        },
        Some(message.to_string()),
    )
}

/// `DatabaseBusy` + 真实消息 `database is locked`（无 "busy" 字样）⇒ 必须判定为 busy。
///
/// 这正是现网死分支的反例：`contains("busy")` 会返回 false。
#[test]
fn database_busy_without_word_busy_is_classified_as_busy() {
    // Arrange
    let error = sqlite_failure(ErrorCode::DatabaseBusy, 5, "database is locked");

    // Act
    let busy = is_busy_error(&error);

    // Assert
    assert!(
        busy,
        "ADR 0099 阶段 0：`SQLITE_BUSY`(DatabaseBusy) + 消息 `database is locked` 必须判定为 busy；\
         按 `contains(\"busy\")` 字符串判定会得到 false（现网死分支）"
    );
}

/// `DatabaseLocked` + `database table is locked` ⇒ 必须判定为 busy。
#[test]
fn database_locked_is_classified_as_busy() {
    // Arrange
    let error = sqlite_failure(ErrorCode::DatabaseLocked, 6, "database table is locked");

    // Act
    let busy = is_busy_error(&error);

    // Assert
    assert!(
        busy,
        "ADR 0099 阶段 0：`SQLITE_LOCKED`(DatabaseLocked) 必须判定为 busy（同属可重试锁冲突）"
    );
}

/// 非锁冲突错误即使消息里带 "busy" 也**不得**判定为 busy（反向守门，专治字符串启发式）。
#[test]
fn non_busy_error_mentioning_busy_is_not_classified_as_busy() {
    // Arrange：消息里含 "busy"，但错误码是约束冲突（不可重试）。
    let error = sqlite_failure(
        ErrorCode::ConstraintViolation,
        19,
        "busy constraint violation (not a lock conflict)",
    );

    // Act
    let busy = is_busy_error(&error);

    // Assert
    assert!(
        !busy,
        "ADR 0099 阶段 0：判定必须只看 `ErrorCode`，消息文本里的 \"busy\" 不得影响判定\
         （禁止 `to_string().contains(\"busy\")`）"
    );
}

/// 非 SQLite 失败错误（参数名非法等）必须判定为非 busy。
#[test]
fn non_sqlite_failure_is_not_classified_as_busy() {
    // Arrange
    let error = rusqlite::Error::InvalidParameterName("busy".to_string());

    // Act
    let busy = is_busy_error(&error);

    // Assert
    assert!(
        !busy,
        "ADR 0099 阶段 0：非 `SqliteFailure` 错误一律不是 busy（不可重试）"
    );
}

//! 用量账本 `usage_ledger`（Wave-7 B001，契约 G1）。
//!
//! 语义总览：
//! - **Append-only + UPSERT 幂等**：主键 `(session_id, turn_id)`；冲突时只更新可变列，
//!   `recorded_at_ms` / `day_key` **首写冻结**（保证时序与天桶划分绝对单调稳定）。
//! - **未知模型语义**：`cost_usd = NULL` 且 `priced = 0`（**严禁**把「未计价」伪造成 0 成本）。
//! - **纯增量迁移**：只 `CREATE TABLE/INDEX IF NOT EXISTS`，绝不 bump `EVENT_SCHEMA_VERSION`。
//! - **故障隔离**：写失败**绝不阻塞 turn**；本模块提供「重试 1 次 + 结构化 warn + 内存 spool」。
//!
//! 本模块只做 SQL，不持有连接：调用方（`SqliteSessionBackend`）传入 `&rusqlite::Connection`。

use rusqlite::{params, Connection};
use std::fmt;

/// 账本表结构（纯增量 DDL；与契约 G1 逐字对齐）。
pub const LEDGER_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS usage_ledger (
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    day_key TEXT NOT NULL,
    provider_name TEXT NOT NULL,
    model TEXT NOT NULL,
    protocol TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cost_usd REAL,
    priced INTEGER NOT NULL,
    PRIMARY KEY (session_id, turn_id)
);
CREATE INDEX IF NOT EXISTS idx_usage_ledger_day_key ON usage_ledger(day_key);
CREATE INDEX IF NOT EXISTS idx_usage_ledger_session_id ON usage_ledger(session_id);
";

/// UPSERT 语句（契约 G1.2 逐字对齐）：只更新可变列，冻结 `recorded_at_ms` / `day_key`。
const UPSERT_SQL: &str = "
INSERT INTO usage_ledger (
    session_id, turn_id, recorded_at_ms, day_key, provider_name, model, protocol,
    input_tokens, output_tokens, cache_read_tokens, cost_usd, priced
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
ON CONFLICT(session_id, turn_id) DO UPDATE SET
    provider_name = excluded.provider_name,
    model = excluded.model,
    protocol = excluded.protocol,
    input_tokens = excluded.input_tokens,
    output_tokens = excluded.output_tokens,
    cache_read_tokens = excluded.cache_read_tokens,
    cost_usd = excluded.cost_usd,
    priced = excluded.priced;
";

/// 一条账本记录（一次 turn 一行）。
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerEntry {
    pub session_id: String,
    pub turn_id: String,
    pub recorded_at_ms: i64,
    /// `YYYY-MM-DD`（UTC 天桶；首写冻结）。
    pub day_key: String,
    pub provider_name: String,
    pub model: String,
    pub protocol: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    /// 未知模型必须为 `None`（写入 SQL NULL），**严禁** `Some(0.0)`。
    pub cost_usd: Option<f64>,
}

impl LedgerEntry {
    /// 构造一次计价记录（`priced = 1`）。
    #[allow(clippy::too_many_arguments)]
    pub fn priced(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        recorded_at_ms: i64,
        provider_name: impl Into<String>,
        model: impl Into<String>,
        protocol: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cost_usd: f64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            recorded_at_ms,
            day_key: day_key_for_epoch_ms(recorded_at_ms),
            provider_name: provider_name.into(),
            model: model.into(),
            protocol: protocol.into(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cost_usd: Some(cost_usd),
        }
    }

    /// 构造一次**未计价**记录（未知模型）：`cost_usd = None`（SQL NULL），`priced = 0`。
    #[allow(clippy::too_many_arguments)]
    pub fn unpriced(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        recorded_at_ms: i64,
        provider_name: impl Into<String>,
        model: impl Into<String>,
        protocol: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            recorded_at_ms,
            day_key: day_key_for_epoch_ms(recorded_at_ms),
            provider_name: provider_name.into(),
            model: model.into(),
            protocol: protocol.into(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cost_usd: None,
        }
    }

    /// 是否已计价（`cost_usd` 为 `Some`）。
    pub fn is_priced(&self) -> bool {
        self.cost_usd.is_some()
    }
}

/// 账本错误（只描述底层故障；调用方必须 fail-open 吞掉）。
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerError(pub String);

impl LedgerError {
    pub fn sql(context: &str, error: impl fmt::Display) -> Self {
        Self(format!("{context}: {error}"))
    }
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "usage_ledger: {}", self.0)
    }
}

/// 初始化账本表（幂等，纯增量）。
pub fn ensure_ledger_schema(conn: &Connection) -> Result<(), LedgerError> {
    conn.execute_batch(LEDGER_SCHEMA_SQL)
        .map_err(|error| LedgerError::sql("ensure schema", error))
}

/// 幂等写入一条记录（UPSERT；`recorded_at_ms` / `day_key` 首写冻结）。
pub fn append_entry(conn: &Connection, entry: &LedgerEntry) -> Result<(), LedgerError> {
    conn.execute(
        UPSERT_SQL,
        params![
            entry.session_id,
            entry.turn_id,
            entry.recorded_at_ms,
            entry.day_key,
            entry.provider_name,
            entry.model,
            entry.protocol,
            entry.input_tokens as i64,
            entry.output_tokens as i64,
            entry.cache_read_tokens as i64,
            entry.cost_usd,
            if entry.is_priced() { 1i64 } else { 0i64 },
        ],
    )
    .map_err(|error| LedgerError::sql("append", error))?;
    Ok(())
}

/// 内存 spool：最终写失败后暂存（有界），供后续异步重试；进程内最多 256 条。
static SPOOL: std::sync::LazyLock<std::sync::Mutex<Vec<LedgerEntry>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
const SPOOL_CAPACITY: usize = 256;

/// 带韧性包装的写入：失败重试 1 次；仍失败则 spool + 结构化 warn，并返回 Err 供调用方忽略。
///
/// 契约 G1.5：**绝不阻塞 turn 主流程**（本函数不 panic、不长时间阻塞；调用方必须忽略 Err）。
pub fn append_entry_resilient(conn: &Connection, entry: &LedgerEntry) -> Result<(), LedgerError> {
    match append_entry(conn, entry) {
        Ok(()) => Ok(()),
        Err(first) => match append_entry(conn, entry) {
            Ok(()) => Ok(()),
            Err(second) => {
                // 结构化 warn：只带稳定维度，不带金额/阈值（G5.3）。
                crate::agent::otel::log::log_json_with_fields(
                    "warn",
                    "usage ledger append failed after retry",
                    None,
                    &[
                        ("kind", serde_json::json!("ledger_append")),
                        ("session_id", serde_json::json!(entry.session_id)),
                        ("turn_id", serde_json::json!(entry.turn_id)),
                        ("error", serde_json::json!(format!("{first} | {second}"))),
                    ],
                );
                spool(entry.clone());
                Err(second)
            }
        },
    }
}

/// 把最终失败记录塞进有界内存 spool。
fn spool(entry: LedgerEntry) {
    let mut guard = SPOOL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.len() >= SPOOL_CAPACITY {
        guard.remove(0);
    }
    guard.push(entry);
}

/// 读取并清空 spool 中的待重试记录（诊断/测试用）。
pub fn drain_spool() -> Vec<LedgerEntry> {
    let mut guard = SPOOL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *guard)
}

/// 当前 spool 条数（诊断用）。
pub fn spool_len() -> usize {
    SPOOL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len()
}

/// 聚合范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerScope {
    /// 全部历史。
    All,
    /// 单个会话（跨天）。
    Session(String),
    /// 单个天桶（跨会话）。
    Day(String),
    /// 单会话 + 单天桶。
    SessionDay { session_id: String, day_key: String },
}

/// 窗口聚合结果（契约 G1 API / G4 形状）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSummary {
    /// 已计价记录成本之和（保留 6 位小数）。
    pub cost_usd: f64,
    /// 未计价 turn 数（`priced = 0`）。
    pub unpriced_turns: u64,
    /// 总 turn 数。
    pub turns: u64,
}

impl WindowSummary {
    /// 空窗口（空库/无记录；**禁止返回错误**，G4.2）。
    pub fn zero() -> Self {
        Self::default()
    }
}

/// 天桶聚合（G4 `days[]` 单元素形状）。
#[derive(Debug, Clone, PartialEq)]
pub struct DaySummary {
    pub day_key: String,
    pub cost_usd: f64,
    pub unpriced_turns: u64,
    pub turns: u64,
}

/// 按窗口聚合账本。
pub fn window_summary(conn: &Connection, scope: &LedgerScope) -> Result<WindowSummary, LedgerError> {
    let (where_clause, bindings): (&str, Vec<String>) = match scope {
        LedgerScope::All => ("1 = 1", Vec::new()),
        LedgerScope::Session(session_id) => ("session_id = ?1", vec![session_id.clone()]),
        LedgerScope::Day(day_key) => ("day_key = ?1", vec![day_key.clone()]),
        LedgerScope::SessionDay { session_id, day_key } => {
            ("session_id = ?1 AND day_key = ?2", vec![session_id.clone(), day_key.clone()])
        }
    };
    let sql = format!(
        "SELECT COALESCE(SUM(cost_usd), 0.0),
                COALESCE(SUM(CASE WHEN priced = 0 THEN 1 ELSE 0 END), 0),
                COUNT(*)
         FROM usage_ledger WHERE {where_clause}"
    );
    let params_ref: Vec<&dyn rusqlite::ToSql> =
        bindings.iter().map(|value| value as &dyn rusqlite::ToSql).collect();
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| LedgerError::sql("prepare window_summary", error))?;
    let row = statement
        .query_row(params_ref.as_slice(), |row| {
            Ok(WindowSummary {
                cost_usd: row.get::<_, f64>(0)?,
                unpriced_turns: row.get::<_, i64>(1)? as u64,
                turns: row.get::<_, i64>(2)? as u64,
            })
        })
        .map_err(|error| LedgerError::sql("query window_summary", error))?;
    Ok(WindowSummary {
        cost_usd: round_usd6(row.cost_usd),
        ..row
    })
}

/// 按天桶聚合（`days[]`，day_key 升序）；`session_id` 为 `Some` 时只统计该会话。
pub fn day_summaries(
    conn: &Connection,
    session_id: Option<&str>,
) -> Result<Vec<DaySummary>, LedgerError> {
    let (sql, params_ref): (&str, Vec<&dyn rusqlite::ToSql>) = match session_id {
        Some(_) => (
            "SELECT day_key,
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(CASE WHEN priced = 0 THEN 1 ELSE 0 END), 0),
                    COUNT(*)
             FROM usage_ledger WHERE session_id = ?1
             GROUP BY day_key ORDER BY day_key",
            Vec::new(),
        ),
        None => (
            "SELECT day_key,
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(CASE WHEN priced = 0 THEN 1 ELSE 0 END), 0),
                    COUNT(*)
             FROM usage_ledger
             GROUP BY day_key ORDER BY day_key",
            Vec::new(),
        ),
    };
    let _ = params_ref;
    let mut statement = conn
        .prepare(sql)
        .map_err(|error| LedgerError::sql("prepare day_summaries", error))?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(DaySummary {
            day_key: row.get::<_, String>(0)?,
            cost_usd: round_usd6(row.get::<_, f64>(1)?),
            unpriced_turns: row.get::<_, i64>(2)? as u64,
            turns: row.get::<_, i64>(3)? as u64,
        })
    };
    let rows = match session_id {
        Some(session_id) => statement
            .query_map(params![session_id], map_row)
            .map_err(|error| LedgerError::sql("query day_summaries", error))?
            .collect::<Result<Vec<_>, _>>(),
        None => statement
            .query_map([], map_row)
            .map_err(|error| LedgerError::sql("query day_summaries", error))?
            .collect::<Result<Vec<_>, _>>(),
    };
    rows.map_err(|error| LedgerError::sql("collect day_summaries", error))
}

/// USD 金额统一四舍五入到 6 位小数（G4.2 / NFR `cost_precision_decimals = 6`）。
pub fn round_usd6(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * 1_000_000.0).round() / 1_000_000.0
}

/// UTC 天桶键 `YYYY-MM-DD`。
pub fn day_key_for_epoch_ms(epoch_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(epoch_ms)
        .map(|instant| instant.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

/// 当前时钟（epoch 毫秒）。
pub fn now_epoch_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory db");
        ensure_ledger_schema(&conn).expect("schema");
        conn
    }

    fn priced_entry(turn: &str, cost: f64, recorded_at_ms: i64) -> LedgerEntry {
        LedgerEntry::priced(
            "sess-1",
            turn,
            recorded_at_ms,
            "openai",
            "gpt-4o",
            "openai-completions",
            1000,
            500,
            200,
            cost,
        )
    }

    /// G1 判据 1：同一 `(session, turn)` 连写 3 次 ⇒ 1 行，且首写的 `recorded_at_ms` / `day_key` 冻结。
    #[test]
    fn append_is_idempotent_and_freezes_time_columns() {
        let conn = memory_conn();
        let first = priced_entry("turn-1", 0.01, 1_700_000_000_000);
        append_entry(&conn, &first).expect("first append");

        let mut second = priced_entry("turn-1", 0.02, 1_700_100_000_000);
        second.model = "gpt-4.1".to_string();
        append_entry(&conn, &second).expect("second append");
        let mut third = priced_entry("turn-1", 0.03, 1_700_200_000_000);
        third.day_key = "1999-01-01".to_string();
        third.model = "gpt-4o-mini".to_string();
        append_entry(&conn, &third).expect("third append");

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_ledger", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 1, "幂等：同一 (session,turn) 只能有 1 行");

        let (recorded_at_ms, day_key, cost, model): (i64, String, f64, String) = conn
            .query_row(
                "SELECT recorded_at_ms, day_key, cost_usd, model FROM usage_ledger",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("select");
        assert_eq!(recorded_at_ms, 1_700_000_000_000, "recorded_at_ms 必须首写冻结");
        assert_eq!(day_key, "2023-11-14", "day_key 必须首写冻结");
        assert_eq!(cost, 0.03, "可变列必须被最后一次写入更新");
        assert_eq!(model, "gpt-4o-mini", "可变列必须被最后一次写入更新");
    }

    /// G1 判据 3：未知模型 ⇒ `cost_usd IS NULL` 且 `priced = 0`（严禁 0.0）。
    #[test]
    fn unpriced_entry_stores_null_not_zero() {
        let conn = memory_conn();
        let entry = LedgerEntry::unpriced(
            "sess-1",
            "turn-u",
            1_700_000_000_000,
            "openai",
            "unknown-model",
            "openai-completions",
            10,
            5,
            0,
        );
        append_entry(&conn, &entry).expect("append");

        let (is_null, priced): (bool, i64) = conn
            .query_row(
                "SELECT cost_usd IS NULL, priced FROM usage_ledger",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("select");
        assert!(is_null, "未知模型 cost_usd 必须为 SQL NULL");
        assert_eq!(priced, 0, "未知模型 priced 必须为 0");

        let summary = window_summary(&conn, &LedgerScope::All).expect("summary");
        assert_eq!(summary.unpriced_turns, 1);
        assert_eq!(summary.turns, 1);
        assert_eq!(summary.cost_usd, 0.0, "未计价记录不得贡献成本");
    }

    /// G1 判据 2：关闭后重开同一库，`SUM(cost_usd)` 与写入前一致。
    #[test]
    fn sum_survives_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "pony-ledger-reopen-{}-{}",
            std::process::id(),
            now_epoch_ms()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("ledger.db");

        {
            let conn = Connection::open(&path).expect("open");
            ensure_ledger_schema(&conn).expect("schema");
            append_entry(&conn, &priced_entry("turn-a", 0.0125, 1_700_000_000_000)).expect("a");
            append_entry(&conn, &priced_entry("turn-b", 0.0075, 1_700_000_000_000)).expect("b");
        }
        {
            let conn = Connection::open(&path).expect("reopen");
            ensure_ledger_schema(&conn).expect("schema again");
            let sum: f64 = conn
                .query_row("SELECT SUM(cost_usd) FROM usage_ledger", [], |row| row.get(0))
                .expect("sum");
            assert_eq!(round_usd6(sum), 0.02, "重开后 SUM 必须与写入值一致");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 跨天归桶：不同天写入分别落桶，窗口查询互不污染。
    #[test]
    fn entries_bucket_by_day_key() {
        let conn = memory_conn();
        let day1 = 1_700_000_000_000; // 2023-11-14
        let day2 = day1 + 86_400_000; // 2023-11-15
        append_entry(&conn, &priced_entry("turn-d1", 1.0, day1)).expect("d1");
        append_entry(&conn, &priced_entry("turn-d2", 2.0, day2)).expect("d2");

        let day1_summary =
            window_summary(&conn, &LedgerScope::Day(day_key_for_epoch_ms(day1))).expect("day1");
        assert_eq!(day1_summary.cost_usd, 1.0);
        assert_eq!(day1_summary.turns, 1);

        let days = day_summaries(&conn, Some("sess-1")).expect("days");
        assert_eq!(days.len(), 2, "必须按天分成两个桶");
        assert_eq!(days[0].day_key, "2023-11-14");
        assert_eq!(days[1].day_key, "2023-11-15");
        assert_eq!(round_usd6(days.iter().map(|day| day.cost_usd).sum()), 3.0);
    }

    /// 会话范围与 session+day 组合范围过滤正确。
    #[test]
    fn scope_filters_are_exact() {
        let conn = memory_conn();
        let now = 1_700_000_000_000i64;
        append_entry(&conn, &priced_entry("turn-1", 0.5, now)).expect("s1");
        append_entry(
            &conn,
            &LedgerEntry::priced(
                "sess-2", "turn-2", now, "anthropic", "claude-3-5-sonnet", "anthropic-messages", 1, 1, 0, 0.25,
            ),
        )
        .expect("s2");

        assert_eq!(window_summary(&conn, &LedgerScope::All).expect("all").turns, 2);
        assert_eq!(
            window_summary(&conn, &LedgerScope::Session("sess-2".to_string())).expect("s2").cost_usd,
            0.25
        );
        let session_day = LedgerScope::SessionDay {
            session_id: "sess-1".to_string(),
            day_key: day_key_for_epoch_ms(now),
        };
        assert_eq!(window_summary(&conn, &session_day).expect("sd").cost_usd, 0.5);
    }

    /// G1 判据 4：写故障被吸收（不 panic、不阻塞）+ spool 暂存 + 重试路径不抛。
    #[test]
    fn resilient_append_absorbs_failure_and_spools() {
        let conn = memory_conn();
        conn.execute("DROP TABLE usage_ledger", []).expect("drop table");
        let _ = drain_spool();

        let result = append_entry_resilient(&conn, &priced_entry("turn-fail", 0.1, 1_700_000_000_000));
        assert!(result.is_err(), "写失败必须返回 Err 供调用方忽略（绝不 panic）");
        assert_eq!(spool_len(), 1, "最终失败必须进入 spool 待重试");
        let _ = drain_spool();
    }

    /// 空库查询返回零值结构体（G4.2：禁止返回错误）。
    #[test]
    fn empty_ledger_summaries_are_zero() {
        let conn = memory_conn();
        let summary = window_summary(&conn, &LedgerScope::All).expect("empty summary ok");
        assert_eq!(summary, WindowSummary::zero());
        assert!(day_summaries(&conn, None).expect("empty days ok").is_empty());
    }
}

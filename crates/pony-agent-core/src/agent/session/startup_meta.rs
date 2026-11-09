//! 启动期「恢复目标」解析的**廉价 meta 查询**（P1 启动提速）。
//!
//! ## 为什么需要它
//!
//! `pony resume` 必须在进入 raw mode 之前知道「恢复哪个会话」，但旧实现为了拿一个
//! id 走的是 `SessionStore::new().list_sessions()`：整库装载 = 日志 fold + 索引投影
//! 补全 + trace/事件自愈。随后控制面又用 `SessionStore::new()` 装载**同一份库**。
//! 即「进 TUI 前把库完整读了两遍」——生产库 1.2 GB 时这就是启动卡顿的主因。
//!
//! 本模块只发**一条** `SELECT`（JSON 字段在 SQLite 侧投影，不反序列化整包 blob），
//! 不 fold 日志、不做自愈、不构造 `SessionStore`；真正的全量装载只留给控制面一次。
//!
//! ## 不变量
//!
//! - **只读**：`SQLITE_OPEN_READ_ONLY` + `PRAGMA query_only`，绝不写库、不抢单实例写锁；
//! - **可失败可降级**：任何一步失败（库损坏 / 无 JSON1 / 表缺失）都返回 `Err`，调用方
//!   退回 `SessionStore::new().list_sessions()`——慢但不改语义；
//! - **口径对齐**：过滤/排序必须与 `SessionStore::list_sessions()` 一致（可持久化过滤 +
//!   `updated_at_ms DESC, conversation_id ASC`），否则默认恢复目标会漂移。
//!
//! 过滤口径说明：`session_is_persistable()` 在装载后的内存态上判定（history/trace/nodes
//! 非空）。轻量索引行（停写 blob 后）内容数组全空，只剩索引字段，因此这里同时接受
//! 「`turnCount > 0`」作为等价判据——轻量行存在日志就一定有轮次，二者在生产形态下同真。

use rusqlite::Connection;
use std::path::Path;

/// 一条会话 meta 行（只剩启动期解析真正消费的字段）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMetaRow {
    pub conversation_id: String,
    /// 生效标题（`titleOverride` 优先，退回行上的派生 title）。
    pub title: String,
    pub updated_at_ms: u64,
}

/// 单条 SQL：字段在 SQLite 侧投影 + 与 `list_sessions()` 同口径的过滤与排序。
///
/// `json_valid` 先挡住坏行（一行坏 JSON 不应让整条快速路径失效）；`json_extract`
/// 取不到键时返回 NULL，由 `COALESCE` 兜底。
const META_SQL: &str = r#"
SELECT conversation_id,
       COALESCE(json_extract(session_data, '$.titleOverride'), title, '') AS effective_title,
       updated_at_ms
  FROM sessions
 WHERE json_valid(session_data)
   AND (
        COALESCE(json_extract(session_data, '$.turnCount'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.history'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.turnTraceHistory'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.longTermMemoryEntries'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.memoryWriteEvidence'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.memoryWriteHookTraceRecords'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.historyStateEvidence'), 0) > 0
     OR COALESCE(json_array_length(session_data, '$.historyNodes'), 0) > 0
   )
 ORDER BY updated_at_ms DESC, conversation_id ASC
"#;

/// 读会话 meta（只读连接，单查询）。库文件不存在 ⇒ 空列表（等价于「没有可恢复会话」）。
pub fn load_session_meta_rows(db_path: &Path) -> Result<Vec<SessionMetaRow>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open session db read-only ({}): {error}", db_path.display()))?;
    // 启动期不排队：读锁冲突由调用方降级到整库装载，绝不在这里把 UI 启动卡住。
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));
    let _ = conn.execute_batch("PRAGMA query_only=ON;");
    let mut statement = conn
        .prepare(META_SQL)
        .map_err(|error| format!("prepare session meta query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let updated_at_ms: i64 = row.get(2)?;
            Ok(SessionMetaRow {
                conversation_id: row.get(0)?,
                title: row.get(1)?,
                updated_at_ms: updated_at_ms.max(0) as u64,
            })
        })
        .map_err(|error| format!("query session meta rows: {error}"))?;
    let mut metas = Vec::new();
    for row in rows {
        metas.push(row.map_err(|error| format!("read session meta row: {error}"))?);
    }
    Ok(metas)
}

/// 「当前没有可恢复的会话」的唯一文案（CLI 退出码 2 的输出，测试可逐字断言）。
pub const NO_RESUMABLE_SESSION_MESSAGE: &str =
    "当前没有可恢复的会话。请先运行 `pony` 开始一个新会话。";

/// `pony resume [<id>]` 的恢复目标解析（廉价路径优先，失败降级整库装载）。
///
/// 返回 `(session_id, existed)`：显式 id 一律接受（存在性仅作 UI 说明）；
/// 缺省取最近的主会话（队友/子代理会话不抢默认续接）。
pub fn resolve_resume_target(requested: Option<&str>) -> Result<(String, bool), String> {
    let db_path = crate::agent::sqlite_session::default_sqlite_path();
    let legacy_json = db_path.parent().map(|dir| dir.join("sessions.json"));
    let has_legacy_json = legacy_json.as_deref().is_some_and(|path| path.exists());
    let degraded = |reason: String| {
        // 快速路径不适用时退回唯一真值源（整库装载）：慢，但语义与旧实现逐字一致。
        eprintln!("[pony-tui] resume_meta_degraded: {reason} — 退回整库装载解析恢复目标");
        let overviews = super::store::SessionStore::new().list_sessions();
        resolve_from_overviews(&overviews, requested)
    };
    match load_session_meta_rows(&db_path) {
        // legacy `sessions.json` 尚未迁移时库里可能还没有行：交给整库装载触发迁移。
        Ok(metas) if metas.is_empty() && has_legacy_json => degraded(
            "会话库为空且检测到 legacy sessions.json（待迁移）".to_string(),
        ),
        Ok(metas) => resolve_from_meta_rows(&metas, requested),
        Err(error) => degraded(error),
    }
}

/// 纯函数：meta 行 → 恢复目标（meta 查询与降级路径共用同一套判定）。
pub fn resolve_from_meta_rows(
    rows: &[SessionMetaRow],
    requested: Option<&str>,
) -> Result<(String, bool), String> {
    match requested {
        // 显式 id 一律接受（可能来自退出提示/诊断/队友会话）；存在性按 meta 行判定。
        Some(id) => Ok((
            id.to_string(),
            rows.iter().any(|row| row.conversation_id == id),
        )),
        None => rows
            .iter()
            .find(|row| super::is_main_session_id(&row.conversation_id))
            .map(|row| (row.conversation_id.clone(), true))
            .ok_or_else(|| NO_RESUMABLE_SESSION_MESSAGE.to_string()),
    }
}

/// 降级路径：整库装载后的会话摘要 → 恢复目标（判定与 [`resolve_from_meta_rows`] 同源）。
fn resolve_from_overviews(
    overviews: &[super::types::SessionOverview],
    requested: Option<&str>,
) -> Result<(String, bool), String> {
    match requested {
        Some(id) => Ok((
            id.to_string(),
            overviews
                .iter()
                .any(|overview| overview.conversation_id == id),
        )),
        None => overviews
            .iter()
            .find(|overview| super::is_main_session_id(&overview.conversation_id))
            .map(|overview| (overview.conversation_id.clone(), true))
            .ok_or_else(|| NO_RESUMABLE_SESSION_MESSAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn temp_db_path(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "pony-startup-meta-{tag}-{}-{unique}.db",
            std::process::id()
        ))
    }

    fn insert_row(conn: &Connection, id: &str, title: &str, updated_at_ms: i64, payload: &str) {
        conn.execute(
            "INSERT INTO sessions (conversation_id, title, updated_at_ms, session_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, title, updated_at_ms, payload],
        )
        .expect("insert session row");
    }

    fn seed_db(path: &Path) {
        let conn = Connection::open(path).expect("open write db");
        conn.execute_batch(
            "CREATE TABLE sessions (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                session_data TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("create schema");
        // 轻量索引行（生产形态）：内容数组全空，只有 turnCount 等索引字段。
        insert_row(
            &conn,
            "pa_recent",
            "派生标题",
            3000,
            r#"{"conversationId":"pa_recent","title":"派生标题","summary":"","history":[],"turnCount":4,"updatedAtMs":3000,"titleOverride":null,"__index_only":true}"#,
        );
        insert_row(
            &conn,
            "pa_tie_a",
            "并列 A",
            2000,
            r#"{"conversationId":"pa_tie_a","title":"并列 A","summary":"","history":[],"turnCount":2,"updatedAtMs":2000,"titleOverride":"改名后的标题","__index_only":true}"#,
        );
        insert_row(
            &conn,
            "pa_tie_b",
            "并列 B",
            2000,
            r#"{"conversationId":"pa_tie_b","title":"并列 B","summary":"","history":[],"turnCount":1,"updatedAtMs":2000,"__index_only":true}"#,
        );
        // 零轮空壳：`list_sessions()` 不认它，meta 查询也必须过滤掉。
        insert_row(
            &conn,
            "pa_shell",
            "空壳",
            4000,
            r#"{"conversationId":"pa_shell","title":"空壳","summary":"","history":[],"turnCount":0,"updatedAtMs":4000}"#,
        );
        // 坏 JSON：`json_valid` 挡住，不能让整条快速路径失效。
        insert_row(&conn, "pa_broken", "坏行", 9000, "{not-json");
    }

    #[test]
    fn meta_rows_follow_list_sessions_order_and_filters() {
        let path = temp_db_path("order");
        seed_db(&path);
        let metas = load_session_meta_rows(&path).expect("meta query should succeed");
        let ids: Vec<&str> = metas.iter().map(|row| row.conversation_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["pa_recent", "pa_tie_a", "pa_tie_b"],
            "按 updated_at_ms DESC / conversation_id ASC 排序，且过滤零轮空壳与坏 JSON 行"
        );
        assert_eq!(
            metas[1].title, "改名后的标题",
            "titleOverride 必须压过行上的派生 title"
        );
        assert_eq!(metas[1].updated_at_ms, 2000);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn explicit_resume_id_is_accepted_and_existence_comes_from_meta_rows() {
        let metas = vec![SessionMetaRow {
            conversation_id: "pa_real".to_string(),
            title: "标题".to_string(),
            updated_at_ms: 10,
        }];
        assert_eq!(
            resolve_from_meta_rows(&metas, Some("pa_real")).unwrap(),
            ("pa_real".to_string(), true)
        );
        // 未落库的 id 也接受（退出提示复制粘贴场景），只是 existed=false。
        assert_eq!(
            resolve_from_meta_rows(&metas, Some("pa_ghost")).unwrap(),
            ("pa_ghost".to_string(), false)
        );
    }

    #[test]
    fn default_target_skips_teammate_and_subagent_sessions() {
        let metas = vec![
            SessionMetaRow {
                conversation_id: "teammate-abc".to_string(),
                title: "队友".to_string(),
                updated_at_ms: 30,
            },
            SessionMetaRow {
                conversation_id: "subagent-def".to_string(),
                title: "子代理".to_string(),
                updated_at_ms: 20,
            },
            SessionMetaRow {
                conversation_id: "pa_main".to_string(),
                title: "主会话".to_string(),
                updated_at_ms: 10,
            },
        ];
        assert_eq!(
            resolve_from_meta_rows(&metas, None).unwrap(),
            ("pa_main".to_string(), true)
        );
    }

    #[test]
    fn empty_meta_rows_report_no_resumable_session() {
        let error = resolve_from_meta_rows(&[], None).expect_err("空库必须报错而非静默开新会话");
        assert_eq!(error, NO_RESUMABLE_SESSION_MESSAGE);
    }

    #[test]
    fn missing_db_file_is_an_empty_meta_list() {
        let path = temp_db_path("missing");
        assert!(!path.exists());
        assert_eq!(load_session_meta_rows(&path).unwrap(), Vec::new());
    }
}

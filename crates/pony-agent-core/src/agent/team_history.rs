use crate::agent::orchestration::{TeamTaskItem, TeammateMember};
use crate::agent::session::{SessionOverview, SessionStore};
use crate::agent::turn_event::TurnEvent;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TeamHistoryTaskRow {
    pub task: TeamTaskItem,
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub is_historical: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TeamHistoryMemberRow {
    pub member: TeammateMember,
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub is_historical: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TeamHistorySession {
    pub session_id: String,
    pub title: String,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub in_sessions_table: bool,
    #[serde(default)]
    pub in_turn_events: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TeamHistoryDiagnostics {
    pub skipped_payload_count: u64,
    pub skipped_session_count: u64,
    pub db_unavailable: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TeamHistorySnapshot {
    pub tasks: Vec<TeamHistoryTaskRow>,
    pub members: Vec<TeamHistoryMemberRow>,
    pub sessions: Vec<TeamHistorySession>,
    pub diagnostics: TeamHistoryDiagnostics,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TeamHistoryRefreshRequest {
    pub db_path: String,
    pub trace_id: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum TeamHistoryRefreshStatus {
    #[default]
    Loading,
    Ready,
    Offline,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TeamHistoryCacheState {
    #[serde(default)]
    pub status: TeamHistoryRefreshStatus,
    #[serde(default)]
    pub snapshot: TeamHistorySnapshot,
}

// 注意（2026-11-07 wave-release-unblock）：本模块除 `team_history_snapshot_from_events` 外仍是 `todo!()` 空桩，
// 生产路径无调用点（已 grep 全仓确认），仅 `tests/team_history_acceptance.rs` 引用；为让 `-D warnings`
// （CI/发版同口径）转绿，这里把未使用形参统一加 `_` 前缀——**纯告警修复，零语义改动**，实现仍归原 owner。
/// Read-only schema probe: no DDL, no version backfill, no `ensure_schema`.
pub fn team_history_schema_probe(_db_path: &Path) -> bool {
    todo!("NotImplemented")
}

/// Full session discovery union: `sessions` table + distinct `turn_events.session_id`.
pub fn team_history_discover_sessions(_db_path: &Path) -> Vec<TeamHistorySession> {
    todo!("NotImplemented")
}

/// Production aggregate entry used by the async refresh job; opens `?mode=ro`.
pub fn team_history_snapshot_from_db_path(_db_path: &Path) -> TeamHistorySnapshot {
    todo!("NotImplemented")
}

/// Compatibility seam over an already-loaded store; not used to open SQLite.
pub fn team_history_snapshot_from_store(_store: &SessionStore) -> TeamHistorySnapshot {
    todo!("NotImplemented")
}

/// 可注入重建入口：显式传入会话清单，并以 session_id 回调读取事件流；不触全局单例/生产 DB。
pub fn team_history_snapshot_from_events<F>(
    sessions: &[SessionOverview],
    load_events: F,
) -> TeamHistorySnapshot
where
    F: Fn(&str) -> Vec<(u64, String, TurnEvent)>,
{
    let mut snapshot = TeamHistorySnapshot::default();
    for session in sessions {
        let events = load_events(&session.conversation_id);
        for (_seq, _type, event) in events {
            if let TurnEvent::ToolResult { result: Some(res_str), .. } = event {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&res_str) {
                    let tool = val.get("tool").and_then(|t| t.as_str()).unwrap_or("");
                    match tool {
                        "team_task_create" | "team_task_update" => {
                            if let Some(task_val) = val.get("result") {
                                if let Ok(task) = serde_json::from_value::<TeamTaskItem>(task_val.clone()) {
                                    snapshot.tasks.push(TeamHistoryTaskRow {
                                        task,
                                        source_session_id: Some(session.conversation_id.clone()),
                                        is_historical: true,
                                    });
                                }
                            }
                        }
                        "spawn_teammate" => {
                            if let Some(member_val) = val.get("result").and_then(|r| r.get("member")) {
                                if let Ok(member) = serde_json::from_value::<TeammateMember>(member_val.clone()) {
                                    snapshot.members.push(TeamHistoryMemberRow {
                                        member,
                                        source_session_id: Some(session.conversation_id.clone()),
                                        is_historical: true,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    snapshot
}

/// 可注入 live 合并：live 参数由调用方显式获取，便于同 id 覆盖与并行测试隔离。
pub fn merge_team_history(
    _history: &TeamHistorySnapshot,
    _live_tasks: &[TeamTaskItem],
    _live_agents: &[TeammateMember],
) -> TeamHistorySnapshot {
    todo!("NotImplemented")
}

pub fn team_history_tasks_for_tui(
    _snapshot: &TeamHistorySnapshot,
    _live: &[TeamTaskItem],
) -> Vec<TeamHistoryTaskRow> {
    todo!("NotImplemented")
}

pub fn team_history_members_for_tui(
    _snapshot: &TeamHistorySnapshot,
    _live: &[TeammateMember],
) -> Vec<TeamHistoryMemberRow> {
    todo!("NotImplemented")
}

pub fn team_history_cache_from_job_result(
    _result: Option<TeamHistorySnapshot>,
) -> TeamHistoryCacheState {
    todo!("NotImplemented")
}

pub fn team_history_task_action_allowed(_row: &TeamHistoryTaskRow) -> bool {
    todo!("NotImplemented")
}

pub fn team_history_member_action_allowed(_row: &TeamHistoryMemberRow) -> bool {
    todo!("NotImplemented")
}

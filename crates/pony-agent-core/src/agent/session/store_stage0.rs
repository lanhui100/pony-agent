//! ADR 0099 阶段 0：`SessionStore` 侧的**提交边界编排**（CAS 重试 / 版本前进 / 快照构造）。
//!
//! 契约真源：`.dev-team/contracts/stage0-write-model.md` §3（I1–I3）与 §5（E1/E4）。
//!
//! 为什么独立成文件：`session/store.rs` 与 `agent/sqlite_session.rs` 已计入
//! `scripts/file-size-baseline.json` 的**文件行数棘轮**（净值只许减、不许增），本阶段新增
//! 的编排逻辑落在本文件（无基线条目、远低于 3000 行阈值），`store.rs` 里 18 个
//! `save_to_backend` 全库入口只留一行调用。模块声明见 `store.rs` 顶部
//! `#[path = "store_stage0.rs"] mod stage0;`。

use super::{
    collect_trace_union, session_is_persistable, session_state_for_backend, PersistedStore,
    SessionMap, SessionStore,
};
use crate::agent::session::{
    CommitOutcome, SeparateTraceTableMode, SessionVersion, SessionVersionMap, TraceMigrationState,
};

impl SessionStore {
    /// PA-088：Authoritative 会话的 trace 表写入（用全量 Union，须在 blob 剥离 trace 前
    /// 执行；仅写顶层会让 >24 轮会话的旧节点 refs 在重启 materialize 时解析不到）。
    pub(super) fn write_trace_tables(&self, sessions: &SessionMap) {
        if !matches!(
            self.backend.trace_storage_mode(),
            SeparateTraceTableMode::WriteSeparate
        ) {
            return;
        }
        for (session_id, session) in sessions {
            if matches!(
                session.trace_migration_state,
                TraceMigrationState::TraceTableAuthoritative
            ) {
                let union = collect_trace_union(session);
                let _ = self.backend.replace_session_traces(session_id, &union);
            }
        }
    }

    /// 由内存会话集构造提交快照（只留可持久化会话；携带版本侧表）。
    pub(super) fn to_persisted_store(&self, sessions: &SessionMap) -> PersistedStore {
        let trace_mode = self.backend.trace_storage_mode();
        PersistedStore {
            sessions: sessions
                .iter()
                .filter(|(_, session)| session_is_persistable(session))
                .map(|(session_id, session)| {
                    (
                        session_id.clone(),
                        session_state_for_backend(session, trace_mode),
                    )
                })
                .collect::<SessionMap>(),
            attachment_assets: self.attachment_assets.clone(),
            session_attachment_index: self.session_attachment_index.clone(),
            mcp_source_snapshots: self.mcp_source_snapshots.clone(),
            skill_source_snapshots: self.skill_source_snapshots.clone(),
            workspaces: self.workspaces.clone(),
            path_authorizations: self.path_authorizations.entries(),
            versions: self.versions_of(sessions),
        }
    }

    /// 期望版本 = 本实例装载/上次提交后已知的版本。装载时见过、现在库里没有的会话
    /// ⇒ 提交时判为墓碑并跳过（I3：不复活），而不是被当成新会话 INSERT。
    fn versions_of(&self, sessions: &SessionMap) -> SessionVersionMap {
        let Ok(known) = self.versions.lock() else {
            return SessionVersionMap::new();
        };
        sessions
            .keys()
            .filter_map(|session_id| {
                known
                    .get(session_id)
                    .map(|version| (session_id.clone(), *version))
            })
            .collect()
    }

    /// ADR 0099 阶段 0（I1/I2/I3）：单一提交边界编排。
    ///
    /// - `Committed` ⇒ 版本前进，本次落盘结束；
    /// - `Unsupported`（File / Memory / 计数代理）⇒ 回退 `save_store`，保持旧语义；
    /// - `StaleVersion` ⇒ 重载库内最新版本后**有界重试**（≤3 次，50→200ms 退避）；
    /// - `Failed` / 重试耗尽 ⇒ fail loud（结构化日志，不静默成功）。
    pub(super) fn commit_store(&self, sessions: &SessionMap, label: &str) {
        let mut store = self.to_persisted_store(sessions);
        for attempt in 0..3u32 {
            match self.backend.commit_boundary(&store) {
                CommitOutcome::Committed => {
                    self.advance_versions(&store);
                    return;
                }
                CommitOutcome::Unsupported => {
                    if let Err(error) = self.backend.save_store(&store) {
                        eprintln!(
                            "[pony-agent][session] {label} 失败：会话库未落盘（数据可能丢失）：{error}"
                        );
                    }
                    return;
                }
                CommitOutcome::StaleVersion { expected, actual } => {
                    eprintln!(
                        "[pony-agent][session] {label} CAS 冲突 expected={expected} actual={actual}，\
                         重载版本后第 {} 次重试",
                        attempt + 1
                    );
                    std::thread::sleep(std::time::Duration::from_millis(50u64 << attempt.min(2)));
                    let Some(fresh) = self.backend.load_store() else {
                        break;
                    };
                    store.versions = merged_versions(&store.versions, &fresh.versions);
                }
                CommitOutcome::Failed => {
                    eprintln!(
                        "[pony-agent][session] {label} 失败：提交边界返回 Failed（数据可能丢失）"
                    );
                    return;
                }
            }
        }
        eprintln!(
            "[pony-agent][session] {label} 放弃：CAS 冲突重试 3 次仍失败（未静默成功，请排查并发写）"
        );
    }

    /// 提交成功后版本前进：已装载的会话 +1；本实例新建的会话（提交时建行 version=0）记 0。
    fn advance_versions(&self, store: &PersistedStore) {
        let Ok(mut known) = self.versions.lock() else {
            return;
        };
        for session_id in store.sessions.keys() {
            let next = known
                .get(session_id)
                .map(|version| version.0 + 1)
                .unwrap_or(0);
            known.insert(session_id.clone(), SessionVersion(next));
        }
    }
}

/// ADR 0099 阶段 0（I2/I3）：CAS 重试时的版本合并。
///
/// 库内最新版本优先；快照里存在但**库里已无行**的会话保留旧期望版本 ⇒ 再次提交时被
/// 判为墓碑并跳过（而不是被当成新会话 INSERT 回去，I3 不复活）。
fn merged_versions(snapshot: &SessionVersionMap, fresh: &SessionVersionMap) -> SessionVersionMap {
    let mut merged = fresh.clone();
    for (session_id, version) in snapshot {
        if !merged.contains_key(session_id) {
            merged.insert(session_id.clone(), *version);
        }
    }
    merged
}

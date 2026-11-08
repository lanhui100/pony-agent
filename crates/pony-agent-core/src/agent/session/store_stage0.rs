//! ADR 0099 阶段 0：`SessionStore` 侧的**提交边界编排**（CAS 收口 / 版本前进 / 快照构造）。
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
    CommitOutcome, SeparateTraceTableMode, SessionBackendMutationResult, SessionVersion,
    SessionVersionMap, TraceMigrationState,
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
                // 失败不得静默：authoritative 会话的 blob 已剥离 trace，表写不进去就等于
                // 永久丢 trace（可观测、可排障，而不是 `let _ =` 吞掉）。
                if let SessionBackendMutationResult::Failed =
                    self.backend.replace_session_traces(session_id, &union)
                {
                    eprintln!(
                        "[pony-agent][session] trace 表写入失败（session={session_id}）：\
                         blob 已剥离 trace，权威 trace 可能永久丢失"
                    );
                }
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

    /// ADR 0099 阶段 0（I1/I2/I3）：单一提交边界编排（**StaleVersion 不可重试**）。
    ///
    /// - `Committed` ⇒ 版本前进，本次落盘结束；
    /// - `Unsupported`（File / Memory / 计数代理）⇒ 回退 `save_store`，保持旧语义；
    /// - `StaleVersion` ⇒ **放弃本次整包提交**并 fail loud：契约把版本冲突列为不可重试
    ///   结果——重投旧快照（哪怕换上新版本号）就是 LWW 覆盖，比不重试更糟。需要重试时必须
    ///   由调用方**重新装载最新状态**后再构造新的 `PersistedStore`；
    /// - `Failed` ⇒ fail loud（不静默成功）。
    pub(super) fn commit_store(&self, sessions: &SessionMap, label: &str) {
        let store = self.to_persisted_store(sessions);
        match self.backend.commit_boundary(&store) {
            CommitOutcome::Committed => self.advance_versions(&store),
            CommitOutcome::Unsupported => {
                if let Err(error) = self.backend.save_store(&store) {
                    eprintln!(
                        "[pony-agent][session] {label} 失败：会话库未落盘（数据可能丢失）：{error}"
                    );
                }
            }
            CommitOutcome::StaleVersion { expected, actual } => eprintln!(
                "[pony-agent][session] {label} 放弃本次整包提交：commit_boundary_stale_version \
                 （不可重试）expected={expected} actual={actual} sessions={}；\
                 旧快照不得覆盖库内新版本，须重新装载最新状态后重投",
                store.sessions.len()
            ),
            CommitOutcome::Failed => eprintln!(
                "[pony-agent][session] {label} 失败：commit_boundary_failed（数据可能丢失）"
            ),
        }
    }

    /// 单行写成功后**同步 CAS 期望版本**（取库内真实值）。
    ///
    /// 单行写路径（`upsert_session` / `persist_session_with_trace_mutation`）都会让
    /// `sessions.version` +1；缓存若不跟上，下一次整包提交会被自己刚写的那一版判成
    /// `StaleVersion`（"重启→聊天→建工作区必撞 stale" 的根因）。这里只在自己写入成功后
    /// 刷新，因此**不会**掩盖别的实例造成的过期快照。
    pub(super) fn note_version_from_backend(&self, session_id: &str) {
        let Some(version) = self.backend.session_version(session_id) else {
            return;
        };
        if let Ok(mut known) = self.versions.lock() {
            known.insert(session_id.to_string(), SessionVersion(version));
        }
    }

    /// 提交成功后刷新 CAS 期望版本——**取库内实际写入值**（`session_version` PK 读）。
    ///
    /// 不用 `known + 1` 推算：那会在"本次提交写了别的会话/别的实例也写过"时恒定差一，
    /// 让下一次整包提交永远 `StaleVersion`（L3-A 复现的 off-by-one）。
    /// 墓碑会话（本次跳过、行已不在）返回 `None` ⇒ 保留旧期望版本，下次继续判墓碑（I3）。
    fn advance_versions(&self, store: &PersistedStore) {
        let Ok(mut known) = self.versions.lock() else {
            return;
        };
        for session_id in store.sessions.keys() {
            if let Some(version) = self.backend.session_version(session_id) {
                known.insert(session_id.clone(), SessionVersion(version));
            }
        }
    }
}

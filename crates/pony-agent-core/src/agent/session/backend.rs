use super::store::PersistedStore;
use super::types::{
    AttachmentAssetMap, HistoryBranch, HistoryCursor, HistoryNode, SessionAttachmentIndex,
    SessionState, TurnHistoryMessage, TurnTraceRecord,
};
use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};
use crate::agent::hooks::HookTraceRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceMigrationState {
    #[default]
    LegacyBlob,
    DualWrite,
    TraceTableAuthoritative,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeparateTraceTableMode {
    #[default]
    Off,
    DualWrite,
    WriteSeparate,
}

#[derive(Clone, Debug)]
pub enum SessionBackendTraceLoadResult {
    Unsupported,
    Loaded(Vec<TurnTraceRecord>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionBackendMutationResult {
    Unsupported,
    NotFound,
    Succeeded,
    Failed,
}
#[derive(Clone, Debug)]
pub enum SessionTraceMutation {
    ReplaceAll {
        traces: Vec<TurnTraceRecord>,
    },
    UpsertOne {
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTerminalEvent {
        turn_id: String,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
        updated_at: u64,
        /// PA-094：trace 缓存行水位（事件日志水位，None 时保持原值）。
        event_watermark: Option<u64>,
    },
    AppendHookRecords {
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
}

/// PA-089 阶段 3：规范化双写命令（spec 3.2/3.7 定稿）。
/// 每个命令由 SqliteSessionBackend 在**统一事务**内同时写 blob（旧 sessions 表）
/// 与 normalized_* 表。`epoch` 用于迁移 barrier——旧 epoch 命令在 materialize 后拒绝。
#[derive(Clone, Debug)]
pub enum PersistCommand {
    AppendMessage {
        epoch: u64,
        session_id: String,
        message: TurnHistoryMessage,
        ordinal: usize,
    },
    UpsertTurn {
        epoch: u64,
        session_id: String,
        turn: TurnStateRecord,
    },
    AppendTrace {
        epoch: u64,
        session_id: String,
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTraceTerminal {
        epoch: u64,
        session_id: String,
        turn_id: String,
        terminal_patch: TraceTerminalPatch,
    },
    AppendHookRecords {
        epoch: u64,
        session_id: String,
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
    UpdateHistoryNode {
        epoch: u64,
        session_id: String,
        node: HistoryNode,
    },
    UpdateCursor {
        epoch: u64,
        session_id: String,
        cursor: HistoryCursor,
    },
    UpdateBranch {
        epoch: u64,
        session_id: String,
        branch: HistoryBranch,
    },
    UpdateSessionMeta {
        epoch: u64,
        session_id: String,
        meta_patch: SessionMetaPatch,
    },
    RemoveSession {
        epoch: u64,
        session_id: String,
    },
    /// PA-091：turn 终态事件批次落盘（与快照写入同事务）。
    FlushEvents {
        epoch: u64,
        session_id: String,
        turn_id: String,
        /// PA-093：事件归属分支（阶段 3 起分支可见性推导的基石；legacy 数据 'main'）。
        branch_id: String,
        events: Vec<crate::agent::turn_event::TurnEvent>,
    },
    PublishMetadata {
        epoch: u64,
        key: String,
        value: String,
    },
}

/// turn 的规范化记录（normalized_turns 行）。
#[derive(Clone, Debug)]
pub struct TurnStateRecord {
    pub turn_id: String,
    pub ordinal: usize,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub user_message_id: Option<String>,
    pub assistant_message_id: Option<String>,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

/// trace 终态补丁（normalized_turn_traces 更新）。
#[derive(Clone, Debug)]
pub struct TraceTerminalPatch {
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub updated_at: u64,
    pub phase: Option<String>,
    /// PA-094：trace 表降级为投影缓存——行带 seq 水位（事件日志水位，
    /// 清表重建时用于对齐折叠范围；annotate 时捕获会话水位）。
    pub event_watermark: Option<u64>,
}

/// 会话元数据补丁（normalized_sessions 更新）。
#[derive(Clone, Debug, Default)]
pub struct SessionMetaPatch {
    pub title: Option<String>,
    pub title_override: Option<Option<String>>,
    pub archived: Option<bool>,
    pub summary: Option<String>,
    pub turn_count: Option<usize>,
    pub last_referenced_file: Option<String>,
    pub updated_at_ms: Option<u64>,
}

/// ADR 0099 阶段 0：提交边界结果（唯一提交入口
/// [`SessionBackend::commit_boundary`] 的返回）。
///
/// 语义真源：`.dev-team/contracts/stage0-write-model.md`（不变量 I1–I5）。
///
/// - `Committed`：本次快照已作为**新版本**落盘（且本 turn 只落这一次）。
/// - `StaleVersion { expected, actual }`：`store` 携带的**期望版本**落后于库内实际
///   版本 ⇒ **必须零写入**（不覆盖、不删除、不复活），调用方以重载 / 只读收场。
/// - `Failed`：写盘失败（I/O、序列化、事务回滚）；调用方必须可观测上报，
///   禁止用 `let _ =` 静默吞掉。
/// - `Unsupported`：后端无版本守门能力（File / Memory），语义退化到 `save_store`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed,
    StaleVersion { expected: u64, actual: u64 },
    Failed,
    Unsupported,
}

/// ADR 0099 阶段 0：孤儿会话回填报告（契约类型，阶段 0 实作任务填充产出）。
///
/// 契约（`.dev-team/contracts/stage0-write-model.md` 不变量 I4）：
/// 回填源只允许 `turn_events ∪ normalized_*`，**禁止**以 legacy `sessions` blob 为源；
/// 无法回填的会话显式标 `retired`（UI 明示），不得物理删除。
///
/// 字段语义：`backfilled` = 已从规范化表 / 事件流重建并写回 blob 的会话；
/// `retired` = 源数据不足、显式退役的会话；`checksum_mismatch` = 重建后校验和
/// 与规范化行不一致的会话（进 quarantine 式报告，不静默覆盖）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub backfilled: Vec<String>,
    pub retired: Vec<String>,
    pub checksum_mismatch: Vec<String>,
}

/// 命令执行结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersistCommandOutcome {
    Succeeded,
    Unsupported,
    /// 旧 epoch 命令被拒绝（materialize barrier 后）。
    StaleEpoch,
    Failed,
}

impl PersistCommand {
    /// 命令所属 epoch（迁移 barrier 用）。
    pub fn epoch(&self) -> u64 {
        match self {
            PersistCommand::AppendMessage { epoch, .. }
            | PersistCommand::UpsertTurn { epoch, .. }
            | PersistCommand::AppendTrace { epoch, .. }
            | PersistCommand::UpdateTraceTerminal { epoch, .. }
            | PersistCommand::AppendHookRecords { epoch, .. }
            | PersistCommand::UpdateHistoryNode { epoch, .. }
            | PersistCommand::UpdateCursor { epoch, .. }
            | PersistCommand::UpdateBranch { epoch, .. }
            | PersistCommand::UpdateSessionMeta { epoch, .. }
            | PersistCommand::RemoveSession { epoch, .. }
            | PersistCommand::FlushEvents { epoch, .. }
            | PersistCommand::PublishMetadata { epoch, .. } => *epoch,
        }
    }
}
pub trait SessionBackend: Send + Sync {
    fn load_store(&self) -> Option<PersistedStore>;
    /// 整包写盘。**失败必须上报**（T2-3）：历史上这里返回 `()`，文件后端把
    /// `create_dir_all` / 序列化 / `fs::write` 的失败全部静默吞掉，上层于是把
    /// "会话丢失"当成"保存成功"。返回 `Err` 的调用方必须至少落一条可观测日志，
    /// 不得用 `let _ =` 再吞一次。
    fn save_store(&self, store: &PersistedStore) -> Result<(), String>;

    /// ADR 0099 阶段 0：**单一提交边界**（契约空桩，阶段 0 实作任务填充）。
    ///
    /// 不变量（真源：`.dev-team/contracts/stage0-write-model.md`）：
    /// ① 每 turn 全库提交 ≤1 次——19 个 `save_to_backend` /
    /// `save_session_state_to_backend` 调用点收敛到本入口；
    /// ② `store` 携带的期望版本与库内实际版本不匹配 ⇒ 返回
    /// [`CommitOutcome::StaleVersion`] 且**零写入**（不覆盖新 turn）；
    /// ③ 显式删除后任何旧版本快照不得复活该会话（墓碑优先于快照）。
    ///
    /// 默认 `Unsupported`：File / Memory 后端无版本守门，保持可编译并把语义退化到
    /// [`Self::save_store`]；生产路径（`SqliteSessionBackend`）必须在阶段 0 覆写。
    fn commit_boundary(&self, _store: &PersistedStore) -> CommitOutcome {
        CommitOutcome::Unsupported
    }

    fn trace_storage_mode(&self) -> SeparateTraceTableMode {
        SeparateTraceTableMode::Off
    }
    /// PA-089 阶段 3：规范化双写命令（blob + normalized_* 表统一事务）。
    /// 默认 Unsupported——File/Memory backend 无需实现；SqliteSessionBackend 实现。
    fn persist_command(&self, _command: PersistCommand) -> PersistCommandOutcome {
        PersistCommandOutcome::Unsupported
    }
    /// 批量规范化持久化命令（单个 SQLite BEGIN EXCLUSIVE 事务原子提交）。
    fn persist_commands_batch(&self, commands: Vec<PersistCommand>) -> PersistCommandOutcome {
        for command in commands {
            let outcome = self.persist_command(command);
            if !matches!(
                outcome,
                PersistCommandOutcome::Succeeded | PersistCommandOutcome::Unsupported
            ) {
                return outcome;
            }
        }
        PersistCommandOutcome::Succeeded
    }
    fn upsert_session(&self, _session_id: &str, _session: &SessionState) -> bool {
        false
    }
    fn remove_session(
        &self,
        _session_id: &str,
        _attachment_assets: &AttachmentAssetMap,
        _session_attachment_index: &SessionAttachmentIndex,
        _mcp_source_snapshots: &HashMap<String, McpSourceSnapshot>,
        _skill_source_snapshots: &HashMap<String, SkillSourceSnapshot>,
    ) -> bool {
        false
    }
    fn load_session_traces(&self, _session_id: &str) -> SessionBackendTraceLoadResult {
        SessionBackendTraceLoadResult::Unsupported
    }

    /// ADR 0094：按 id 从**规范化表**重建单个会话（`normalized_sessions` +
    /// `normalized_messages` + trace/node/cursor 子表）。
    ///
    /// 用途：legacy `sessions` blob 行丢失（历史整包写盘误删 / 迁移半途）时，
    /// `pony resume <id>` 仍能恢复出真实历史，而不是谎称「没有该会话」。
    /// 默认 `None`：内存/文件后端没有规范化表，无从重建。
    fn load_normalized_session(&self, _session_id: &str) -> Option<SessionState> {
        None
    }

    /// Wave-7 B001：写入一条用量账本记录（幂等 UPSERT）。
    ///
    /// 默认实现：非 SQLite 后端无独立账本表 ⇒ 静默成功（fail-open，不阻断 turn）。
    fn append_usage_ledger_entry(
        &self,
        _entry: &crate::agent::usage_ledger::LedgerEntry,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Wave-7 B001：账本窗口聚合（默认零值，非 SQLite 后端无账本）。
    fn usage_ledger_window_summary(
        &self,
        _scope: &crate::agent::usage_ledger::LedgerScope,
    ) -> Result<crate::agent::usage_ledger::WindowSummary, String> {
        Ok(crate::agent::usage_ledger::WindowSummary::zero())
    }

    /// Wave-7 B001：账本天桶聚合（默认空）。
    fn usage_ledger_day_summaries(
        &self,
        _session_id: Option<&str>,
    ) -> Result<Vec<crate::agent::usage_ledger::DaySummary>, String> {
        Ok(Vec::new())
    }
    /// PA-093：读取会话事件流 `(seq, branch_id, event)`（seq 升序，可截断）。
    /// 默认返回空（File/Memory backend 不支持——引用化节点在非 SQLite 后端下
    /// checkout 退化为空视图并告警；生产路径恒为 SqliteSessionBackend）。
    fn load_turn_events(
        &self,
        _session_id: &str,
        _up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, crate::agent::turn_event::TurnEvent)> {
        Vec::new()
    }
    /// PA-095：带契约校验的事件加载——版本不匹配或坏 payload fail loud（Err 上抛）。
    /// 默认委托 `load_turn_events`（非 SQLite 后端无契约概念，恒 Ok）。
    fn load_turn_events_checked(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Result<Vec<(u64, String, crate::agent::turn_event::TurnEvent)>, String> {
        Ok(self.load_turn_events(session_id, up_to_seq))
    }
    /// PA-095 #2：后端是否具备事件表能力（append_turn 物化的回退判据——仅
    /// 真无事件后端允许回退调用方参数）。默认 false（Memory/File 等）。
    fn supports_turn_events(&self) -> bool {
        false
    }
    /// PA-095 #7（实施后审核 P2）：列出会话已外置的 observation 引用
    /// （形如 `bco:<turn_id>:<seq>`）。默认空（非 SQLite 后端无独立表）。
    /// 供 flush 后把 ref 附加到内存 trace 缓存行——防止异步 trace worker 的
    /// 整行 REPLACE 把同事务回填的引用覆盖掉。
    fn load_observation_refs(&self, _session_id: &str) -> Vec<String> {
        Vec::new()
    }
    /// PA-095：事件 schema 版本校验（缺失视为 v1 并回填；不匹配 Err）。
    /// 默认 Ok（非 SQLite 后端无 store_metadata 契约）。
    fn validate_event_schema(&self) -> Result<u64, String> {
        Ok(crate::agent::turn_event::EVENT_SCHEMA_VERSION)
    }
    /// PA-093：当前事件水位（已落盘事件总数）。默认 0（非 SQLite 后端无事件流）。
    fn load_event_watermark(&self, _session_id: &str) -> u64 {
        0
    }
    /// PA-094：按引用加载 build_context_observation 全量 payload（大字段外置）。
    /// 默认 None（非 SQLite 后端无独立表；legacy 内嵌数据走 trace 字段）。
    fn load_build_context_observation(
        &self,
        _session_id: &str,
        _observation_ref: &str,
    ) -> Option<crate::agent::provider::BuildContextObservation> {
        None
    }
    fn replace_session_traces(
        &self,
        _session_id: &str,
        _traces: &[TurnTraceRecord],
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn upsert_turn_trace(
        &self,
        _session_id: &str,
        _trace: &TurnTraceRecord,
        _trace_order: usize,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn update_turn_trace_terminal_event(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _event_id: Option<&str>,
        _event_type: Option<&str>,
        _event_version: Option<&str>,
        _sequence: Option<u64>,
        _emitted_at_ms: Option<u64>,
        _updated_at: u64,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn append_turn_trace_hook_records(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _hook_trace_records: &[HookTraceRecord],
        _updated_at: u64,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn persist_session_with_trace_mutation(
        &self,
        _session_id: &str,
        _session: &SessionState,
        _mutation: SessionTraceMutation,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn attachment_root(&self) -> Option<PathBuf>;
}

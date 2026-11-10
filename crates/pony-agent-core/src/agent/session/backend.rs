use super::store::PersistedStore;
use super::types::{
    AttachmentAssetMap, HistoryBranch, HistoryCursor, HistoryNode, LongTermMemoryRecord,
    SessionAttachmentIndex, SessionState, TurnHistoryMessage, TurnTraceRecord,
};
use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};
use crate::agent::hooks::{HistoryStateHookEvidence, HookTraceRecord, PersistedEffectEvidence};
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
    /// 上下文压缩提交：**一次压缩 = 一条 append-only 命令**。
    ///
    /// 语义（与 `TurnEvent::HistorySquash` 事件、`store.rs::commit_compaction`
    /// 一一对应）：
    /// - `node`：压缩后的**可见节点快照**（整份替换同 `node_id` 的行），会话历史的
    ///   可见视图由它派生；
    /// - `transcript`：压缩后的原生 transcript **全量替换**（不是追加）。
    ///
    /// 为什么必须落日志：压缩原先只写 blob（`replace_session_history` →
    /// `save_to_backend`），而装载以**日志为真源**
    /// （`merge_log_truth_into_sessions`）⇒ 重启后压缩结果被日志里的旧历史覆盖，
    /// 「压缩了但重启就回到未压缩」。
    CompactHistory {
        epoch: u64,
        session_id: String,
        node: HistoryNode,
        transcript: Vec<serde_json::Value>,
        summary_message: String,
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
    /// ADR 0102：provider 原生 transcript 的**追加**载体。
    ///
    /// 语义唯一的理由是写路径：`store.rs` 对 transcript 做的是 `extend`（逐轮追加），
    /// 日志必须能表达同一件事，否则「有日志的会话」重启即丢 transcript
    /// （阶段 2-D 的轻量索引行不再写它，日志又没有载体）。
    AppendProviderTranscript {
        epoch: u64,
        session_id: String,
        /// 本批**新增**的 transcript 消息（不是全量快照）。
        messages: Vec<serde_json::Value>,
    },
    /// ADR 0102：会话记忆 + `history_state_evidence` 的载体。
    ///
    /// **两种语义刻意放在同一条命令里**，因为它们由同一次「记忆更新」产生，拆开会让
    /// 重放出现「记忆已换、证据还是旧的」中间态：
    /// - `entries` / `evidence` / `hook_trace_records`：**整份替换**（对齐
    ///   `update_long_term_memory_from_user_message` 的产物——它就地重建这三项）；
    /// - `history_state_evidence`：**追加**（对齐 `persist_history_state_hook_evidence`
    ///   的 `extend` 语义）。
    UpdateSessionMemory {
        epoch: u64,
        session_id: String,
        entries: Vec<LongTermMemoryRecord>,
        evidence: Vec<PersistedEffectEvidence>,
        hook_trace_records: Vec<HookTraceRecord>,
        history_state_evidence: Vec<HistoryStateHookEvidence>,
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
    /// ADR 0107 §5 P0（日志先行 + 失败域隔离）：命令**已写入会话日志**（真源 +
    /// `fsync`），但索引（SQLite）未应用——索引滞后可恢复，**不等于本轮历史丢失**。
    ///
    /// 只有「日志写成功 + 索引写失败（含 busy 重试耗尽 / begin tx 不可写 / 只读降级）」
    /// 才返回它；日志写失败仍是 [`PersistCommandOutcome::Failed`]（fail-loud，SQLite 一行不改）。
    ///
    /// 调用方契约（`SessionStore::append_turn_fallible` 等）：必须提交内存 live 状态 +
    /// 打一条含 `session_id` / `outcome` 的结构化「索引滞后」告警，**禁止**把它当作失败
    /// 回滚内存，也**不要**再走索引侧单行落盘（索引由后续写入 / 重建补齐）。
    LogPersisted,
    /// ADR 0107 §5 P1：**同一会话正被另一个进程写**（会话日志 `flock` 冲突）。
    ///
    /// 与 [`PersistCommandOutcome::Failed`] 的区别：这不是数据损坏、也不是索引故障，
    /// 而是「会话级单写者」的正常拒绝——调用方必须映射成可识别的
    /// `SessionError::SessionWriterBusy` 并让用户看到"该会话在别处被占用"。
    WriterBusy,
    Failed,
}

/// ADR 0107 §5 P0：`LogPersisted`（日志已落盘、索引未应用）的结构化告警。
///
/// 含 `session_id` 与 `outcome`，可直接 grep 断言。返回 `true` 表示**索引未应用**：
/// 调用方必须提交内存 live 状态并**跳过**索引侧单行落盘（索引滞后由后续写入 / 重建补齐）。
pub fn report_index_deferred(session_id: &str, outcome: &PersistCommandOutcome) -> bool {
    if !matches!(outcome, PersistCommandOutcome::LogPersisted) {
        return false;
    }
    INDEX_DEFERRED_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    eprintln!(
        "[pony-agent][session] index_deferred: session={session_id} outcome={outcome:?} total={} \
         （会话日志已落盘、SQLite 索引滞后：本轮历史提交内存 live 状态，索引由后续写入/重建补齐）",
        index_deferred_total()
    );
    true
}

/// ADR 0107 §5 P1：进程级「索引滞后」累计次数（可机读，供诊断/测试断言）。
///
/// 语义：会话日志已落盘、但 SQLite 索引未应用的**提交次数**。它是「索引新鲜度」的
/// 最简可观测信号——非零即说明本进程至少有一轮历史只存在于日志里（可重建、未丢）。
static INDEX_DEFERRED_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 读取累计的「索引滞后」次数（进程级，只增不减）。
pub fn index_deferred_total() -> u64 {
    INDEX_DEFERRED_TOTAL.load(std::sync::atomic::Ordering::Relaxed)
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
            | PersistCommand::CompactHistory { epoch, .. }
            | PersistCommand::UpdateCursor { epoch, .. }
            | PersistCommand::UpdateBranch { epoch, .. }
            | PersistCommand::UpdateSessionMeta { epoch, .. }
            | PersistCommand::AppendProviderTranscript { epoch, .. }
            | PersistCommand::UpdateSessionMemory { epoch, .. }
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

    /// 仅测试用：丢弃会话日志写者缓存（释放文件级独占锁）。
    ///
    /// Windows 的 `LockFileEx` 是**强制**锁：同一进程内另一个句柄在写者持锁期间读不了该文件。
    /// 需要"写完 → 重新装载 / 直接读盘"的用例必须先放锁；Unix 的 `flock` 是劝告锁，读不受影响
    /// （故此类缺陷只在 Windows CI 暴露：B041 单元测试、B044 集成测试）。
    /// 默认 no-op：非 SQLite backend 无日志写者。
    ///
    /// 集成测试（`tests/` 是独立 crate，看不到 `#[cfg(test)]` 项）也要调用它，故这里**不加**
    /// `#[cfg(test)]`；`#[doc(hidden)]` 标明它不是产品契约，只服务测试夹具。
    #[doc(hidden)]
    fn release_log_writers_for_test(&self) {}

    fn trace_storage_mode(&self) -> SeparateTraceTableMode {
        SeparateTraceTableMode::Off
    }

    /// ADR 0099 阶段 0（E5）→ ADR 0107 §5 P1：后端是否只读。
    ///
    /// P1 起「抢不到库级锁 ⇒ 整库降级只读」已取消，本方法等价于
    /// [`Self::storage_read_only`]（仅维护/诊断句柄为 `true`）；保留它是因为既有调用点
    /// 语义就是「这次写入是否会被拒绝」。
    fn is_read_only(&self) -> bool {
        self.storage_read_only()
    }

    /// ADR 0107 §5 P1：后端是否为**维护/诊断只读句柄**（`mode=ro` + `query_only`）。
    ///
    /// 默认 `false`。宿主据此关闭同一库上的**旁路写连接**（`pending_asks` 等）——维护
    /// 句柄必须保持「库文件零写入」。日常实例不再进入该状态（不再有锁降级）。
    fn storage_read_only(&self) -> bool {
        false
    }

    /// ADR 0099 阶段 0（I2）：读某个会话的库内版本（`sessions.version`）。
    ///
    /// `SessionStore` 在用**单行写**（`upsert_session` / `persist_session_with_trace_mutation`）
    /// 落盘后，用它把 CAS 期望版本刷新成库内真实值——否则「自己刚写的那一版」会在下一次
    /// 整包提交时被判成 `StaleVersion`。默认 `None`（非 SQLite 后端无版本概念）。
    fn session_version(&self, _session_id: &str) -> Option<u64> {
        None
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

#[cfg(test)]
mod index_deferred_tests {
    use super::*;

    /// ADR 0107 §5 P1：`index_deferred` 必须可机读（不只靠 grep 日志行）。
    #[test]
    fn index_deferred_counter_tracks_log_persisted_only() {
        let before = index_deferred_total();
        assert!(
            !report_index_deferred("s-index-deferred", &PersistCommandOutcome::Succeeded),
            "非 LogPersisted 不得计入索引滞后"
        );
        assert_eq!(index_deferred_total(), before, "非 LogPersisted 不得改变计数");
        assert!(
            report_index_deferred("s-index-deferred", &PersistCommandOutcome::LogPersisted),
            "LogPersisted 必须报告索引未应用"
        );
        assert_eq!(
            index_deferred_total(),
            before + 1,
            "LogPersisted 必须让计数 +1"
        );
    }
}

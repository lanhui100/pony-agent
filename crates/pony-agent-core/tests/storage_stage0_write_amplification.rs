//! ADR 0099 阶段 0 · 契约 1：写放大收口（一次 `append_turn` 只允许 1 次全库提交）。
//!
//! 现网事实（ADR 0099 背景表）：`session/store.rs` 的 19 个 `save_to_backend` /
//! `save_session_state_to_backend` 调用点会把整库 `PersistedStore`（真实库 5.1 MB）
//! `INSERT OR REPLACE` ≥20 次/turn ⇒ ≈102 MB/turn，写放大 ≈790×。
//!
//! 阶段 0 契约（`.dev-team/contracts/stage0-write-model.md` 不变量 I1）：
//! 每 turn 只允许**一次**提交边界穿越；`SessionBackend::commit_boundary` 是唯一提交
//! 入口（默认 `CommitOutcome::Unsupported` ⇒ 调用方回退 `save_store`）。
//!
//! 判定方式：`CountingBackend` 包装真实 `SqliteSessionBackend`（模式同
//! `crates/pony-agent-core/src/agent/control_plane/mod.rs:7650` 既有实现），统计
//! `save_store` 调用次数。默认 `commit_boundary` 返回 `Unsupported`，因此实现完成后
//! 恰好回退一次 `save_store` ⇒ 计数必须 **== 1**。
//!
//! 隔离：数据库固定落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/sessions.db`；用例结束 `Drop` 清理。
//!
//! 红相（交付时）：计数为 0（当前 `append_turn` 走 `persist_commands_batch` +
//! `persist_session_with_trace_mutation`，从不经过统一提交边界）⇒ 断言 `== 1` 失败。

use pony_agent_core::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};
use pony_agent_core::agent::session::{
    AttachmentAsset, CommitOutcome, PersistCommand, PersistedStore, SeparateTraceTableMode,
    SessionBackend, SessionBackendMutationResult, SessionBackendTraceLoadResult, SessionState,
    SessionStore, SessionTraceMutation, TurnTraceRecord,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// 隔离夹具
// ---------------------------------------------------------------------------

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0099-stage0-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn db(&self) -> PathBuf {
        self.path.join("sessions.db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn open_sqlite(db: &Path) -> SqliteSessionBackend {
    SqliteSessionBackend::new_with_trace_mode(
        db.to_path_buf(),
        // WriteSeparate ⇒ 新会话进入 TraceTableAuthoritative，走真实生产写入形态。
        SeparateTraceTableMode::WriteSeparate,
    )
}

// ---------------------------------------------------------------------------
// CountingBackend：统计全库提交次数（save_store）
// ---------------------------------------------------------------------------

struct CountingBackend {
    inner: SqliteSessionBackend,
    save_store_count: Arc<AtomicUsize>,
    commit_boundary_count: Arc<AtomicUsize>,
    attachment_root: PathBuf,
}

impl CountingBackend {
    /// 返回后端 + 两个计数器（全库提交 `save_store` 次数、提交边界 `commit_boundary` 穿越次数）。
    fn new(db: &Path, attachment_root: PathBuf) -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let save_store_count = Arc::new(AtomicUsize::new(0));
        let commit_boundary_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                inner: open_sqlite(db),
                save_store_count: Arc::clone(&save_store_count),
                commit_boundary_count: Arc::clone(&commit_boundary_count),
                attachment_root,
            },
            save_store_count,
            commit_boundary_count,
        )
    }
}

impl SessionBackend for CountingBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        self.inner.load_store()
    }

    fn save_store(&self, store: &PersistedStore) -> Result<(), String> {
        self.save_store_count.fetch_add(1, Ordering::SeqCst);
        self.inner.save_store(store)
    }

    /// 契约 §3 I1 的验收口径：CountingBackend 覆写 `commit_boundary` 计数后返回
    /// `Unsupported` ⇒ 调用方回退 `save_store`，于是一次 turn 的全库提交次数可直接观察。
    fn commit_boundary(&self, _store: &PersistedStore) -> CommitOutcome {
        self.commit_boundary_count.fetch_add(1, Ordering::SeqCst);
        CommitOutcome::Unsupported
    }

    fn trace_storage_mode(&self) -> SeparateTraceTableMode {
        self.inner.trace_storage_mode()
    }

    fn persist_command(
        &self,
        command: PersistCommand,
    ) -> pony_agent_core::agent::session::PersistCommandOutcome {
        self.inner.persist_command(command)
    }

    fn persist_commands_batch(
        &self,
        commands: Vec<PersistCommand>,
    ) -> pony_agent_core::agent::session::PersistCommandOutcome {
        self.inner.persist_commands_batch(commands)
    }

    fn upsert_session(&self, session_id: &str, session: &SessionState) -> bool {
        self.inner.upsert_session(session_id, session)
    }

    fn remove_session(
        &self,
        session_id: &str,
        attachment_assets: &HashMap<String, AttachmentAsset>,
        session_attachment_index: &HashMap<String, Vec<String>>,
        mcp_source_snapshots: &HashMap<String, McpSourceSnapshot>,
        skill_source_snapshots: &HashMap<String, SkillSourceSnapshot>,
    ) -> bool {
        self.inner.remove_session(
            session_id,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
        )
    }

    fn load_session_traces(&self, session_id: &str) -> SessionBackendTraceLoadResult {
        self.inner.load_session_traces(session_id)
    }

    fn supports_turn_events(&self) -> bool {
        self.inner.supports_turn_events()
    }

    fn load_turn_events(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, pony_agent_core::agent::turn_event::TurnEvent)> {
        self.inner.load_turn_events(session_id, up_to_seq)
    }

    fn load_turn_events_checked(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Result<Vec<(u64, String, pony_agent_core::agent::turn_event::TurnEvent)>, String> {
        self.inner.load_turn_events_checked(session_id, up_to_seq)
    }

    fn validate_event_schema(&self) -> Result<u64, String> {
        self.inner.validate_event_schema()
    }

    fn load_event_watermark(&self, session_id: &str) -> u64 {
        self.inner.load_event_watermark(session_id)
    }

    fn replace_session_traces(
        &self,
        session_id: &str,
        traces: &[TurnTraceRecord],
    ) -> SessionBackendMutationResult {
        self.inner.replace_session_traces(session_id, traces)
    }

    fn upsert_turn_trace(
        &self,
        session_id: &str,
        trace: &TurnTraceRecord,
        trace_order: usize,
    ) -> SessionBackendMutationResult {
        self.inner.upsert_turn_trace(session_id, trace, trace_order)
    }

    fn persist_session_with_trace_mutation(
        &self,
        session_id: &str,
        session: &SessionState,
        mutation: SessionTraceMutation,
    ) -> SessionBackendMutationResult {
        self.inner
            .persist_session_with_trace_mutation(session_id, session, mutation)
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

// ---------------------------------------------------------------------------
// 契约 1：写放大收口
// ---------------------------------------------------------------------------

/// 一次 `append_turn` 只允许穿越 1 次提交边界，且最多 1 次全库快照写盘。
///
/// 口径说明（探针实测）：`SessionStore::with_backend` 装载期会自带一次 `save_store`
/// （默认会话首建），因此计数必须取 **Act 前后的增量**，不能取绝对值。
///
/// 红相根因：当前 `append_turn` 的写盘路径是「`persist_commands_batch`（规范化双写事务）
/// + `persist_session_with_trace_mutation`（单会话表权威写）」，**完全不经过**
/// `SessionBackend::commit_boundary` ⇒ 边界穿越增量 = 0（实测），断言失败。
/// 这正是 ADR 0099 阶段 0 第 1 条要收口的结构性原因：19 个落盘调用点各自为政，
/// 一整个 turn 生命周期内整库快照会被反复覆盖。
#[test]
fn append_turn_triggers_exactly_one_full_store_commit() {
    // Arrange
    let dir = TempDir::new("amp");
    let db = dir.db();
    let (backend, save_store_count, commit_boundary_count) =
        CountingBackend::new(&db, dir.path.join("attachments"));
    let mut store = SessionStore::with_backend(Box::new(backend));
    let commits_before = save_store_count.load(Ordering::SeqCst);
    let boundaries_before = commit_boundary_count.load(Ordering::SeqCst);

    // Act
    store.append_turn(
        Some("s-amp"),
        "写放大探针：用户消息",
        "写放大探针：助手回复",
        None,
        Vec::new(),
    );

    // Assert ①：本 turn 必须**恰好穿越一次**单一提交边界。
    let boundaries = commit_boundary_count.load(Ordering::SeqCst) - boundaries_before;
    assert_eq!(
        boundaries, 1,
        "ADR 0099 阶段 0 / I1：一次 append_turn 必须恰好穿越 1 次 \
         SessionBackend::commit_boundary，实际 {boundaries} 次。\
         当前实现完全不调用该入口（19 个 save_to_backend / save_session_state_to_backend \
         调用点各自为政），写放大 ≈790× 无法收口"
    );

    // Assert ②：本 turn 内全库快照写盘必须恰好 1 次（`commit_boundary` 返回
    // `Unsupported` ⇒ 调用方回退 `save_store` 一次；契约 §3 I1 的验收口径）。
    let commits = save_store_count.load(Ordering::SeqCst) - commits_before;
    assert_eq!(
        commits, 1,
        "ADR 0099 阶段 0 / I1：一次 append_turn 内全库快照写盘（save_store）必须恰好 1 次，\
         实际 {commits} 次（现网口径 5.1MB × ≥20 次/turn ≈ 102MB/turn，目标 ~5MB/turn）"
    );

    // 非空测试防线：这次 turn 必须真的落盘，否则「只提交 1 次」毫无意义。
    let mut reloaded = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    let snapshot = reloaded.snapshot(Some("s-amp"), &[]);
    assert_eq!(
        snapshot.history.len(),
        2,
        "前提防线：turn 必须已落盘（user + assistant 两条），否则提交次数断言无意义"
    );
}

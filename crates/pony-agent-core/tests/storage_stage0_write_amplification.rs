//! ADR 0099 阶段 0 · 契约 1（Lead 复核后 re-freeze 口径）：写放大收口。
//!
//! ## 已裁定的事实（Lead 裁决 2026-11-08，取代前一版「save_store 增量 ==1」口径）
//!
//! - `save_session_state_to_backend`（`session/store.rs:2462`）已有三级降级：
//!   `persist_session_with_trace_mutation` → `upsert_session`（**单行写**）→ 最后才整包
//!   `save_store`。因此**消息级 `append_turn` 的正确行为是 `save_store` 增量 == 0**
//!   （既有绿用例 `control_plane/mod.rs:8053` 已按此断言）。
//! - 真正的写放大来源是 `save_to_backend` 的 ~18 个**全库入口**
//!   （如 `create_workspace` → `store.rs:2035` 直接 `save_to_backend()`），不是 `append_turn`。
//! - 阶段 0 契约（`.dev-team/contracts/stage0-write-model.md` I1 v1.1）：全库入口必须收敛到
//!   **单一提交边界** `SessionBackend::commit_boundary`；装载期自带 1 次 `save_store`
//!   （`with_backend` → `default_sessions()` / `bootstrap_default_workspace` /
//!   `refresh_session_metadata` 置 `should_save`），故一切计数必须取 **Act 前后增量**。
//!
//! ## 两条用例的判定口径
//!
//! 1. `append_turn_costs_exactly_one_incremental_commit_primitive`
//!    - `save_store` 增量 **== 0**（消息级 turn 不得触发整包写）；
//!    - 提交原语总次数 **== 1**，原语集合 = `commit_boundary` + `upsert_session` +
//!      `persist_session_with_trace_mutation` + `save_store`（四者调用数之和的增量）。
//! 2. `full_store_entry_routes_through_commit_boundary_once`
//!    - 全库入口（`create_workspace` → `save_to_backend`）必须**只走 1 次 `commit_boundary`**；
//!    - `save_store` 增量 ≤ 1（若实现仍需整包写，必须经提交边界而非直连 `save_store`）；
//!    - 前置防线：这次提交必须真的落盘（`store_metadata.workspaces` 含新工作区）。
//!
//! 计数实现：`CountingBackend` 包装真实 `SqliteSessionBackend`（模式同
//! `agent/control_plane/mod.rs:7650` 既有实现），`commit_boundary` 计入后**委托内层**
//! （内层落码后自行落盘，不再经包装层的 `save_store`，从而能区分「直连整包写」与
//! 「经提交边界」）。
//!
//! 隔离：数据库固定落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/sessions.db`；用例结束 `Drop` 清理。
//!
//! 红相（交付时）：用例 1 为**回归守卫**（今天已绿：`persist_session_with_trace_mutation`
//! 单次单行写 ⇒ 原语总数 1、`save_store` 0）；用例 2 为**病灶红相**（今天
//! `create_workspace` 直连 `save_store` ⇒ `commit_boundary` 0 次，断言失败）。

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
// CountingBackend：统计四类提交原语
// ---------------------------------------------------------------------------

/// 提交原语计数快照（全部取 Act 前后**增量**，因为装载期自带 1 次 `save_store`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CommitCounts {
    save_store: usize,
    commit_boundary: usize,
    upsert_session: usize,
    persist_session_with_trace_mutation: usize,
}

impl CommitCounts {
    /// 提交原语总次数（Lead 裁定口径：四者之和）。
    fn total(self) -> usize {
        self.save_store
            + self.commit_boundary
            + self.upsert_session
            + self.persist_session_with_trace_mutation
    }
}

struct CountingBackend {
    inner: SqliteSessionBackend,
    save_store: Arc<AtomicUsize>,
    commit_boundary: Arc<AtomicUsize>,
    upsert_session: Arc<AtomicUsize>,
    persist_session_with_trace_mutation: Arc<AtomicUsize>,
    attachment_root: PathBuf,
}

impl CountingBackend {
    fn new(
        db: &Path,
        attachment_root: PathBuf,
    ) -> (
        Self,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
    ) {
        let save_store = Arc::new(AtomicUsize::new(0));
        let commit_boundary = Arc::new(AtomicUsize::new(0));
        let upsert_session = Arc::new(AtomicUsize::new(0));
        let persist_session_with_trace_mutation = Arc::new(AtomicUsize::new(0));
        (
            Self {
                inner: open_sqlite(db),
                save_store: Arc::clone(&save_store),
                commit_boundary: Arc::clone(&commit_boundary),
                upsert_session: Arc::clone(&upsert_session),
                persist_session_with_trace_mutation: Arc::clone(
                    &persist_session_with_trace_mutation,
                ),
                attachment_root,
            },
            save_store,
            commit_boundary,
            upsert_session,
            persist_session_with_trace_mutation,
        )
    }
}

impl SessionBackend for CountingBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        self.inner.load_store()
    }

    fn save_store(&self, store: &PersistedStore) -> Result<(), String> {
        self.save_store.fetch_add(1, Ordering::SeqCst);
        self.inner.save_store(store)
    }

    /// 计入后**委托内层**：内层实现提交边界后自行落盘，不再经包装层的 `save_store`，
    /// 从而「直连整包写」与「经提交边界」在计数上可区分。
    fn commit_boundary(&self, store: &PersistedStore) -> CommitOutcome {
        self.commit_boundary.fetch_add(1, Ordering::SeqCst);
        self.inner.commit_boundary(store)
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
        self.upsert_session.fetch_add(1, Ordering::SeqCst);
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
        self.persist_session_with_trace_mutation
            .fetch_add(1, Ordering::SeqCst);
        self.inner
            .persist_session_with_trace_mutation(session_id, session, mutation)
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

/// 读取当前四类原语计数（供 Act 前记基线、Act 后算增量）。
fn read_counts(
    save_store: &AtomicUsize,
    commit_boundary: &AtomicUsize,
    upsert_session: &AtomicUsize,
    persist_session_with_trace_mutation: &AtomicUsize,
) -> CommitCounts {
    CommitCounts {
        save_store: save_store.load(Ordering::SeqCst),
        commit_boundary: commit_boundary.load(Ordering::SeqCst),
        upsert_session: upsert_session.load(Ordering::SeqCst),
        persist_session_with_trace_mutation: persist_session_with_trace_mutation
            .load(Ordering::SeqCst),
    }
}

fn delta(after: CommitCounts, before: CommitCounts) -> CommitCounts {
    CommitCounts {
        save_store: after.save_store - before.save_store,
        commit_boundary: after.commit_boundary - before.commit_boundary,
        upsert_session: after.upsert_session - before.upsert_session,
        persist_session_with_trace_mutation: after.persist_session_with_trace_mutation
            - before.persist_session_with_trace_mutation,
    }
}

fn store_metadata_contains(db: &Path, key: &str, needle: &str) -> bool {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT value FROM store_metadata WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .map(|value| value.contains(needle))
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 契约 1-a：消息级 turn 只允许 1 次增量提交原语，且零整包写
// ---------------------------------------------------------------------------

/// `append_turn` 的写盘成本：**1 个增量提交原语**（单行写），**0 次整包 `save_store`**。
///
/// 这是既有正确行为的回归守卫（`save_session_state_to_backend` 的三级降级：
/// `persist_session_with_trace_mutation` → `upsert_session` 单行 → 最后才 `save_store`）。
/// 若实现把消息级 turn 也改成「提交边界 + 整包写」，原语总数会 >1、`save_store` 会 >0，
/// 两条断言同时抓住。
#[test]
fn append_turn_costs_exactly_one_incremental_commit_primitive() {
    // Arrange
    let dir = TempDir::new("amp-turn");
    let db = dir.db();
    let (backend, save_store, commit_boundary, upsert_session, psmtm) =
        CountingBackend::new(&db, dir.path.join("attachments"));
    let mut store = SessionStore::with_backend(Box::new(backend));
    let before = read_counts(&save_store, &commit_boundary, &upsert_session, &psmtm);

    // Act
    store.append_turn(
        Some("s-amp"),
        "写放大探针：用户消息",
        "写放大探针：助手回复",
        None,
        Vec::new(),
    );

    // Assert ①：消息级 turn 不得触发整包写。
    let spent = delta(
        read_counts(&save_store, &commit_boundary, &upsert_session, &psmtm),
        before,
    );
    assert_eq!(
        spent.save_store, 0,
        "ADR 0099 阶段 0 / I1：append_turn 是消息级写入，必须走单行写路径\
         （persist_session_with_trace_mutation / upsert_session），不得触发整包 save_store；\
         实际整包写 {} 次（明细 {spent:?}）",
        spent.save_store
    );

    // Assert ②：提交原语总数必须恰好 1（四者之和的增量）。
    assert_eq!(
        spent.total(),
        1,
        "ADR 0099 阶段 0 / I1：一次 append_turn 的提交原语总数必须恰好 1\
         （commit_boundary + upsert_session + persist_session_with_trace_mutation + save_store），\
         实际 {} 次（明细 {spent:?}）——多一次就是写放大回来了",
        spent.total()
    );

    // 非空测试防线：这次 turn 必须真的落盘。
    let mut reloaded = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    assert_eq!(
        reloaded.snapshot(Some("s-amp"), &[]).history.len(),
        2,
        "前提防线：turn 必须已落盘（user + assistant 两条）"
    );
}

// ---------------------------------------------------------------------------
// 契约 1-b：全库入口（真实病灶）必须收敛到 1 次提交边界，禁止直连整包写
// ---------------------------------------------------------------------------

/// `create_workspace` → `save_to_backend()` 是全库落盘入口的真实病灶
/// （`session/store.rs:2035`；ADR 0099 背景：18 个同类入口 × 整库 5.1 MB）。
///
/// 阶段 0 要求：该入口必须**只走 1 次 `commit_boundary`**，且 `save_store` 增量 ≤1
/// （若实现仍需整包写，必须经提交边界而非直连 `save_store`）。
///
/// 红相（交付时）：`save_to_backend` 直接调 `save_store`（`store.rs:2409` 附近），
/// `commit_boundary` 增量 0 ⇒ 断言失败；这正是 102 MB/turn 写放大的结构性入口。
#[test]
fn full_store_entry_routes_through_commit_boundary_once() {
    // Arrange
    let dir = TempDir::new("amp-fullstore");
    let db = dir.db();
    let workspace_root = dir.path.join("workspace-root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let (backend, save_store, commit_boundary, upsert_session, psmtm) =
        CountingBackend::new(&db, dir.path.join("attachments"));
    let mut store = SessionStore::with_backend(Box::new(backend));
    let before = read_counts(&save_store, &commit_boundary, &upsert_session, &psmtm);

    // Act：走全库落盘入口。
    store
        .create_workspace("amp-ws", workspace_root.to_str().expect("utf-8 root"))
        .expect("create_workspace 必须成功");

    // Assert ①：必须恰好穿越一次单一提交边界。
    let spent = delta(
        read_counts(&save_store, &commit_boundary, &upsert_session, &psmtm),
        before,
    );
    assert_eq!(
        spent.commit_boundary, 1,
        "ADR 0099 阶段 0 / I1：全库入口（create_workspace → save_to_backend）必须恰好穿越 \
         1 次 SessionBackend::commit_boundary，实际 {} 次；今天该入口直连 save_store\
         （store.rs:2035 → :2409），是 18 个全库写放大入口的典型（明细 {spent:?}）",
        spent.commit_boundary
    );

    // Assert ②：若仍需整包写，必须经提交边界，且全库写 ≤1 次/入口。
    assert!(
        spent.save_store <= 1,
        "ADR 0099 阶段 0 / I1：全库入口的整包写（save_store）必须 ≤1 次，实际 {} 次（明细 {spent:?}）",
        spent.save_store
    );

    // 前置防线：这次提交必须真的落盘，否则「1 次提交边界」是空转。
    assert!(
        store_metadata_contains(&db, "workspaces", "amp-ws"),
        "前提防线：create_workspace 的提交必须真的落盘（store_metadata.workspaces 含 amp-ws）"
    );
}

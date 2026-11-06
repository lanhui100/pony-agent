//! Ask（`builtin:ask_user` / Interaction 控制请求）**持久化 + 生命周期收口**冻结验收测试
//! —— 红相（L2-T 功能契约 + L2-AT 对抗破坏）。
//!
//! 契约真值源（只读，Lead 冻结）：
//! * `.dev-team/contract-matrix-ask-persistence.md` §2 数据模型 / §3 状态机 / §4 CAS 语义 /
//!   §8 验收映射（本文测试名与 §8 一一对应）。
//! * `docs/decisions/proposed/0074-ask-persistence-lifecycle.md`。
//! * 协议空桩：`crates/pony-agent-core/src/agent/ask_persistence.rs`
//!   （`PersistedAsk` / `AskLifecycleState` / `AskPersistenceStore` /
//!   `open_ask_store` / `open_in_memory_ask_store`）。
//!
//! 被测形态（Lead 裁决）：未决 Ask 从「dispatcher 进程内存 + 60s 墙钟 TTL」改为
//! 「会话持久化 + 生命周期状态机」。因此：
//! 1. 未决 Ask 落盘 ⇒ **新实例（模拟进程重启）**重新打开同一 store 后仍可列出、可作答；
//! 2. 墙钟不再是产物语义 —— `created_at_ms` 只用于展示/排序，**不**决定可否作答；
//! 3. 状态机 `Pending → Answered | Cancelled | Superseded | Orphaned`，终态不可再迁移；
//! 4. `version` / `nonce` 随记录落盘 ⇒ 跨实例 CAS 仍拒绝重放与版本不符；
//! 5. 会话内出现新用户消息 ⇒ 旧 Pending 转 `Superseded`，不再出现在待决列表；
//! 6. 显式忽略 ⇒ `Cancelled`（不是 `Superseded`、不是过期）。
//!
//! 红相纪律（test-expert「零假测试」律）：
//! * 本文件只引用**已存在**的 pub 符号，且**不**自行实现 `AskPersistenceStore` —— 统一经
//!   契约冻结的工厂 `open_ask_store` / `open_in_memory_ask_store` 构造 store。
//! * 空桩期工厂以 `unimplemented!()` 收场：`open_store` 把该 panic **收敛为 `Err`**，再由
//!   `expect_ok` 以**断言失败**（`assert!`）呈现——红相因此是断言失败，而非编译错误或
//!   逃逸的 panic。
//! * 每条用例断言都锚定**可观察性质**（状态、版本、待决列表、可选迁移），不做 `is_ok()` 空断言。
//!
//! 运行：`cargo test -p pony-agent-core --test ask_persistence_acceptance`

use pony_agent_core::agent::ask_persistence::{
    open_ask_store, open_in_memory_ask_store, AskLifecycleState, AskPersistenceResult,
    AskPersistenceStore, PersistedAsk,
};
use pony_agent_core::agent::tool_runtime::PendingControlRequestKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

// ─────────────────────────────────────────────────────────────────────────────
// 测试基建（均为已存在 pub API 之上的最小封装；不含任何 SUT 实现）
// ─────────────────────────────────────────────────────────────────────────────

/// 隔离临时目录（谁创建谁清理：`Drop` 删除，避免脏数据污染后续运行）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "pony-ask-persist-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// 会话 SQLite 库路径（契约 §2：`sessions.db`，与 sessions/normalized_* 同库）。
    fn db(&self) -> PathBuf {
        self.path.join("sessions.db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// 串行化「临时抑制 panic hook」的临界区：仅用于把空桩 `unimplemented!()` 的 panic
/// 收敛成可控错误值，避免红相输出被误读为「panic 逃逸」。
static PANIC_HOOK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 在**抑制默认 panic 打印**的前提下捕获 panic，返回 `Err(payload)` 而不是让 panic 逃逸。
fn catch_stub_panic<T>(
    operation: impl FnOnce() -> T,
) -> std::result::Result<T, Box<dyn std::any::Any + Send>> {
    let _guard = PANIC_HOOK_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
    std::panic::set_hook(previous_hook);
    outcome
}

/// 打开**持久化** Ask store；空桩期 `unimplemented!()` 的 panic 被收敛成 `Err`，
/// 从而让红相以断言失败（而不是 panic 逃逸 / 编译错误）呈现。
fn open_store(db_path: &Path) -> AskPersistenceResult<Box<dyn AskPersistenceStore>> {
    match catch_stub_panic(|| open_ask_store(db_path)) {
        Ok(result) => result,
        Err(payload) => Err(format!(
            "open_ask_store panicked (protocol stub not implemented yet): {}",
            panic_text(payload.as_ref())
        )),
    }
}

/// 打开**进程内** Ask store（同一收敛口径）。
#[allow(dead_code)]
fn open_memory_store() -> AskPersistenceResult<Box<dyn AskPersistenceStore>> {
    match catch_stub_panic(open_in_memory_ask_store) {
        Ok(result) => result,
        Err(payload) => Err(format!(
            "open_in_memory_ask_store panicked (protocol stub not implemented yet): {}",
            panic_text(payload.as_ref())
        )),
    }
}

/// 断言「必须成功」，失败时给出**契约条目 + 原因**（断言失败，非 `unwrap` 逃逸）。
fn expect_ok<T>(context: &str, result: AskPersistenceResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            assert!(false, "{context} —— 实际错误: {error}");
            unreachable!("assert! above always fails on Err")
        }
    }
}

/// 断言「必须被拒绝」，失败时给出契约条目；成功时返回被接受的记录以便定位问题。
fn expect_err<T: std::fmt::Debug>(context: &str, result: AskPersistenceResult<T>) -> String {
    match result {
        Err(error) => error,
        Ok(value) => {
            assert!(
                false,
                "{context} —— 该迁移/写入必须被拒绝，实际被接受: {value:?}"
            );
            unreachable!("assert! above always fails on Ok")
        }
    }
}

/// 一条 Pending Ask 记录（契约 §2 字段全集）。
fn pending_ask(request_id: &str, session_id: &str, created_at_ms: u64) -> PersistedAsk {
    PersistedAsk {
        request_id: request_id.to_string(),
        version: 1,
        nonce: format!("nonce-{request_id}"),
        session_id: session_id.to_string(),
        run_id: Some("run-1".to_string()),
        turn_id: "turn-1".to_string(),
        call_id: "call-1".to_string(),
        kind: PendingControlRequestKind::Interaction,
        prompt: "继续？".to_string(),
        options: vec!["是".to_string(), "否".to_string()],
        state: AskLifecycleState::Pending,
        created_at_ms,
        answered_at_ms: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// P1：未决 Ask 落盘，重启（新实例）后可列出
// ─────────────────────────────────────────────────────────────────────────────

/// P1（契约 §2/§8）：`upsert` 落盘后进程结束；**新实例**重开同一库仍能 `load_pending` 到
/// 同一 `request_id` / `version` / `nonce` / `prompt` / `options`，且按 `created_at_ms` 升序、
/// 按会话分区。
#[test]
fn persisted_ask_roundtrip_survives_process_restart() {
    let dir = TempDir::new("roundtrip");
    let db = dir.db();

    // 进程 A：两个未决 Ask（不同 created_at_ms 以锚定排序）+ 另一个会话的一条。
    {
        let store = expect_ok(
            "P1：`open_ask_store(db)` 必须返回可用的持久化 store（空桩期未实现）",
            open_store(&db),
        );
        expect_ok(
            "P1：`upsert` 必须成功写入未决 Ask",
            store.upsert(&pending_ask("ask-late", "session-1", 2_000)),
        );
        expect_ok(
            "P1：`upsert` 必须成功写入未决 Ask",
            store.upsert(&pending_ask("ask-early", "session-1", 1_000)),
        );
        expect_ok(
            "P1：`upsert` 必须成功写入未决 Ask",
            store.upsert(&pending_ask("ask-other", "session-2", 1_500)),
        );
        // 进程 A 结束：store 丢弃（后台连接关闭），内存态随之消失。
    }

    // 进程 B（模拟重启）：仅凭磁盘重新打开。
    let store_b = expect_ok(
        "P1：重启后 `open_ask_store` 必须仍能打开同一库（迁移向前兼容）",
        open_store(&db),
    );
    let recovered = expect_ok(
        "P1：重启后 `load_pending` 必须读出磁盘上的未决 Ask",
        store_b.load_pending("session-1"),
    );

    assert_eq!(
        recovered
            .iter()
            .map(|ask| ask.request_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ask-early", "ask-late"],
        "P1：重启后必须按 created_at_ms 升序列出该会话的全部未决 Ask（内存态永远无法满足此项）"
    );
    let first = &recovered[0];
    assert_eq!(first.session_id, "session-1", "P1：session_id 必须原样落盘");
    assert_eq!(first.version, 1, "P1：version 必须原样落盘（跨实例 CAS 前提）");
    assert_eq!(
        first.nonce, "nonce-ask-early",
        "P1：nonce 必须原样落盘（跨实例防重放前提）"
    );
    assert_eq!(first.prompt, "继续？", "P1：prompt 必须原样落盘");
    assert_eq!(
        first.options,
        vec!["是".to_string(), "否".to_string()],
        "P1：options 必须原样落盘（string 数组）"
    );
    assert_eq!(first.state, AskLifecycleState::Pending, "P1：state 必须为 pending");
    assert_eq!(first.created_at_ms, 1_000, "P1：created_at_ms 必须原样落盘");

    assert_eq!(
        expect_ok(
            "P1：`load_pending` 必须按会话分区返回",
            store_b.load_pending("session-2")
        )
        .len(),
        1,
        "P1：会话分区必须生效（session-2 只有 1 条未决）"
    );
    assert!(
        expect_ok(
            "P1：未知会话 `load_pending` 必须返回空列表而不是错误",
            store_b.load_pending("session-does-not-exist")
        )
        .is_empty(),
        "P1：未知会话必须返回空列表（宿主据此判定“无未决”）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P2：状态机（终态不可再迁移 / 显式忽略 → Cancelled）
// ─────────────────────────────────────────────────────────────────────────────

/// P2（契约 §3/§8）：`Answered` 是终态——重放、终态互迁、以及任何回退到 `Pending` 的
/// 「复活」尝试都必须被拒绝，且终态记录不得再出现在待决列表。
#[test]
fn state_machine_rejects_terminal_transitions() {
    let dir = TempDir::new("terminal");
    let db = dir.db();
    let store = expect_ok("P2：必须能打开持久化 store", open_store(&db));
    expect_ok(
        "P2：前置 upsert 必须成功",
        store.upsert(&pending_ask("ask-1", "session-1", 1_000)),
    );

    let answered = expect_ok(
        "P2：`Pending → Answered` 必须成功",
        store.transition(
            "ask-1",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert_eq!(
        answered.state,
        AskLifecycleState::Answered,
        "P2：迁移后状态必须为 answered"
    );
    assert_eq!(answered.version, 2, "P2：迁移必须把 version 提升到 2");
    assert!(
        answered.answered_at_ms.is_some(),
        "P2：`Pending → Answered` 必须写入 answered_at_ms"
    );
    assert!(
        expect_ok(
            "P2：已作答记录不得再出现在待决列表",
            store.load_pending("session-1")
        )
        .is_empty(),
        "P2：`Answered` 记录不得出现在待决列表（宿主不得重复呈现）"
    );

    // 终态 → 任意（含回退到 Pending 的越迁）一律拒绝。契约 §3 末行。
    for to in [
        AskLifecycleState::Pending,
        AskLifecycleState::Answered,
        AskLifecycleState::Cancelled,
        AskLifecycleState::Superseded,
        AskLifecycleState::Orphaned,
    ] {
        let from_answered = format!("P2：`Answered → {to:?}` 属非法越迁，必须被拒绝");
        let from_pending = format!("P2：谎称 `Pending` 的重放（→ {to:?}）必须被拒绝");
        expect_err(
            &from_answered,
            store.transition("ask-1", AskLifecycleState::Answered, to, 2),
        );
        expect_err(
            &from_pending,
            store.transition("ask-1", AskLifecycleState::Pending, to, 1),
        );
    }

    // 复活检查：拒绝之后记录仍不得回到待决列表。
    assert!(
        expect_ok(
            "P2：被拒绝的越迁不得改变记录状态",
            store.load_pending("session-1")
        )
        .is_empty(),
        "P2：非法越迁被拒后，记录不得“复活”进待决列表"
    );
}

/// P2（契约 §3/§8）：显式忽略（`Pending → Cancelled`）必须是**独立终态**，与
/// `Superseded`（新消息取代）不可混同；版本前移、字段落盘、且重启后仍不可作答。
#[test]
fn explicit_ignore_transitions_to_cancelled() {
    let dir = TempDir::new("cancelled");
    let db = dir.db();

    {
        let store = expect_ok("P2：必须能打开持久化 store", open_store(&db));
        expect_ok(
            "P2：前置 upsert 必须成功",
            store.upsert(&pending_ask("ask-ignore", "session-1", 1_000)),
        );

        let cancelled = expect_ok(
            "P2：显式忽略必须走 `Pending → Cancelled`",
            store.transition(
                "ask-ignore",
                AskLifecycleState::Pending,
                AskLifecycleState::Cancelled,
                1,
            ),
        );
        assert_eq!(
            cancelled.state,
            AskLifecycleState::Cancelled,
            "P2：忽略后的终态必须是 `Cancelled`（不得是 Superseded/Orphaned/Answered）"
        );
        assert_eq!(cancelled.version, 2, "P2：忽略必须把 version 提升到 2");
        assert!(
            cancelled.answered_at_ms.is_none(),
            "P2：忽略（Cancelled）不是作答，不得写 answered_at_ms"
        );
        assert!(
            expect_ok(
                "P2：已忽略记录不得再出现在待决列表",
                store.load_pending("session-1")
            )
            .is_empty(),
            "P2：`Cancelled` 记录不再出现在待决列表（面板据此关闭且不再出现）"
        );
    }

    // 重启后：终态仍在磁盘上 —— 不能再作答、也不能再被“取代”成第二条终态。
    let store_b = expect_ok("P2：重启后必须能打开同一库", open_store(&db));
    expect_err(
        "P2：重启后对 `Cancelled` 记录的作答必须被拒绝（终态不可再迁移）",
        store_b.transition(
            "ask-ignore",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert_eq!(
        expect_ok(
            "P2：重启后新用户消息不得把已 Cancelled 的记录二次迁移",
            store_b.supersede_pending_on_user_message("session-1", 9_999)
        ),
        0,
        "P2：已是终态的忽略记录不得再被 Supersede（终态互斥）"
    );

    // 终态记录再执行「合法边」也必须被拒绝，且拒绝理由必须是契约 §4 的稳定错误码
    // （`load(request_id)` 读面本波不提供，故用稳定错误码锚定终态身份）。
    let terminal_conflict = expect_err(
        "P2：终态记录再执行 `Cancelled → Answered`（最新 version=2）必须被拒绝",
        store_b.transition(
            "ask-ignore",
            AskLifecycleState::Cancelled,
            AskLifecycleState::Answered,
            2,
        ),
    );
    assert!(
        terminal_conflict.contains("ask_state_conflict")
            || terminal_conflict.contains("ask_version_mismatch"),
        "P2：终态拒绝必须给出契约 §4 稳定错误码（ask_state_conflict / ask_version_mismatch），实际: {terminal_conflict}"
    );

    // 隔离性：同会话中新出现的未决 Ask 不受历史终态影响。
    expect_ok(
        "P2：新 Ask upsert 必须成功",
        store_b.upsert(&pending_ask("ask-next", "session-1", 5_000)),
    );
    assert_eq!(
        expect_ok(
            "P2：新 Ask 必须可列出",
            store_b.load_pending("session-1")
        )
        .iter()
        .map(|ask| ask.request_id.as_str())
        .collect::<Vec<_>>(),
        vec!["ask-next"],
        "P2：历史终态不得污染新的待决列表"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P3：跨实例 CAS（重放 / 版本不符）
// ─────────────────────────────────────────────────────────────────────────────

/// P3（契约 §4/§8）：进程 A 创建的 Ask，在进程 B（重启后新实例）作答成功后，任何实例重放
/// **同一** `(request_id, version)` 必须被拒绝——CAS 事实来自磁盘记录而不是进程内存。
#[test]
fn cas_rejects_replay_after_restart() {
    let dir = TempDir::new("replay");
    let db = dir.db();

    // 进程 A 创建。
    {
        let store = expect_ok("P3：必须能打开持久化 store", open_store(&db));
        expect_ok(
            "P3：前置 upsert 必须成功",
            store.upsert(&pending_ask("ask-replay", "session-1", 1_000)),
        );
    }

    // 进程 B 作答（version 1 → 2）。
    {
        let store_b = expect_ok("P3：重启后必须能打开同一库", open_store(&db));
        let answered = expect_ok(
            "P3：重启后必须能凭磁盘记录作答（跨进程 CAS 正向）",
            store_b.transition(
                "ask-replay",
                AskLifecycleState::Pending,
                AskLifecycleState::Answered,
                1,
            ),
        );
        assert_eq!(answered.version, 2, "P3：作答后 version 必须为 2");
        assert_eq!(answered.state, AskLifecycleState::Answered, "P3：作答后状态");
    }

    // 进程 C 重放进程 B 已经消费过的 (Pending, version=1)。
    let store_c = expect_ok("P3：第三个实例必须能打开同一库", open_store(&db));
    expect_err(
        "P3：重启后新实例必须拒绝重放（`version`/`nonce` 随记录落盘，不是进程内存）",
        store_c.transition(
            "ask-replay",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert!(
        expect_ok(
            "P3：重放被拒后记录不得回到待决列表",
            store_c.load_pending("session-1")
        )
        .is_empty(),
        "P3：重放被拒后不得复活记录"
    );
    expect_err(
        "P3：对未知 request_id 的作答必须 fail-closed（ask_not_found 语义）",
        store_c.transition(
            "ask-never-existed",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
}

/// P3（契约 §4/§8）：版本不符必须被拒绝，而**正确版本**仍然成功——用因果对照证明拒绝理由
/// 就是版本，而不是「什么都没实现」的放行/拒绝副作用。
#[test]
fn cas_rejects_version_mismatch_after_restart() {
    let dir = TempDir::new("version");
    let db = dir.db();
    {
        let store = expect_ok("P3：必须能打开持久化 store", open_store(&db));
        expect_ok(
            "P3：前置 upsert 必须成功",
            store.upsert(&pending_ask("ask-version", "session-1", 1_000)),
        );
    }

    let store_b = expect_ok("P3：重启后必须能打开同一库", open_store(&db));
    // 版本不符：磁盘记录是 1，谎称 99。
    expect_err(
        "P3：`expected_version` 与磁盘记录不符时必须拒绝（ask_version_mismatch 语义）",
        store_b.transition(
            "ask-version",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            99,
        ),
    );
    // 版本过低（陈旧视图）同样拒绝。
    expect_err(
        "P3：陈旧低版本必须被拒绝（防版本回退）",
        store_b.transition(
            "ask-version",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            0,
        ),
    );
    assert_eq!(
        expect_ok(
            "P3：被拒绝的版本不符尝试不得改变记录",
            store_b.load_pending("session-1")
        )
        .len(),
        1,
        "P3：版本不符被拒后记录仍必须处于 pending"
    );

    // 因果对照：正确版本必须成功（证明拒绝不是“一律拒绝”）。
    let answered = expect_ok(
        "P3：正确版本必须成功（否则上一条断言的拒绝理由不成立）",
        store_b.transition(
            "ask-version",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert_eq!(answered.version, 2, "P3：成功作答后 version 必须前移");

    // 重启后以旧版本作答依旧拒绝（version 已随记录前移，磁盘为真值源）。
    let store_c = expect_ok("P3：第三个实例必须能打开同一库", open_store(&db));
    expect_err(
        "P3：重启后旧版本作答必须被拒绝（版本前移已落盘）",
        store_c.transition(
            "ask-version",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P4/P5：新用户消息取代未决 Ask；P6：墙钟不再决定生死
// ─────────────────────────────────────────────────────────────────────────────

/// P4/P5（契约 §3/§8）：会话内出现新用户消息 ⇒ 该会话所有 `Pending` Ask 转 `Superseded`
/// 且不再出现在待决列表；跨会话不得误伤；重启后仍然是被取代状态。
#[test]
fn new_user_message_supersedes_pending_ask() {
    let dir = TempDir::new("supersede");
    let db = dir.db();

    {
        let store = expect_ok("P4：必须能打开持久化 store", open_store(&db));
        for (id, session, created) in [
            ("ask-a", "session-1", 1_000u64),
            ("ask-b", "session-1", 1_100),
            ("ask-other", "session-2", 1_200),
        ] {
            expect_ok(
                "P4：前置 upsert 必须成功",
                store.upsert(&pending_ask(id, session, created)),
            );
        }
        assert_eq!(
            expect_ok(
                "P4：前置条件——两条未决 Ask 必须可列出",
                store.load_pending("session-1")
            )
            .len(),
            2,
            "P4：前置条件失败：session-1 应有 2 条未决 Ask"
        );

        let moved = expect_ok(
            "P4：`supersede_pending_on_user_message` 必须成功",
            store.supersede_pending_on_user_message("session-1", 2_000),
        );
        assert_eq!(
            moved, 2,
            "P4：新用户消息必须把该会话**全部** Pending Ask 转 Superseded（返回迁移条数）"
        );
        assert!(
            expect_ok(
                "P4：被取代的 Ask 不得再出现在待决列表",
                store.load_pending("session-1")
            )
            .is_empty(),
            "P4：`Superseded` 记录必须从待决列表消失（客户端面板据此关闭）"
        );
        assert_eq!(
            expect_ok(
                "P4：跨会话不得误伤",
                store.load_pending("session-2")
            )
            .len(),
            1,
            "P4：取代只作用于同一会话（session-2 的未决 Ask 必须保留）"
        );
        expect_err(
            "P4：已被取代的记录不能再被作答（`Pending` 前置不再成立）",
            store.transition(
                "ask-a",
                AskLifecycleState::Pending,
                AskLifecycleState::Answered,
                1,
            ),
        );
        assert_eq!(
            expect_ok(
                "P4：无 Pending 时再次取代必须为 0（幂等）",
                store.supersede_pending_on_user_message("session-1", 3_000)
            ),
            0,
            "P4：已无待决 Ask 时第二次取代必须返回 0"
        );
    }

    // 重启后：取代是**落盘**的生命周期事实，不是进程内标记。
    let store_b = expect_ok("P4：重启后必须能打开同一库", open_store(&db));
    assert!(
        expect_ok(
            "P4：重启后仍不得列出被取代的 Ask",
            store_b.load_pending("session-1")
        )
        .is_empty(),
        "P4：重启后 `Superseded` 必须仍然是终态（持久化，而非内存标记）"
    );
    expect_err(
        "P4：重启后作答被取代的记录必须被拒绝",
        store_b.transition(
            "ask-b",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
}

/// P6（契约 §2/§4/§8）：**墙钟不再决定生死**。
///
/// * 持久化记录里没有 `expiresAtMs`（只有用于展示/排序的 `createdAtMs`）；
/// * 创建很久之后仍可作答（时间不是拒绝理由）；
/// * 纯时间驱动的 GC 不得回收 `Pending`（只有会话归档才产生 `Orphaned`）；
/// * GC 只作用于被归档会话，跨会话不误伤。
#[test]
fn no_wall_clock_expiry_path() {
    let dir = TempDir::new("no-expiry");
    let db = dir.db();
    let store = expect_ok("P6：必须能打开持久化 store", open_store(&db));

    // 很久以前创建（相当于旧语义下早已“过期”数十次）。
    const CREATED_AT_MS: u64 = 1_000;
    const TEN_YEARS_LATER_MS: u64 = CREATED_AT_MS + 10 * 365 * 24 * 60 * 60 * 1_000;
    expect_ok(
        "P6：前置 upsert 必须成功",
        store.upsert(&pending_ask("ask-old", "session-1", CREATED_AT_MS)),
    );

    let pending = expect_ok(
        "P6：未决 Ask 必须可列出",
        store.load_pending("session-1"),
    );
    assert_eq!(pending.len(), 1, "P6：前置条件失败：应有一条未决 Ask");

    // ① 线缆形状：记录不得携带 expiresAtMs（旧 TTL 语义已退出产品面）。
    let wire = serde_json::to_value(&pending[0]).expect("PersistedAsk must serialize");
    assert!(
        wire.get("expiresAtMs").is_none() && wire.get("expires_at_ms").is_none(),
        "P6：持久化记录不得携带 expiresAtMs / expires_at_ms（契约 §2 明令该字段不出现）；实际: {wire}"
    );
    assert_eq!(
        wire.get("createdAtMs"),
        Some(&serde_json::json!(CREATED_AT_MS)),
        "P6：记录必须携带 createdAtMs（仅用于展示/排序，非时效判定）；实际: {wire}"
    );
    assert_eq!(
        wire.get("state"),
        Some(&serde_json::json!("pending")),
        "P6：state 必须以 snake_case 落线缆；实际: {wire}"
    );

    // ② 纯时间 GC 不得回收 Pending（now 远在未来也不许）。
    let reclaimed = expect_ok(
        "P6：`gc_orphans` 必须可执行",
        store.gc_orphans(&[], TEN_YEARS_LATER_MS),
    );
    assert_eq!(
        reclaimed, 0,
        "P6：没有任何会话被归档时，纯时间推进不得回收 Pending Ask（墙钟退出产品语义）"
    );
    assert_eq!(
        expect_ok(
            "P6：纯时间 GC 之后未决 Ask 必须仍在",
            store.load_pending("session-1")
        )
        .len(),
        1,
        "P6：时间不得杀死未决 Ask"
    );

    // ③ 「很久以前创建」的 Ask 仍然可以作答。
    let answered = expect_ok(
        "P6：创建已“过期”十年的 Ask 仍必须可作答（时间不是拒绝理由）",
        store.transition(
            "ask-old",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert_eq!(
        answered.state,
        AskLifecycleState::Answered,
        "P6：作答必须成功并把状态推进到 answered"
    );

    // ④ 只有「会话归档」才产生 Orphaned，且按会话分区回收。
    expect_ok(
        "P6：第二个会话的前置 upsert 必须成功",
        store.upsert(&pending_ask("ask-kept", "session-2", CREATED_AT_MS)),
    );
    let orphaned = expect_ok(
        "P6：`gc_orphans` 对归档会话必须可执行",
        store.gc_orphans(&["session-archived".to_string()], TEN_YEARS_LATER_MS),
    );
    assert_eq!(
        orphaned, 0,
        "P6：归档列表不含 session-1/session-2 时不得回收（按会话精确匹配）"
    );
    assert_eq!(
        expect_ok(
            "P6：未归档会话的未决 Ask 必须保留",
            store.load_pending("session-2")
        )
        .len(),
        1,
        "P6：未归档会话的 Pending Ask 必须保留"
    );

    expect_ok(
        "P6：归档会话的前置 upsert 必须成功",
        store.upsert(&pending_ask("ask-orphan", "session-archived", CREATED_AT_MS)),
    );
    let orphaned = expect_ok(
        "P6：`gc_orphans` 必须回收归档会话的记录",
        store.gc_orphans(&["session-archived".to_string()], TEN_YEARS_LATER_MS),
    );
    assert_eq!(
        orphaned, 1,
        "P6：会话归档必须把该会话的记录转 `Orphaned`（返回条数）"
    );
    assert!(
        expect_ok(
            "P6：已归档会话的 Ask 不得再出现在待决列表",
            store.load_pending("session-archived")
        )
        .is_empty(),
        "P6：`Orphaned` 记录不得出现在待决列表"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// L2-AT 对抗破坏注入（确定性；无 sleep、无随机、无概率；见
// tests/acceptance/adversarial/ask_persistence/README.md）
// ─────────────────────────────────────────────────────────────────────────────

/// 对抗 A1：**并发双提交同一 request_id**。
///
/// 两个独立实例（各自的 store 句柄、各自的数据库连接）在 `Barrier` 处会合后同时提交
/// `Pending → Answered (version=1)`。断言：**恰好一个**成功、**恰好一个**被拒绝，
/// 且最终记录只前移一个版本、不再出现在待决列表。
#[test]
fn adversarial_concurrent_double_answer_yields_exactly_one_winner() {
    let dir = TempDir::new("race");
    let db = dir.db();
    {
        let store = expect_ok("A1：必须能打开持久化 store", open_store(&db));
        expect_ok(
            "A1：前置 upsert 必须成功",
            store.upsert(&pending_ask("ask-race", "session-1", 1_000)),
        );
    }

    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_worker| {
            let db = db.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || -> AskPersistenceResult<u64> {
                let store = open_store(&db)?;
                // 固定会合点：两侧都在此之前完成打开连接，再同时提交（无 sleep 竞猜）。
                barrier.wait();
                store
                    .transition(
                        "ask-race",
                        AskLifecycleState::Pending,
                        AskLifecycleState::Answered,
                        1,
                    )
                    .map(|record| record.version)
            })
        })
        .collect();

    let outcomes: Vec<AskPersistenceResult<u64>> = workers
        .into_iter()
        .map(|handle| match handle.join() {
            Ok(outcome) => outcome,
            Err(payload) => Err(format!("worker panicked: {}", panic_text(payload.as_ref()))),
        })
        .collect();

    let winners: Vec<u64> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok().copied())
        .collect();
    assert_eq!(
        winners,
        vec![2],
        "A1：并发双提交必须**恰好一个**成功（版本前移到 2）；实际结果: {outcomes:?}"
    );
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_err()).count(),
        1,
        "A1：并发双提交必须有**恰好一个**被 CAS 拒绝；实际结果: {outcomes:?}"
    );

    // 终局：记录已作答，任何实例都不能再消费。
    let store_after = expect_ok("A1：竞态后必须能重新打开库", open_store(&db));
    assert!(
        expect_ok(
            "A1：竞态后记录不得回到待决列表",
            store_after.load_pending("session-1")
        )
        .is_empty(),
        "A1：竞争后记录必须已离开待决列表"
    );
    expect_err(
        "A1：竞态后任何实例的再次提交都必须被拒绝",
        store_after.transition(
            "ask-race",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
}

/// 对抗 A2：**重放旧 version**（陈旧版本 + 已消费版本，固定顺序注入）。
///
/// 顺序：记录版本前移到 3 → 用 v1/v2 重放必须全部被拒绝 → 用 v3 必须成功（因果对照），
/// 重启后旧版本仍被拒绝。
#[test]
fn adversarial_replay_stale_version_after_restart_rejected() {
    let dir = TempDir::new("adv-replay");
    let db = dir.db();

    let mut ask = pending_ask("ask-stale", "session-1", 1_000);
    ask.version = 3; // 前移到 3，模拟「已被别的实例推进过」的陈旧视图。
    {
        let store = expect_ok("A2：必须能打开持久化 store", open_store(&db));
        expect_ok("A2：前置 upsert 必须成功", store.upsert(&ask));
    }

    let store_b = expect_ok("A2：重启后必须能打开同一库", open_store(&db));
    for stale in [1u64, 2] {
        expect_err(
            &format!("A2：用陈旧 version={stale} 重放必须被拒绝（防重放）"),
            store_b.transition(
                "ask-stale",
                AskLifecycleState::Pending,
                AskLifecycleState::Answered,
                stale,
            ),
        );
    }
    assert_eq!(
        expect_ok(
            "A2：陈旧版本重放被拒后记录必须保持 pending",
            store_b.load_pending("session-1")
        )
        .len(),
        1,
        "A2：所有陈旧版本重放都必须被拒绝且不改变状态"
    );

    // 因果对照：当前版本 3 必须成功。
    let answered = expect_ok(
        "A2：当前版本必须成功（否则前面的拒绝不能归因于版本）",
        store_b.transition(
            "ask-stale",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            3,
        ),
    );
    assert_eq!(
        answered.version, 4,
        "A2：成功迁移后版本必须前移到 4"
    );

    // 重启后重放「刚刚被消费的那个版本」依旧被拒绝。
    let store_c = expect_ok("A2：第三个实例必须能打开同一库", open_store(&db));
    expect_err(
        "A2：重启后重放已消费的 version=3 必须被拒绝",
        store_c.transition(
            "ask-stale",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            3,
        ),
    );
}

/// 对抗 A3：**非法状态越迁**（`Answered → Pending` 及终态互迁）。
///
/// 宿主唯一合法的"回退"念头就是让已作答的提问重新待决；本用例遍历全部目标状态，
/// 断言这些越迁一律被拒绝，且记录不会被「复活」进待决列表。
#[test]
fn adversarial_terminal_state_cannot_be_reverted_to_pending() {
    let dir = TempDir::new("adv-illegal");
    let db = dir.db();
    let store = expect_ok("A3：必须能打开持久化 store", open_store(&db));
    expect_ok(
        "A3：前置 upsert 必须成功",
        store.upsert(&pending_ask("ask-terminal", "session-1", 1_000)),
    );
    let answered = expect_ok(
        "A3：前置作答必须成功",
        store.transition(
            "ask-terminal",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert_eq!(answered.version, 2, "A3：前置条件——版本应前移到 2");

    // Answered → 任意（重点 Pending）全部非法。
    for to in [
        AskLifecycleState::Pending,
        AskLifecycleState::Answered,
        AskLifecycleState::Cancelled,
        AskLifecycleState::Superseded,
        AskLifecycleState::Orphaned,
    ] {
        expect_err(
            &format!("A3：`Answered → {to:?}` 非法越迁必须被拒绝"),
            store.transition("ask-terminal", AskLifecycleState::Answered, to, 2),
        );
    }

    // 即使谎称「当前仍是 Pending」（版本也对不上）也必须拒绝 —— 不存在任何回退通道。
    expect_err(
        "A3：以 Pending 为前置的越迁必须被拒绝（不存在回退到待决的通道）",
        store.transition(
            "ask-terminal",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            2,
        ),
    );

    // 记录没有被复活，且同会话的新 Ask 不受影响。
    assert!(
        expect_ok(
            "A3：越迁被拒后记录不得复活",
            store.load_pending("session-1")
        )
        .is_empty(),
        "A3：非法越迁被拒后，`Answered` 记录不得重新出现在待决列表"
    );
    expect_ok(
        "A3：新 Ask 必须可写入",
        store.upsert(&pending_ask("ask-fresh", "session-1", 2_000)),
    );
    assert_eq!(
        expect_ok(
            "A3：新 Ask 必须可列出",
            store.load_pending("session-1")
        )
        .iter()
        .map(|ask| ask.request_id.as_str())
        .collect::<Vec<_>>(),
        vec!["ask-fresh"],
        "A3：终态记录不得污染新的待决列表"
    );
}

/// 孤立记录（`Orphaned`）同样不可复活：归档会话被 GC 后，重启并以原版本作答必须被拒绝。
#[test]
fn adversarial_orphaned_ask_cannot_be_answered_after_restart() {
    let dir = TempDir::new("adv-orphan");
    let db = dir.db();
    {
        let store = expect_ok("A4：必须能打开持久化 store", open_store(&db));
        expect_ok(
            "A4：前置 upsert 必须成功",
            store.upsert(&pending_ask("ask-orphan", "session-gone", 1_000)),
        );
        assert_eq!(
            expect_ok(
                "A4：归档前记录必须为待决",
                store.gc_orphans(&["session-gone".to_string()], 2_000)
            ),
            1,
            "A4：归档会话的记录必须被 GC 转 Orphaned"
        );
    }
    let store_b = expect_ok("A4：重启后必须能打开同一库", open_store(&db));
    expect_err(
        "A4：重启后作答已 Orphaned 的记录必须被拒绝（终态不可迁移）",
        store_b.transition(
            "ask-orphan",
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
            1,
        ),
    );
    assert!(
        expect_ok(
            "A4：Orphaned 记录不得出现在待决列表",
            store_b.load_pending("session-gone")
        )
        .is_empty(),
        "A4：`Orphaned` 记录不得再被列出"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// T10 / L3-B BLOCKING 回归（红相）：会话隔离主键 + 作答作用域 + fail-closed
// ═════════════════════════════════════════════════════════════════════════════
//
// 契约依据（Lead 2026-11-06 原地改写）：`.dev-team/contract-matrix-ask-persistence.md` §2（复合主键
// `(session_id, request_id)`）、§4（`answer/cancel` 校验 `(request_id, version, session_id)`，
// nonce 由服务端自持、客户端不传；缺作用域 ⇒ `ask_scope_required`）、§5（`ask_list_pending` 无作用域 ⇒
// 空数组 + `ask_scope_required`；投影不含 nonce）、§7（持久化不可用必须 fail-closed，禁止回落内存态）。
//
// 红相意图：以下 5 条用例在**当前实现**下必须按断言失败（不是编译错误），它们是 executor-core 修复
// `crates/pony-agent-core/src/agent/ask_persistence.rs` / `control_plane/ask_plan_commands.rs` 的冻结门禁。
// 冻结口径（不做二次改签）：作用域必须由调用方显式提供——「不提供作用域 ⇒ 必须 Err」是本组用例的最小锚点。

/// 真实 guided dispatcher（内置注册表 + FakeClock + LegacyCompatiblePolicyEvaluator）。
fn t10_dispatcher() -> Arc<pony_agent_core::agent::dispatcher::GovernedDispatcher> {
    use pony_agent_core::agent::dispatcher::GovernedDispatcher;
    use pony_agent_core::agent::governed_executor::LegacyCompatiblePolicyEvaluator;
    use pony_agent_core::agent::tool_runtime::{FakeClock, RuntimeClock};
    use pony_agent_core::agent::tools::ToolRegistrySnapshot;

    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin tool registry must construct cleanly"),
    );
    let clock: Arc<dyn RuntimeClock> = Arc::new(FakeClock::new(1_000));
    let dispatcher = GovernedDispatcher::new(registry, clock);
    dispatcher.register_policy_evaluator(Arc::new(LegacyCompatiblePolicyEvaluator));
    Arc::new(dispatcher)
}

/// 通过真实受管派发路径抛出一个 Interaction（Ask）pending 控制请求，返回 (request_id, 快照)。
fn t10_dispatch_ask(
    dispatcher: &pony_agent_core::agent::dispatcher::GovernedDispatcher,
    session_id: &str,
    call_id: &str,
) -> (String, pony_agent_core::agent::tool_runtime::PendingControlRequest) {
    use pony_agent_core::agent::dispatcher::DispatchContext;
    use pony_agent_core::agent::tool_runtime::{InvocationOrigin, ToolDispatchRequest};

    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: "builtin:ask_user".to_string(),
            call_id: call_id.to_string(),
            arguments: serde_json::json!({
                "question": "继续？",
                "options": ["是", "否"],
                "description": "T10 安全红相"
            }),
        },
        &DispatchContext {
            session_id: Some(session_id.to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            host_control_available: true,
            ..Default::default()
        },
    );
    let request_id = outcome
        .control_outcome
        .expect("host-mediated ask must surface a WaitingHost control outcome")
        .request_id;
    let pending = dispatcher
        .pending_request(&request_id)
        .expect("dispatched ask must be persisted in the dispatcher mirror");
    (request_id, pending)
}

/// 宿主控制面（注入显式 dispatcher + 显式持久化 store，避免依赖默认会话库）。
fn t10_plane(
    dispatcher: Arc<pony_agent_core::agent::dispatcher::GovernedDispatcher>,
    store: Arc<dyn AskPersistenceStore>,
) -> pony_agent_core::agent::control_plane::HostControlPlane {
    use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;

    HostControlPlaneBuilder::new()
        .ask_dispatcher(dispatcher)
        .ask_store(store)
        .build()
}

/// 把 `ask_list_pending` 的 JSON 返回摊平成条目列表（兼容数组形状与 `{asks:[...]}` 形状）。
fn t10_entries(value: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(items) = value.as_array() {
        return items.clone();
    }
    for key in ["asks", "pendingAsks", "items"] {
        if let Some(items) = value.get(key).and_then(|inner| inner.as_array()) {
            return items.clone();
        }
    }
    Vec::new()
}

/// L3-B ①：`pending_asks` 必须是**复合主键 `(session_id, request_id)`**。
///
/// request_id 是进程内计数器（每个进程首个 Ask 恒为 `control-000000000000`），单列主键会让
/// session-B 的首个 Ask 覆盖 session-A 的未决 Ask（并把它改挂到 B、version 重置为 1，
/// 旧凭据重新生效）。本用例断言两个会话各自的未决 Ask 互不覆盖。
#[test]
fn security_cross_session_upsert_must_not_overwrite_another_sessions_pending_ask() {
    let dir = TempDir::new("sec-composite-pk");
    let db = dir.db();
    let store = expect_ok("SEC-PK：必须能打开持久化 store", open_store(&db));

    let mut ask_a = pending_ask("control-000000000000", "session-A", 1_000);
    ask_a.nonce = "nonce-A".to_string();
    ask_a.prompt = "A 的问题".to_string();
    let mut ask_b = pending_ask("control-000000000000", "session-B", 2_000);
    ask_b.nonce = "nonce-B".to_string();
    ask_b.prompt = "B 的问题".to_string();

    expect_ok(
        "SEC-PK：session-A 的 upsert 必须成功",
        store.upsert(&ask_a),
    );
    expect_ok(
        "SEC-PK：session-B 用同一 request_id upsert 必须成功（复合主键下两行共存）",
        store.upsert(&ask_b),
    );

    let pending_a = expect_ok(
        "SEC-PK：load_pending(session-A) 必须可读",
        store.load_pending("session-A"),
    );
    assert_eq!(
        pending_a.len(),
        1,
        "SEC-PK：session-B 写入同名 request_id 不得覆盖 session-A 的未决 Ask（契约 §2 复合主键 \
         `(session_id, request_id)`）；当前实现按 request_id 单列主键覆盖，A 的未决 Ask 已被改挂到 B"
    );
    assert_eq!(pending_a[0].session_id, "session-A", "SEC-PK：A 的记录必须仍属于 A");
    assert_eq!(
        pending_a[0].nonce, "nonce-A",
        "SEC-PK：A 的 CAS 凭据（nonce）不得被 B 的记录覆盖"
    );
    assert_eq!(pending_a[0].prompt, "A 的问题", "SEC-PK：A 的问题文本不得被覆盖");
    assert_eq!(pending_a[0].version, 1, "SEC-PK：A 的 version 不得被重置");

    let pending_b = expect_ok(
        "SEC-PK：load_pending(session-B) 必须可读",
        store.load_pending("session-B"),
    );
    assert_eq!(pending_b.len(), 1, "SEC-PK：B 自己的未决 Ask 必须可读");
    assert_eq!(pending_b[0].session_id, "session-B");
    assert_eq!(pending_b[0].nonce, "nonce-B");

    // 重启（新实例、同库）后仍然各自独立——复合主键必须是磁盘事实，不是内存态。
    drop(store);
    let reopened = expect_ok("SEC-PK：重启后必须能打开同一库", open_store(&db));
    assert_eq!(
        expect_ok(
            "SEC-PK：重启后 load_pending(session-A) 必须可读",
            reopened.load_pending("session-A")
        )
        .iter()
        .map(|ask| ask.nonce.as_str())
        .collect::<Vec<_>>(),
        vec!["nonce-A"],
        "SEC-PK：重启后 A 的未决 Ask（含凭据）不得被 B 的同名记录顶替"
    );
}

/// L3-B ②a：`ask_list_pending` **无会话作用域必须返回空数组**（契约 §5），不得把全库
/// requestId/version 泄露给无作用域宿主调用。
#[test]
fn security_unscoped_list_pending_must_not_leak_other_sessions_credentials() {
    use pony_agent_core::agent::ask_persistence::AskPersistenceStore as _;

    let dir = TempDir::new("sec-list-leak");
    let db = dir.db();
    let store: Arc<dyn AskPersistenceStore> =
        Arc::from(expect_ok("SEC-LIST：必须能打开持久化 store", open_store(&db)));
    expect_ok(
        "SEC-LIST：session-A 的 upsert 必须成功",
        store.upsert(&pending_ask("control-000000000000", "session-A", 1_000)),
    );
    expect_ok(
        "SEC-LIST：session-B 的 upsert 必须成功",
        store.upsert(&pending_ask("control-000000000001", "session-B", 2_000)),
    );
    // 让平面同时“认识”两个会话：会话候选来自 dispatcher 内存镜像 + 会话库。
    // （若只写持久化层，宿主根本不知道这两个会话存在，断言会因“候选为空”而假绿。）
    let dispatcher = t10_dispatcher();
    let _ = t10_dispatch_ask(&dispatcher, "session-A", "call-sec-list-a");
    let _ = t10_dispatch_ask(&dispatcher, "session-B", "call-sec-list-b");
    let plane = t10_plane(Arc::clone(&dispatcher), Arc::clone(&store));

    // 无作用域：不得泄露任何会话的 (requestId, version, nonce)。
    let unscoped = plane.list_pending_asks(None);
    let leaked = t10_entries(&unscoped);
    assert!(
        leaked.is_empty(),
        "SEC-LIST：无会话作用域的 ask_list_pending 必须返回空数组（并携带 ask_scope_required），\
         实际返回了 {} 条跨会话记录：{unscoped}",
        leaked.len()
    );

    // 有作用域：只返回该会话的 Pending（也不得掺入其它会话）。
    let scoped_a = t10_entries(&plane.list_pending_asks(Some("session-A")));
    assert_eq!(
        scoped_a.len(),
        1,
        "SEC-LIST：作用域 session-A 必须恰好返回 1 条，实际：{scoped_a:?}"
    );
    assert_eq!(
        scoped_a[0].get("sessionId").and_then(serde_json::Value::as_str),
        Some("session-A"),
        "SEC-LIST：作用域查询不得返回其它会话的记录：{:?}",
        scoped_a[0]
    );
}

/// L3-B ②b：`ask_list_pending` 的投影**不得包含 `nonce`**（服务端自持 CAS 凭据，客户端不传）。
#[test]
fn security_ask_projection_must_not_expose_nonce() {
    let dir = TempDir::new("sec-nonce-projection");
    let db = dir.db();
    let store: Arc<dyn AskPersistenceStore> =
        Arc::from(expect_ok("SEC-NONCE：必须能打开持久化 store", open_store(&db)));
    expect_ok(
        "SEC-NONCE：upsert 必须成功",
        store.upsert(&pending_ask("control-000000000000", "session-A", 1_000)),
    );
    let plane = t10_plane(t10_dispatcher(), Arc::clone(&store));

    let entries = t10_entries(&plane.list_pending_asks(Some("session-A")));
    assert_eq!(entries.len(), 1, "SEC-NONCE：作用域查询必须返回该条未决 Ask");
    let entry = &entries[0];
    assert!(
        entry.get("nonce").is_none(),
        "SEC-NONCE：ask 投影不得暴露 nonce（服务端自持、客户端不传）；实际投影：{entry}"
    );
    assert!(
        entry.get("requestId").is_some() && entry.get("version").is_some(),
        "SEC-NONCE：投影仍必须携带 requestId/version（CAS 版本由客户端回传）：{entry}"
    );
}

/// L3-B ②c：**无会话作用域的作答必须 fail-closed**（`ask_scope_required` / `ask_state_conflict`），
/// 且被拒绝后记录仍为 `Pending`。
#[test]
fn security_answer_without_session_scope_must_fail_closed() {
    let dir = TempDir::new("sec-scope-answer");
    let db = dir.db();
    let store: Arc<dyn AskPersistenceStore> =
        Arc::from(expect_ok("SEC-SCOPE：必须能打开持久化 store", open_store(&db)));
    expect_ok(
        "SEC-SCOPE：upsert 必须成功",
        store.upsert(&pending_ask("control-000000000000", "session-A", 1_000)),
    );
    let plane = t10_plane(t10_dispatcher(), Arc::clone(&store));

    // 调用方没有提供会话作用域：必须拒绝，而不是按 request_id 全库定位后 CAS 成功。
    let outcome = plane.answer_ask("control-000000000000", 1, serde_json::json!("是"));
    let error = match outcome {
        Ok(value) => {
            assert!(
                false,
                "SEC-SCOPE：无会话作用域的作答必须 fail-closed（ask_scope_required），\
                 当前实现直接按 request_id 全库 CAS 成功并返回 {value} ⇒ 任意会话可跨会话作答"
            );
            unreachable!("assert above always fails on Ok")
        }
        Err(error) => error,
    };
    assert!(
        error.contains("ask_scope_required") || error.contains("ask_state_conflict"),
        "SEC-SCOPE：拒绝必须携带稳定错误码（ask_scope_required / ask_state_conflict），实际：{error}"
    );

    let after = expect_ok(
        "SEC-SCOPE：被拒绝后仍必须能读回记录",
        store.load_pending("session-A"),
    );
    assert_eq!(after.len(), 1, "SEC-SCOPE：被拒绝的作答不得移除记录");
    assert_eq!(
        after[0].state,
        AskLifecycleState::Pending,
        "SEC-SCOPE：被拒绝的作答不得改变记录状态"
    );
    assert_eq!(after[0].version, 1, "SEC-SCOPE：被拒绝的作答不得自增版本");
}

/// L3-B ③：**持久化记录缺失时必须 fail-closed**，禁止回落内存镜像并返回 `Ok`（否则答案永不落盘）。
#[test]
fn security_missing_durable_record_must_not_fall_back_to_memory_mirror() {
    use pony_agent_core::agent::ask_persistence::AskPersistenceStore as _;
    use pony_agent_core::agent::tool_runtime::PendingControlRequestState;

    let store: Arc<dyn AskPersistenceStore> = Arc::from(expect_ok(
        "SEC-FAILCLOSED：必须能打开进程内 store",
        open_in_memory_ask_store(),
    ));
    let dispatcher = t10_dispatcher();
    // 只存在于 dispatcher 内存镜像（未 upsert 到持久化层）的 Ask。
    let (request_id, pending) = t10_dispatch_ask(&dispatcher, "session-A", "call-sec-failclosed");
    let plane = t10_plane(Arc::clone(&dispatcher), Arc::clone(&store));

    let outcome = plane.answer_ask(&request_id, pending.version, serde_json::json!("是"));
    let error = match outcome {
        Ok(value) => {
            assert!(
                false,
                "SEC-FAILCLOSED：持久化层查不到该 Ask 时必须 fail-closed（ask_not_found），\
                 当前实现回落内存 CAS 并返回 Ok({value}) ⇒ 答案永不落盘、重启即丢"
            );
            unreachable!("assert above always fails on Ok")
        }
        Err(error) => error,
    };
    assert!(
        error.contains("ask_not_found") || error.contains("ask_state_conflict"),
        "SEC-FAILCLOSED：拒绝必须携带稳定错误码（ask_not_found / ask_state_conflict），实际：{error}"
    );
    // 内存镜像不得被“成功”消费（否则宿主会误以为作答已生效）。
    assert_eq!(
        dispatcher
            .pending_request(&request_id)
            .expect("mirror record must stay readable")
            .state,
        PendingControlRequestState::Pending,
        "SEC-FAILCLOSED：fail-closed 时内存镜像不得被消费"
    );
    // 列表同样不得回落内存镜像（契约 §7：持久化不可用必须 fail-closed，不得静默回退）。
    let listed = t10_entries(&plane.list_pending_asks(Some("session-A")));
    assert!(
        listed.is_empty(),
        "SEC-FAILCLOSED：持久化层无记录时 ask_list_pending 不得回落内存镜像，实际返回：{listed:?}"
    );
}

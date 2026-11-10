//! ADR 0113 D4 对抗复核：**真实写工具 → pre-image → 撤回复原** 的接线验收。
//!
//! 为什么单独一个文件：`tui_rewind_acceptance` 的 R3 是手工调用 `record_pre_image`
//! 造假快照，证明不了「写工具落盘前真的抓了」以及「抓取键（session_id, turn_id）与
//! 撤回侧用的是同一份事实」。本测试走 `ToolRouter` 的真实 `workspace_write_file`
//! 执行路径（不经 governed 审批层，只把快照根重定向到进程隔离目录），断言：
//!
//! - **C1** 落盘前的旧内容被抓住，且同轮同路径只抓第一次（幂等）；
//! - **C2** 撤回按「被撤销轮次里最早 pre-image 取胜」还原，区间内新建文件被删除；
//! - **C3** 缺 turn 事实 ⇒ 不抓取也不猜归属，写入照常；
//! - **C4** 抓取失败（快照根不可写）⇒ fail-open，写入结果不受影响。
//!
//! 纪律：AAA、真实断言、无 `#[ignore]`、不吞异常；`SNAPSHOT_ROOT_ENV` 是进程级
//! 全局量，故本文件内所有用例串行执行（`ENV_GUARD`）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::json;

use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::session::{RewindFileStore, SNAPSHOT_ROOT_ENV};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};

static ENV_GUARD: Mutex<()> = Mutex::new(());

fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn sandbox(label: &str) -> PathBuf {
    let unique = format!(
        "pony-agent-rewind-seam-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create sandbox");
    dir
}

fn workspace_of(root: &Path) -> PathBuf {
    let workspace = root.join("ws");
    fs::create_dir_all(&workspace).expect("create workspace");
    workspace
}

fn turn_context(workspace: &Path, session_id: &str, turn_id: Option<&str>) -> ToolExecutionContext {
    ToolExecutionContext {
        workspace_root: Some(workspace.to_path_buf()),
        session_id: Some(session_id.to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some(session_id.to_string()),
            turn_id: turn_id.map(str::to_string),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 走真实写工具（`workspace_write_file`），而不是直接 `fs::write`——抓取点就在它内部。
///
/// 覆盖写入的「读前写观察门」需要 `expectedHash`；本测试聚焦**抓取接缝**而非 CAS，
/// 故用官方豁免开关（`PONY_ALLOW_UNOBSERVED_EDIT=1`）放行，不伪造哈希。
fn write_via_tool(
    router: &ToolRouter,
    path: &str,
    content: &str,
    context: &ToolExecutionContext,
) -> String {
    std::env::set_var("PONY_ALLOW_UNOBSERVED_EDIT", "1");
    let result = router.execute_with_context(
        &ToolCall {
            call_id: Some("call-rewind-seam".to_string()),
            name: "workspace_write_file".to_string(),
            arguments: json!({ "path": path, "content": content }),
            plan: None,
        },
        context,
    );
    assert_eq!(result.status, "ok", "写工具必须以 ok 收口：{result:?}");
    result.output
}

#[test]
fn write_tool_captures_pre_image_and_rewind_restores_earliest_version() {
    let _guard = serial_guard();
    let root = sandbox("capture");
    let snapshot_root = root.join("snap");
    std::env::set_var(SNAPSHOT_ROOT_ENV, &snapshot_root);
    let workspace = workspace_of(&root);
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let target = workspace.join("a.txt");
    fs::write(&target, "v0").expect("seed v0");

    // Act：turn-1 写两次（第二次必须复用首次 pre-image），turn-2 再写一次并新建文件。
    write_via_tool(
        &router,
        "a.txt",
        "v1",
        &turn_context(&workspace, "sess-1", Some("turn-1")),
    );
    write_via_tool(
        &router,
        "a.txt",
        "v2",
        &turn_context(&workspace, "sess-1", Some("turn-1")),
    );
    write_via_tool(
        &router,
        "a.txt",
        "v3",
        &turn_context(&workspace, "sess-1", Some("turn-2")),
    );
    write_via_tool(
        &router,
        "b.txt",
        "brand-new",
        &turn_context(&workspace, "sess-1", Some("turn-2")),
    );
    assert_eq!(fs::read_to_string(&target).expect("read written"), "v3");

    // Assert C1：抓取点真的在落盘前跑过，且同轮同路径幂等。
    let store = RewindFileStore::new(&snapshot_root);
    assert_eq!(
        store.count_pre_images("sess-1", "turn-1"),
        1,
        "同一 (session, turn, path) 只抓第一次"
    );
    assert_eq!(
        store.count_pre_images("sess-1", "turn-2"),
        2,
        "turn-2 抓到 a.txt 与 b.txt 两条"
    );

    // Assert C2：撤消 turn-1 与 turn-2 ⇒ 最早的 pre-image（v0）取胜，新建文件被删除。
    let outcome = store.restore_turns("sess-1", &["turn-1".to_string(), "turn-2".to_string()]);
    assert!(
        !outcome.degraded(),
        "完整快照不得降级：{outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(&target).expect("read restored"),
        "v0",
        "必须还原到最早 pre-image（真实写工具抓到的 v0）"
    );
    assert!(!workspace.join("b.txt").exists(), "区间内新建的文件必须删除");
}

#[test]
fn write_without_turn_facts_writes_but_does_not_capture() {
    let _guard = serial_guard();
    let root = sandbox("no-turn");
    let snapshot_root = root.join("snap");
    std::env::set_var(SNAPSHOT_ROOT_ENV, &snapshot_root);
    let workspace = workspace_of(&root);
    let router = ToolRouter::with_workspace_root(workspace.clone());

    // Act：只有 session，没有 turn（无从归属轮次）。
    write_via_tool(
        &router,
        "c.txt",
        "no-capture",
        &turn_context(&workspace, "sess-2", None),
    );

    // Assert：写入照做，但不得凭空猜归属。
    assert_eq!(
        fs::read_to_string(workspace.join("c.txt")).expect("read written"),
        "no-capture"
    );
    assert!(
        !snapshot_root.join("sess-2").exists(),
        "缺 turn 事实时不得写入任何快照目录"
    );
}

#[test]
fn capture_failure_is_fail_open_for_the_write() {
    let _guard = serial_guard();
    let root = sandbox("fail-open");
    // 快照根指向一个**普通文件** ⇒ create_dir_all 必失败。
    let blocker = root.join("snapshot-root-is-a-file");
    fs::write(&blocker, "not a dir").expect("seed blocker file");
    std::env::set_var(SNAPSHOT_ROOT_ENV, &blocker);
    let workspace = workspace_of(&root);
    let router = ToolRouter::with_workspace_root(workspace.clone());

    // Act：抓取必然失败，但写入不得被阻断（fail-open）。
    write_via_tool(
        &router,
        "d.txt",
        "still-written",
        &turn_context(&workspace, "sess-3", Some("turn-1")),
    );

    // Assert：内容落盘，快照确实没建成（撤回侧将因此无从还原，属已知范围）。
    assert_eq!(
        fs::read_to_string(workspace.join("d.txt")).expect("read written"),
        "still-written",
        "抓取失败绝不能阻断写入"
    );
    assert!(
        RewindFileStore::new(&blocker)
            .count_pre_images("sess-3", "turn-1")
            == 0,
        "快照根不可写时不得留下半成品条目"
    );
}

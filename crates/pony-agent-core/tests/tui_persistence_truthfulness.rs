//! P0 事故验收：**退出提示不得撒谎**（启动预检已由 ADR 0107 §5 P1 退役）。
//!
//! 事故现场（真实，用户复现过）：
//! 1. 第二个 `pony` TUI 抢不到 `sessions.db.lock` → 静默降级只读（只往被 dup2 到
//!    `tui-runtime.log` 的 stderr 打一句）→ 每轮提交失败，会话**根本没落库**；
//! 2. 退出时仍打印 `pony resume <id>`；用户照跑只得到「尚无历史记录」+ 只读报错。
//!
//! 本文件锁定三件事（只测纯函数与事实采集，不做终端仿真）：
//! - `resume_hint_block_with_evidence`：未落库**不**产出续接命令；已落库才产出；
//!   运行期保存失败必须把「数据可能丢失」说给用户；
//! - 启动预检 `startup_permission` 已随 ADR 0107 §5 P1 退役（多进程可写后不再有
//!   「第二实例必须拒绝启动」的语义）；
//! - `session_persistence_evidence`：真库真数据（XDG 沙箱），已建会话判 `Persisted`，
//!   陌生 id 判 `NotPersisted`。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::session::SessionStore;
use tui::persistence_status::session_persistence_evidence;
use tui::PersistenceEvidence;

const LOG_HINT: &str = "/home/dm/.pony-agent/tui-logs/tui-runtime.log";

#[test]
fn unpersisted_session_gets_no_resume_command_and_discloses_data_loss() {
    let block = tui::resume_hint_block_with_evidence(
        "pa_01a1bfd4-nope",
        false,
        PersistenceEvidence::NotPersisted,
        true,
        LOG_HINT,
    );
    assert!(
        !block.contains("pony resume"),
        "未落库的会话**不得**给出续接命令（那是假承诺，用户照做只会开一个空会话），实际 = {block:?}"
    );
    assert!(
        block.contains("未落库"),
        "必须明确说「未落库」，实际 = {block:?}"
    );
    assert!(
        block.contains("数据可能丢失"),
        "运行期保存失败必须把「数据可能丢失」说给用户，实际 = {block:?}"
    );
    assert!(
        block.contains(LOG_HINT),
        "必须告诉用户去哪看原因（运行日志），实际 = {block:?}"
    );
}

#[test]
fn unpersisted_without_save_failure_does_not_cry_data_loss() {
    let block = tui::resume_hint_block_with_evidence(
        "pa_zero_turn",
        false,
        PersistenceEvidence::NotPersisted,
        false,
        LOG_HINT,
    );
    assert!(!block.contains("pony resume"), "实际 = {block:?}");
    assert!(
        !block.contains("数据可能丢失"),
        "没有保存失败时不得乱报数据丢失（会制造假警报），实际 = {block:?}"
    );
}

#[test]
fn persisted_session_gets_copy_pastable_resume_command() {
    let block = tui::resume_hint_block_with_evidence(
        "pa_real",
        false,
        PersistenceEvidence::Persisted,
        false,
        LOG_HINT,
    );
    assert!(
        block.contains("继续该会话请运行：pony resume pa_real"),
        "已落库必须给可直接复制的续接命令，实际 = {block:?}"
    );
    assert!(
        !block.contains("数据可能丢失"),
        "没有保存失败时不得夹带数据丢失警告，实际 = {block:?}"
    );
    assert_eq!(
        block.lines().count(),
        1,
        "已落库且终端还原完整时恰好一行，实际 = {block:?}"
    );
}

#[test]
fn save_failure_is_disclosed_even_when_session_is_persisted() {
    let block = tui::resume_hint_block_with_evidence(
        "pa_real",
        false,
        PersistenceEvidence::Persisted,
        true,
        LOG_HINT,
    );
    assert!(
        block.contains("pony resume pa_real"),
        "已落库仍要给续接命令，实际 = {block:?}"
    );
    assert!(
        block.contains("数据可能丢失"),
        "已落库但运行期保存失败时，必须说明「数据可能丢失」（最近几轮可能没落盘），实际 = {block:?}"
    );
}

#[test]
fn terminal_recovery_line_stays_last_in_both_branches() {
    for evidence in [
        PersistenceEvidence::Persisted,
        PersistenceEvidence::NotPersisted,
    ] {
        let block =
            tui::resume_hint_block_with_evidence("pa_x", true, evidence, true, LOG_HINT);
        let last = block.lines().last().unwrap_or_default();
        assert!(
            last.contains("reset"),
            "终端自救命令必须排在最后（{evidence:?}），实际 = {block:?}"
        );
    }
}

#[test]
fn legacy_hint_block_contract_is_unchanged() {
    assert_eq!(
        tui::resume_hint_block("session-abcd1234", false),
        "继续该会话请运行：pony resume session-abcd1234",
        "老调用点的逐字契约不得被改写"
    );
}

#[test]
fn persistence_evidence_reads_the_real_session_database() {
    // 沙箱：绝不读写用户真实 `~/.local/share/PonyAgent/sessions.db`。
    let sandbox = std::env::temp_dir().join(format!(
        "pony-hint-truth-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&sandbox).expect("create sandbox");
    unsafe {
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    }

    let session_id = "pa_truthfulness_0001";
    {
        let mut store = SessionStore::new();
        store.append_turn(Some(session_id), "用户问题", "助手回答", None, vec![]);
    }
    let db_path = sandbox.join("PonyAgent").join("sessions.db");
    assert!(
        db_path.exists(),
        "前置条件：会话库应落在沙箱里（{}），否则本用例会污染真实库",
        db_path.display()
    );
    let _ = std::fs::remove_file(sandbox.join("PonyAgent").join("sessions.db.lock"));

    assert_eq!(
        session_persistence_evidence(session_id),
        PersistenceEvidence::Persisted,
        "真的落过库的会话必须判 Persisted（否则用户拿不到续接命令）"
    );
    assert_eq!(
        session_persistence_evidence("pa_01a1bfd4-not-in-db"),
        PersistenceEvidence::NotPersisted,
        "库里没有的 id 必须判 NotPersisted（这正是用户现场丢失的那个会话）"
    );

    let _ = std::fs::remove_dir_all(&sandbox);
}

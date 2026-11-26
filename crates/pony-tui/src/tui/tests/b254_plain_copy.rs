//! B254 红相（C5）：面向用户的文案不得出现技术词。
//!
//! 产品要求（用户原话）：端测是面向用户的，不得显示「兜底」等技术语言，必须通俗简明；
//! 如无必要可不显示。契约真值源：`.dev-team/contract-matrix-b254.md`。
//!
//! 判定口径：只锁**用户可见字符串**，不锁内部日志/事件字段（工程事实允许留在日志里）。
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。

use super::*;

/// 用户可见文案禁用词（工程语言不得外泄到端测）。
const FORBIDDEN: &[&str] = &[
    "兜底",
    "SSE",
    "decode",
    "provider",
    "上游",
    "解析失败",
    "聚合预算",
    "同步兜底",
];

fn assert_plain(label: &str, text: &str) {
    assert!(
        !text.trim().is_empty(),
        "B254-C5：{label} 不得为空（要么给朴素短句，要么由调用方不渲染——不能给空串充数）"
    );
    for word in FORBIDDEN {
        assert!(
            !text.contains(word),
            "B254-C5：{label} 含技术词「{word}」，实际 = {text:?}"
        );
    }
}

/// 重试期状态行（原「流式中断，转同步重试…」/「同步兜底第 N 次…」/「已切本地兜底摘要」）必须朴素。
#[test]
fn followup_phase_texts_are_plain() {
    use pony_agent_core::agent::retry::ProviderPhase;

    let phases = [
        ProviderPhase::StreamFallback,
        ProviderPhase::SyncAttempt { attempt: 2 },
        ProviderPhase::LocalFallback,
    ];
    for phase in phases {
        let text = status_dwell::followup_phase_text(&phase);
        assert_plain(&format!("followup_phase_text({phase:?})"), &text);
    }
}

/// 中断轮的一行状态必须朴素（保留「已中断」的事实，不展示失败归因细节）。
#[test]
fn interrupted_status_line_is_plain() {
    let checkpoint = "[中断轮次检查点]\n已执行的工具步骤：\n- Run 探测";
    let line = errors::interrupted_record_status_line(checkpoint, None)
        .expect("B254：检查点标记必须映射为一行用户可见状态");
    assert_plain("interrupted_record_status_line", line);
}

/// 恢复接续不得把工程归因回放给用户（runtime 传的是状态行，不是检查点原文）。
#[test]
fn resume_from_checkpoint_does_not_replay_engineering_detail() {
    let status_line = errors::INTERRUPTED_FAILED_LINE;
    let resumed = errors::resume_from_checkpoint("已完成的历史", Some(status_line));
    assert_plain("resume_from_checkpoint", &resumed);
}

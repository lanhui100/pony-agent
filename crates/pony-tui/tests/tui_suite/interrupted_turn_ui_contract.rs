//! C4/C5 中断轮次展示与脱敏折叠验收（迁移自 `interrupted_turn_resume_acceptance`，验证 `pony_tui` 侧契约）。

use pony_agent_core::agent::session::interrupted_turn::{
    interrupted_turn_assistant_text, persisted_assistant_text, visible_step_lines, ClosedToolStep,
    PendingToolCall,
};
use pony_agent_core::agent::session::HistoryMessageKind;
use pony_tui as tui;

const DELIVERED_TEXT: &str = "我先看了 8080 端口占用，随后执行了 kill。";
const RESULT_ALPHA: &str = "RESULT_ALPHA_8080 被 pid 4242 占用";
const RESULT_BETA: &str = "RESULT_BETA_kill -TERM 4242 已执行，等待 2s 后复查";
const RAW_WITH_DSML: &str = "先说明计划：保留正文。\n<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n<｜｜DSML｜｜ parameter name=\"command\">cargo test --all</｜｜DSML｜｜ parameter>\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>\n然后给结论。";

fn closed(name: &str, result_summary: &str) -> ClosedToolStep {
    ClosedToolStep {
        name: name.to_string(),
        result_summary: result_summary.to_string(),
    }
}

fn closed_steps() -> Vec<ClosedToolStep> {
    vec![closed("Read", RESULT_ALPHA), closed("Bash", RESULT_BETA)]
}

fn pending_calls() -> Vec<PendingToolCall> {
    vec![PendingToolCall {
        name: "Write".to_string(),
    }]
}

#[test]
fn c4_interrupted_turn_renders_one_status_line_and_keeps_steps_in_detail() {
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);

    let cancelled_line = tui::interrupted_record_status_line(&assistant_text, Some(HistoryMessageKind::Cancelled));
    let failed_line = tui::interrupted_record_status_line(&assistant_text, Some(HistoryMessageKind::Interrupted));

    let cancelled_line = cancelled_line.expect("C4：kind=Cancelled 必须渲染一行状态（不得把过程清单当正文直出）");
    assert!(cancelled_line.contains("已停止"));
    assert!(!cancelled_line.contains('\n'));
    assert!(cancelled_line.chars().count() <= 60);

    let failed_line = failed_line.expect("C4：kind=Interrupted 必须渲染一行状态");
    assert!(failed_line.contains("已中断"));
    assert!(!failed_line.contains('\n'));

    let detail_lines = visible_step_lines(&closed_steps, &pending);
    assert_eq!(detail_lines.len(), closed_steps.len() + pending.len());
    for (index, step) in closed_steps.iter().enumerate() {
        assert!(detail_lines[index].contains(&step.name));
        assert!(!detail_lines[index].contains('\n'));
    }

    let dirty_render = format!(
        "{assistant_text}\n［工具调用］［工具调用］［工具调用］\nparameter name=\"command\">cargo test"
    );
    let rendered = tui::sanitize_model_protocol_text(&dirty_render).0;
    assert!(!rendered.contains("parameter name="));
    assert!(!rendered.contains("［工具调用］［工具调用］"));
    assert!(rendered.matches("［工具调用］").count() <= 1);
    for marker in ["|DSML|", "｜DSML｜｜", "invoke name="] {
        assert!(!assistant_text.contains(marker));
    }
}

#[test]
fn c5_folding_never_writes_back_into_persisted_body() {
    let raw = RAW_WITH_DSML;
    let persisted = persisted_assistant_text(raw);
    let display = tui::sanitize_model_protocol_text(raw).0;

    assert!(persisted.contains("先说明计划：保留正文。"));
    assert!(persisted.contains("然后给结论。"));
    for forbidden in ["［工具调用］", "|DSML|", "｜DSML｜｜", "invoke name=", "parameter name=", "cargo test --all"] {
        assert!(!persisted.contains(forbidden));
    }

    let protocol_only = persisted_assistant_text("<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>");
    assert!(protocol_only.trim().is_empty());
    assert_eq!(persisted_assistant_text(&persisted), persisted);

    for marker in ["|DSML|", "｜DSML｜｜", "invoke name=", "parameter name=", "cargo test --all"] {
        assert!(!display.contains(marker));
    }
    assert!(display.matches("［工具调用］").count() <= 1);

    let legacy_dirty = "［工具调用］\nparameter name=\"command\">cargo test\n正文残留";
    let legacy_cleaned = persisted_assistant_text(legacy_dirty);
    assert!(legacy_cleaned.matches("［工具调用］").count() <= 1);
    assert!(!legacy_cleaned.contains("parameter name="));
}

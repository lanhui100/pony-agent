//! 自锁回归（ADR 0092）：`is_host_continuation_prompt` 必须认得 goal 续轮的真实文案
//! `<goal_round round="N" max="M">继续推进目标：…</goal_round>`。
//!
//! 单列文件的原因：`session/tests.rs` 与 `control_plane/mod.rs` 都已触发代码规模门禁棘轮
//! （只许降不许升），新增回归不得再往里堆行数。

use super::store::is_host_continuation_prompt;

#[test]
fn host_continuation_prompt_recognizes_goal_round_envelope() {
    // 正例：宿主真正下发的那一句（真值源是 goal::build_goal_round_message）。
    assert!(is_host_continuation_prompt(
        &crate::agent::goal::build_goal_round_message("把自锁循环修好", 2, 10)
    ));
    // 向后兼容：旧的 display_message 形态仍然认。
    assert!(is_host_continuation_prompt(
        "[goal round 1] 推进目标：把会话标题修好"
    ));

    // 反例：形似但结构不符的文本一律按「用户真实输入」处理（fail-open 到人工语义）。
    assert!(!is_host_continuation_prompt(
        "<goal_round round=\"x\" max=\"10\">继续推进目标：非法轮次</goal_round>"
    ));
    assert!(!is_host_continuation_prompt(
        "<goal_round round=\"1\" max=\"10\">随便说说</goal_round>"
    ));
    assert!(!is_host_continuation_prompt(
        "<goal_round round=\"1\" max=\"10\">继续推进目标：缺少闭合标签"
    ));
    assert!(!is_host_continuation_prompt("[goal round x] 推进目标：非法轮次"));
    assert!(!is_host_continuation_prompt("[goal round 1] 随便说说"));
    assert!(!is_host_continuation_prompt("继续上一轮未完成的步骤"));
}

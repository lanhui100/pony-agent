//! F-12 冻结验收（B232，v2）：审批续轮必须带回「被挂起回合的原用户指令」。
//!
//! 契约真源：`.dev-team/contract-matrix-approval-resume-context.md` §2 C6/C7/C8。
//! 现场反例：会话 `pa_01a1e905` 的 `turn-45f905c5`（第二轮指令）在工作区外 `Read` 触发审批挂起后，
//! 该轮从不 commit；批准后的续轮请求里**没有任何**第二轮指令，模型退回上一轮审核任务。
//!
//! v2 修订（独立审查 3 条 BLOCKING 后）：
//! - 捕获来源改为「composer 提交时登记的 `pending_user_turn`」，不再从 UI 气泡反查
//!   （goal 轮 / Ask 续轮 / 审批续轮都不产生用户气泡，反查会张冠李戴命中旧轮指令）。
//! - 续轮 `display_message = None`：运行时落库 user 行取 `display_message`
//!   （`runtime/turn_prep.rs:125-131` + `runtime/turn_persist.rs:203`），置 `None` 才会把
//!   组合正文落进历史，否则 F-12 只修一轮。
//! - 新增生产接缝 `build_approval_resume_turn_input`：事件循环与单测共用同一组装口径（防假绿）。
//!
//! 纪律：真实断言（不得无断言/不得 `assert!(true)`）、不吞异常；全部用例只依赖进程内状态，
//! 不读写用户 `sessions.db`、不发起网络调用。

use super::*;

/// 批准续轮的宿主固定文案（真值源在会话库常量，这里经 TUI 常量转发，避免文案漂移）。
const APPROVED_PROMPT: &str = APPROVAL_APPROVED_CONTINUATION_PROMPT;

/// 现场原话：第二轮用户指令（截自 `pa_01a1e905` 的 `turn-45f905c5` user/message 事件）。
const SITE_INSTRUCTION: &str = "本轮重点处理ACP相关事项包括以入账事项，dev team实施p0-p2以及入账的B164的修复。";

fn context(turn_id: &str, user_message: &str) -> SuspendedTurnContext {
    SuspendedTurnContext {
        turn_id: turn_id.to_string(),
        user_message: user_message.to_string(),
    }
}

/// 模拟 composer 提交：登记 pending_user_turn + streaming_turn_id + 用户气泡（与生产同序）。
fn state_after_user_submit(turn_id: &str, user_message: &str) -> TuiState {
    let mut state = TuiState::default();
    state.add_user_message(user_message);
    state.streaming_turn_id = Some(turn_id.to_string());
    state.pending_user_turn = Some(context(turn_id, user_message));
    state
}

// T1（INV-4）：无捕获 ⇒ 组合结果与宿主文案逐字相等（旧行为不得改变）。
#[test]
fn compose_without_context_returns_prompt_verbatim() {
    let composed = compose_approval_resume_message(None, APPROVED_PROMPT);
    assert_eq!(
        composed, APPROVED_PROMPT,
        "无挂起上下文时必须原样返回宿主文案（旧行为逐字不变）"
    );
}

// T2（INV-1）：有捕获 ⇒ 原指令在前、宿主文案在后，两个段标齐全且原指令只出现一次。
#[test]
fn compose_with_context_places_instruction_before_prompt() {
    let captured = context("turn-45f905c5", SITE_INSTRUCTION);
    let composed = compose_approval_resume_message(Some(&captured), APPROVED_PROMPT);

    assert!(
        composed.contains(SITE_INSTRUCTION),
        "组合正文必须包含被挂起回合的原用户指令，实际 = {composed:?}"
    );
    assert!(
        composed.contains(APPROVED_PROMPT),
        "组合正文必须保留宿主续轮文案，实际 = {composed:?}"
    );
    assert!(
        composed.contains("【挂起前用户指令（宿主补录）】"),
        "缺少原指令段标，实际 = {composed:?}"
    );
    assert!(
        composed.contains("【宿主续轮】"),
        "缺少宿主续轮段标，实际 = {composed:?}"
    );

    let instruction_at = composed
        .find(SITE_INSTRUCTION)
        .expect("原指令必须出现在组合正文中");
    let prompt_at = composed
        .find(APPROVED_PROMPT)
        .expect("宿主文案必须出现在组合正文中");
    assert!(
        instruction_at < prompt_at,
        "原指令必须排在宿主续轮文案之前（先给意图，再给续轮要求），实际 = {composed:?}"
    );
    assert_eq!(
        composed.matches(SITE_INSTRUCTION).count(),
        1,
        "原指令不得重复注入（重复会让模型误以为用户说了两遍），实际 = {composed:?}"
    );
    // C7 版式逐字：段标、一个空行、宿主文案，不得多/少换行（防止实现漂移成"近似格式"）。
    assert_eq!(
        composed,
        format!("【挂起前用户指令（宿主补录）】\n{SITE_INSTRUCTION}\n\n【宿主续轮】\n{APPROVED_PROMPT}"),
        "组合正文必须与契约 C7 的版式逐字一致"
    );
}

// T3（INV-4 边界）：指令全空白 ⇒ 回落为宿主文案，不得产出只有段标的空壳。
#[test]
fn compose_with_blank_context_falls_back_to_prompt() {
    for blank in ["", "   ", "\n\t "] {
        let captured = context("turn-blank", blank);
        let composed = compose_approval_resume_message(Some(&captured), APPROVED_PROMPT);
        assert_eq!(
            composed, APPROVED_PROMPT,
            "空白指令（{blank:?}）必须回落为宿主文案，不得注入空段"
        );
    }
}

// T4（C6 v2）：捕获只认「提交登记 + 轮次身份一致」，不得从 UI 气泡反查。
#[test]
fn capture_requires_matching_pending_submit() {
    // ① 无提交登记（goal 轮 / Ask 续轮 / 审批续轮等不产生 composer 提交的轮次）：
    //    即便消息区里有旧的 User 气泡，也不得捕获（否则会把旧轮指令贴成「挂起前指令」）。
    let mut state = TuiState::default();
    state.add_user_message(SITE_INSTRUCTION);
    state.streaming_turn_id = Some("turn-goal-1".to_string());
    state.capture_suspended_turn_context();
    assert!(
        state.suspended_turn_context.is_none(),
        "没有 pending_user_turn 时不得从 UI 气泡反查捕获（goal/续轮会把旧指令张冠李戴）"
    );

    // ② 登记的轮次与当前流式轮不一致（迟到旧 sink 事件 / 轮次已切换）：不得捕获。
    let mut state = state_after_user_submit("turn-old", SITE_INSTRUCTION);
    state.streaming_turn_id = Some("turn-new".to_string());
    state.capture_suspended_turn_context();
    assert!(
        state.suspended_turn_context.is_none(),
        "pending_user_turn 与 streaming_turn_id 不一致时不得捕获（迟到事件防串轮）"
    );
    assert!(
        state.pending_user_turn.is_none(),
        "不匹配的登记必须被丢弃（否则迟到事件后可能被后续无关挂起误认领）"
    );

    // ③ 两者一致：捕获 turn_id 与提交正文，且 pending 被消费。
    let mut state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    let captured = state
        .suspended_turn_context
        .clone()
        .expect("提交轮与流式轮一致时必须捕获挂起上下文");
    assert_eq!(captured.turn_id, "turn-suspended-1");
    assert_eq!(captured.user_message, SITE_INSTRUCTION);
    assert!(
        state.pending_user_turn.is_none(),
        "capture 必须一次性消费 pending_user_turn（否则同一提交会被反复捕获）"
    );
}

// T5（INV-3/INV-1）：take 一次性消费 —— 首次带回原指令，第二次只剩宿主文案。
#[test]
fn take_injects_instruction_once_and_clears_context() {
    let mut state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();

    let first = state.take_approval_resume_message(APPROVED_PROMPT);
    assert!(
        first.contains(SITE_INSTRUCTION),
        "首次续轮必须带回原用户指令，实际 = {first:?}"
    );
    assert!(
        first.contains(APPROVED_PROMPT),
        "首次续轮必须保留宿主文案，实际 = {first:?}"
    );
    assert!(
        state.suspended_turn_context.is_none(),
        "take 必须一次性消费并清空捕获（否则下一轮会重复注入）"
    );

    let second = state.take_approval_resume_message(APPROVED_PROMPT);
    assert_eq!(
        second, APPROVED_PROMPT,
        "捕获已被消费 ⇒ 第二次续轮必须回到旧行为（仅宿主文案）"
    );
}

// T6（C7/INV-1）：拒绝路径同样带回原指令，且与批准路径的组合正文不同。
#[test]
fn deny_resume_also_carries_original_instruction() {
    let mut approved_state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    approved_state.capture_suspended_turn_context();
    let approved = approved_state.take_approval_resume_message(APPROVED_PROMPT);

    let mut denied_state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    denied_state.capture_suspended_turn_context();
    let denied = denied_state.take_approval_resume_message(APPROVAL_DENIED_CONTINUATION_PROMPT);

    assert!(
        denied.contains(SITE_INSTRUCTION),
        "拒绝续轮同样必须带回原用户指令，实际 = {denied:?}"
    );
    assert!(
        denied.contains(APPROVAL_DENIED_CONTINUATION_PROMPT),
        "拒绝续轮必须保留拒绝文案，实际 = {denied:?}"
    );
    assert_ne!(
        approved, denied,
        "批准与拒绝的续轮正文必须不同（否则模型无法区分放行与拒绝）"
    );
}

// T7（C6）：同一次提交只捕获一次；捕获后再次挂起不会复用旧指令（一次性边界）。
#[test]
fn capture_is_not_reusable_after_first_suspension() {
    let mut state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    assert!(state.suspended_turn_context.is_some(), "前置条件：首次捕获成功");

    // 同一轮再次收到 TurnSuspended（重复信号/迟到事件）：pending 已消费 ⇒ 不得重新捕获。
    state.capture_suspended_turn_context();
    assert!(
        state.pending_user_turn.is_none(),
        "重复挂起信号不得让 pending 复活"
    );
}

// T8（INV-5）：会话作用域复位（/clear、切会话）必须丢弃陈旧捕获，避免跨会话注入。
#[test]
fn session_scope_reset_drops_stale_context() {
    let mut state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    assert!(
        state.suspended_turn_context.is_some(),
        "前置条件：捕获已就绪"
    );

    state.reset_session_scope();

    assert!(
        state.suspended_turn_context.is_none(),
        "会话作用域复位后不得保留上一会话/上一轮的挂起上下文（跨会话注入即错误归因）"
    );
    assert!(
        state.pending_user_turn.is_none(),
        "会话作用域复位后不得保留未消费的提交登记"
    );
}

// T9（B1+C7+B3）：生产接缝必须把组合正文写进 message、把 display_message 置 None（否则历史丢指令），
// 且不得新增用户气泡（INV-2）。
#[test]
fn approval_resume_turn_input_injects_message_and_keeps_no_user_bubble() {
    let mut state = state_after_user_submit("turn-suspended-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    let messages_before = state.messages.len();

    let input = build_approval_resume_turn_input(
        &mut state,
        APPROVED_PROMPT,
        "ponyllm".to_string(),
        "workbuddy/deepseek-v4.1-flash".to_string(),
    );

    assert!(
        input.message.contains(SITE_INSTRUCTION),
        "续轮 message 必须带回原用户指令（模型面正文），实际 = {:?}",
        input.message
    );
    assert!(
        input.message.contains(APPROVED_PROMPT),
        "续轮 message 必须保留宿主文案，实际 = {:?}",
        input.message
    );
    // B1：运行时落库 user 行取 display_message（turn_prep.rs:125-131 / turn_persist.rs:203），
    // 置 Some 会让历史只剩宿主文案；必须是 None，从而回落 user_message=组合正文。
    assert_eq!(
        input.display_message, None,
        "display_message 必须为 None，否则历史里只写宿主文案、原指令丢失（F-12 只修一轮）"
    );
    assert_eq!(
        state.messages.len(),
        messages_before,
        "续轮不得新增用户气泡（INV-2：宿主文案不是用户发言）"
    );
    assert!(
        state.suspended_turn_context.is_none(),
        "组装必须一次性消费捕获"
    );
    assert_eq!(
        input.provider_id.as_deref(),
        Some("ponyllm"),
        "provider 必须原样透传（不得被组装接缝改写）"
    );
    assert_eq!(
        input.model_id.as_deref(),
        Some("workbuddy/deepseek-v4.1-flash"),
        "model 必须原样透传（不得被组装接缝改写）"
    );
}

// T10（B3 反向用例）：无捕获时接缝必须 fail-open，正文与宿主文案逐字一致、display 仍为 None。
#[test]
fn approval_resume_turn_input_fail_open_without_context() {
    let mut state = TuiState::default();
    let input = build_approval_resume_turn_input(
        &mut state,
        APPROVED_PROMPT,
        "ponyllm".to_string(),
        "workbuddy/deepseek-v4.1-flash".to_string(),
    );

    assert_eq!(
        input.message, APPROVED_PROMPT,
        "无捕获时正文必须逐字等于宿主文案（fail-open，旧行为不变）"
    );
    assert_eq!(
        input.display_message, None,
        "无捕获时 display_message 仍为 None（落库与模型面一致）"
    );
}


// T11（C9）：链式挂起 —— 续轮自身再次挂起时，第二次续轮仍能带回原指令（连续两次工作区外审批）。
#[test]
fn chained_suspension_reseeds_pending_and_keeps_instruction() {
    let mut state = state_after_user_submit("turn-chained-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    let first = state.take_approval_resume_message(APPROVED_PROMPT);
    assert!(
        first.contains(SITE_INSTRUCTION),
        "第一次续轮必须带回原指令，实际 = {first:?}"
    );
    assert_eq!(
        state.resume_instruction.as_deref(),
        Some(SITE_INSTRUCTION),
        "take 必须把原指令留在 resume_instruction 里，供链式挂起复用"
    );

    // 续轮开跑：登记自身 turn_id，并假设它再次挂起（例如又要读第二个工作区外文件）。
    let resume_turn = "turn-approval-resume-chained-1";
    state.seed_pending_after_resume(resume_turn);
    state.streaming_turn_id = Some(resume_turn.to_string());
    state.capture_suspended_turn_context();

    let captured = state
        .suspended_turn_context
        .clone()
        .expect("链式挂起（第二次挂起）必须再次捕获到原指令");
    assert_eq!(captured.turn_id, resume_turn);
    assert_eq!(
        captured.user_message, SITE_INSTRUCTION,
        "第二次续轮的指令必须仍是原始用户指令（不得退化成宿主文案）"
    );
}

// T12（C9/INV-5）：会话作用域复位必须同时清掉链式指令，避免跨会话把旧指令带进新续轮。
#[test]
fn session_reset_clears_resume_instruction() {
    let mut state = state_after_user_submit("turn-chained-1", SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    let _ = state.take_approval_resume_message(APPROVED_PROMPT);
    assert!(
        state.resume_instruction.is_some(),
        "前置条件：take 后链式指令已登记"
    );

    state.reset_session_scope();

    assert!(
        state.resume_instruction.is_none(),
        "会话作用域复位后不得保留链式指令（否则新会话的续轮会注入旧指令）"
    );
}

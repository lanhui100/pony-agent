//! F-12 冻结验收：审批续轮必须带回「被挂起回合的原用户指令」。
//!
//! 契约真源：`.dev-team/contract-matrix-approval-resume-context.md` §2 C6/C7/C8。
//! 现场反例：会话 `pa_01a1e905` 的 `turn-45f905c5`（第二轮指令）在工作区外 `Read` 触发审批挂起后，
//! 该轮从不 commit；批准后的续轮请求里**没有任何**第二轮指令，模型退回上一轮审核任务。
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

fn state_with_suspended_turn(user_message: &str) -> TuiState {
    let mut state = TuiState::default();
    state.streaming_turn_id = Some("turn-suspended-1".to_string());
    state.add_user_message(user_message);
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

// T4（C6）：capture 的三态 —— 无 turn id / 有 turn id 无用户消息 / 两者齐备。
#[test]
fn capture_requires_turn_id_and_user_message() {
    // ① 无 streaming_turn_id：即便有用户消息也不得捕获（不能冒充别的回合）。
    let mut state = TuiState::default();
    state.add_user_message(SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    assert!(
        state.suspended_turn_context.is_none(),
        "缺少 streaming_turn_id 时不得捕获挂起上下文"
    );

    // ② 有 turn id 但没有用户消息：不得捕获（系统消息/助手消息都不是用户指令）。
    let mut state = TuiState::default();
    state.streaming_turn_id = Some("turn-no-user".to_string());
    state.messages.push(ChatMessage {
        role: MessageRole::System,
        content: "等待用户输入确认...".to_string(),
        timestamp_ms: 0,
    });
    state.capture_suspended_turn_context();
    assert!(
        state.suspended_turn_context.is_none(),
        "没有非空 User 消息时不得捕获（System 不是用户指令）"
    );

    // ③ 两者齐备：捕获 turn_id 与非空指令正文。
    let mut state = state_with_suspended_turn(SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    let captured = state
        .suspended_turn_context
        .clone()
        .expect("有 turn id 且有用户消息时必须捕获");
    assert_eq!(captured.turn_id, "turn-suspended-1");
    assert_eq!(captured.user_message, SITE_INSTRUCTION);
}

// T5（INV-3/INV-1）：take 一次性消费 —— 首次带回原指令，第二次只剩宿主文案。
#[test]
fn take_injects_instruction_once_and_clears_context() {
    let mut state = state_with_suspended_turn(SITE_INSTRUCTION);
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

// T6（C7/INV-1）：拒绝路径同样带回原指令，且文案与批准不同（模型据此换路而非空转）。
#[test]
fn deny_resume_also_carries_original_instruction() {
    let mut state = state_with_suspended_turn(SITE_INSTRUCTION);
    state.capture_suspended_turn_context();
    let denied = state.take_approval_resume_message(APPROVAL_DENIED_CONTINUATION_PROMPT);

    assert!(
        denied.contains(SITE_INSTRUCTION),
        "拒绝续轮同样必须带回原用户指令，实际 = {denied:?}"
    );
    assert!(
        denied.contains(APPROVAL_DENIED_CONTINUATION_PROMPT),
        "拒绝续轮必须保留拒绝文案，实际 = {denied:?}"
    );
    assert_ne!(
        APPROVED_PROMPT, APPROVAL_DENIED_CONTINUATION_PROMPT,
        "批准与拒绝文案必须不同（否则模型无法区分放行与拒绝）"
    );
}

// T7（C6）：反向取「最后一条非空 User」—— 中间夹 System/ToolResult/空白 User 仍命中真实指令。
#[test]
fn capture_picks_latest_non_empty_user_message() {
    let mut state = TuiState::default();
    state.streaming_turn_id = Some("turn-layered".to_string());
    state.add_user_message("第一轮：审核 ACP 委派"); // 旧轮用户消息
    state.messages.push(ChatMessage {
        role: MessageRole::Assistant,
        content: "审核完成".to_string(),
        timestamp_ms: 0,
    });
    state.add_user_message(SITE_INSTRUCTION); // 本轮用户消息
    state.messages.push(ChatMessage {
        role: MessageRole::ToolResult,
        content: "工具输出：...".to_string(),
        timestamp_ms: 0,
    });
    state.add_user_message("   "); // 空白 User 不得冒充指令

    state.capture_suspended_turn_context();
    let captured = state
        .suspended_turn_context
        .clone()
        .expect("必须捕获到最近一条非空用户指令");
    assert_eq!(
        captured.user_message, SITE_INSTRUCTION,
        "必须命中最近一条非空 User（跳过空白 User 与 System/ToolResult）"
    );
}

// T8（INV-5）：会话作用域复位（/clear、切会话）必须丢弃陈旧捕获，避免跨会话注入。
#[test]
fn session_scope_reset_drops_stale_context() {
    let mut state = state_with_suspended_turn(SITE_INSTRUCTION);
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
}

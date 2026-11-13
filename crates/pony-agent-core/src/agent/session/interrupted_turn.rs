//! 中断/失败轮次的「闭合真值」纯函数出口（DSH 对齐：**补齐优先于截断**）。
//!
//! ## 背景（真实事故 `pa_01a1ce16-cf4e-72d0-a92b-ae9857aac737`）
//!
//! TUI 里一轮任务被中断后继续，该轮**已完成的工作从模型上下文里整体消失**：续跑请求只有
//! 12 条消息 / 16165 tokens，模型对那一轮的全部记忆就是一条 492 字符的
//! `[中断轮次检查点] 已执行的工具步骤：…`。
//!
//! 根因是"在飞工作不是真值"：中断只发取消令牌、不落库；收口时整轮被压成一条检查点文本，
//! 工具往返（`TurnToolActivity.result_text` 明明有结果）没有任何载体进入请求组装。
//!
//! ## 本模块的三条不变量
//!
//! 1. **闭合结构是唯一真值**：已交付正文 + 已闭合步骤（名称 + 结果摘要）+ 未开始步骤
//!    （显式"结果未知/未开始"事实）一起落库；悬空调用一律补 error 结果，**不伪装成功**。
//! 2. **上下文从该结构投影**：请求组装读到的是上面的结构（native 走 transcript，非 native
//!    走承载文本），不是单条检查点句。
//! 3. **折叠不写回**：DSML 协议块只在展示侧折叠；持久化正文整块丢弃该协议文本。
//!
//! 全部函数为纯函数（无 IO、无锁、无全局状态），便于红相测试与 L2-AT 注入。

use serde_json::{json, Value};

/// 已闭合的工具步骤（名称 + 结果摘要），按发生顺序。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClosedToolStep {
    /// 工具名（如 `Run` / `Read` / `team_task_list`）。
    pub name: String,
    /// 结果摘要（有界长度；由调用方裁剪，本模块不再二次截断语义）。
    pub result_summary: String,
}

/// 模型已宣告但**未开始/未完成**的调用。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingToolCall {
    /// 工具名。
    pub name: String,
}

/// 中断轮承载文本的结果段标题（检查点头之后的附加段）。
pub const INTERRUPTED_RESULT_SECTION_HEADER: &str = "已完成的工具结果：";

/// 未开始/结果未知调用的固定事实注记前缀。
pub const INTERRUPTED_PENDING_NOTE_PREFIX: &str = "结果未知：该调用未开始或未返回，不要盲目重试";

/// 未开始调用在承载文本里的段标题（与结果段并列，逐条列出工具名）。
pub const INTERRUPTED_PENDING_SECTION_HEADER: &str = "未完成的工具调用：";

/// 步骤/原因折叠成**单行**：换行与连续空白归一为单个空格（检查点与明细行都是单行载体）。
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 已闭合步骤的展示行（`<name> → <result>`，单行）。
fn closed_step_line(step: &ClosedToolStep) -> String {
    format!(
        "{} → {}",
        one_line(&step.name),
        one_line(&step.result_summary)
    )
}

/// 未开始步骤的展示行：显式携带"结果未知/未开始"事实与可操作指引（不伪装成功）。
fn pending_step_line(call: &PendingToolCall) -> String {
    format!(
        "{} → {}；如需继续，请先用读取类工具确认该调用的实际状态。",
        one_line(&call.name),
        INTERRUPTED_PENDING_NOTE_PREFIX
    )
}

/// 「全空白条目」判据：名称与结果都去空白后为空 ⇒ 该条目不承载任何事实。
///
/// 承载文本（`interrupted_turn_assistant_text`）据此折叠畸形容器，避免凭空白输入
/// 伪造出"已完成 X → Y"的过程清单；**过程明细**（`visible_step_lines`）不走此判据——
/// 明细是逐条可查的调试面，条数必须与输入一一对应（畸形项也保留，换行由 `one_line` 收敛）。
fn closed_step_is_blank(step: &ClosedToolStep) -> bool {
    step.name.trim().is_empty() && step.result_summary.trim().is_empty()
}

fn pending_call_is_blank(call: &PendingToolCall) -> bool {
    call.name.trim().is_empty()
}

/// 中断/失败轮的承载文本：既有检查点头 + 已闭合结果段 + 未开始步骤事实。
///
/// 约定：
/// - 头部必须逐字复用 `interrupted_turn_checkpoint_text(delivered_text, &step_lines)` 的输出
///   （1518 条基线逐字断言依赖该格式）；
/// - 已闭合步骤逐条追加 `- <name> → <result_summary>`；
/// - 未开始调用逐条追加 `- <name> → <INTERRUPTED_PENDING_NOTE_PREFIX>…`；
/// - 三者皆空 ⇒ 返回空串（调用方回落既有占位文案）。
pub fn interrupted_turn_assistant_text(
    delivered_text: &str,
    closed_steps: &[ClosedToolStep],
    pending_calls: &[PendingToolCall],
) -> String {
    interrupted_turn_assistant_text_with_reason(delivered_text, closed_steps, pending_calls, None)
}

/// 同 [`interrupted_turn_assistant_text`]，但失败轮把**失败原因**逐字带进头部。
///
/// 头部改用既有 [`crate::agent::session::failed_turn_checkpoint_text`]（`failure_reason` 为空时
/// 逐字退化为 `interrupted_turn_checkpoint_text` 的结果），因此取消/失败两条收口路径共用
/// **同一段**「结果段 + 未开始段」投影，只是头部的失败事实不同（C7 同构、原因不丢）。
pub fn interrupted_turn_assistant_text_with_reason(
    delivered_text: &str,
    closed_steps: &[ClosedToolStep],
    pending_calls: &[PendingToolCall],
    failure_reason: Option<&str>,
) -> String {
    // 头部步骤行由工具名派生（纯函数调用面：没有 description 载体）。
    let header_step_lines: Vec<String> = closed_steps
        .iter()
        .filter(|step| !closed_step_is_blank(step))
        .map(|step| one_line(&step.name))
        .filter(|name| !name.is_empty())
        .collect();
    interrupted_turn_assistant_text_with_header_steps(
        delivered_text,
        &header_step_lines,
        closed_steps,
        pending_calls,
        failure_reason,
    )
}

/// 生产收口入口：头部步骤行由调用方给定（保留既有 `name + description` 检查点格式），
/// 结果段/未开始段由 `closed_steps` / `pending_calls` 投影。
///
/// 分开两个入参是必要的：头部沿用历史格式（`<name> <description>`，1518 基线逐字断言），
/// 而 native 闭合片段里的工具名必须是**裸工具名**（`tool_calls[].function.name`），
/// 二者不能互相污染。
pub fn interrupted_turn_assistant_text_with_header_steps(
    delivered_text: &str,
    header_step_lines: &[String],
    closed_steps: &[ClosedToolStep],
    pending_calls: &[PendingToolCall],
    failure_reason: Option<&str>,
) -> String {
    // 已交付正文先用持久化侧清洗（RC3/C5）：承载文本同样不得携带 DSML 协议原文，
    // 否则协议块会经「历史 carry → 请求体」二次泄漏给下一轮模型。
    let delivered_text = crate::agent::provider::strip_dsml_protocol_text(delivered_text);
    let delivered_text = delivered_text.trim();
    let closed_steps: Vec<&ClosedToolStep> = closed_steps
        .iter()
        .filter(|step| !closed_step_is_blank(step))
        .collect();
    let pending_calls: Vec<&PendingToolCall> = pending_calls
        .iter()
        .filter(|call| !pending_call_is_blank(call))
        .collect();
    let step_lines: Vec<String> = header_step_lines
        .iter()
        .map(|line| one_line(line))
        .filter(|line| !line.is_empty())
        .collect();
    let header = match failure_reason {
        Some(reason) if !reason.trim().is_empty() => {
            crate::agent::session::failed_turn_checkpoint_text(
                delivered_text,
                &step_lines,
                reason,
            )
        }
        _ => crate::agent::session::interrupted_turn_checkpoint_text(delivered_text, &step_lines),
    };
    let mut text = header.unwrap_or_default();
    if !closed_steps.is_empty() {
        text.push('\n');
        text.push_str(INTERRUPTED_RESULT_SECTION_HEADER);
        for step in &closed_steps {
            text.push_str("\n- ");
            text.push_str(&closed_step_line(step));
        }
    }
    if !pending_calls.is_empty() {
        text.push('\n');
        text.push_str(INTERRUPTED_PENDING_SECTION_HEADER);
        for call in &pending_calls {
            text.push_str("\n- ");
            text.push_str(&pending_step_line(call));
        }
    }
    text.trim().to_string()
}

/// 悬空调用补齐的 error 结果文本（进 native transcript 的 `role: tool` 内容）。
///
/// 用**纯文本**而不是 JSON 包封：`role: tool` 的 `content` 是模型直接阅读的自然语言载体，
/// JSON 包封只会在序列化后产生转义噪声（且下游断言/检索无法逐字比对）。文本必须同时
/// 携带「工具名 + 结果不确定事实 + 可操作指引」，且**不得**出现"成功"字样（不伪装成功）。
pub fn pending_tool_result_note(tool_name: &str) -> String {
    format!(
        "工具 `{}` 的{INTERRUPTED_PENDING_NOTE_PREFIX}；如需继续，请先用读取类工具确认该调用的实际状态，再决定是否重试。",
        one_line(tool_name)
    )
}

/// native 协议的闭合片段：user → (assistant tool_calls) → (tool results) → assistant 中断注记。
///
/// 悬空调用必须在其中以 error 结果闭合（见 `pending_tool_result_note`）；
/// 无任何步骤且无已交付正文 ⇒ 返回 `None`（由文本承载回退）。
pub fn interrupted_turn_transcript_closure(
    user_message: &str,
    delivered_text: &str,
    closed_steps: &[ClosedToolStep],
    pending_calls: &[PendingToolCall],
) -> Option<Vec<Value>> {
    if closed_steps.is_empty() && pending_calls.is_empty() && delivered_text.trim().is_empty() {
        return None;
    }
    let mut messages: Vec<Value> = Vec::with_capacity(2 + closed_steps.len() * 2 + pending_calls.len() * 2);
    if !user_message.trim().is_empty() {
        messages.push(json!({ "role": "user", "content": user_message }));
    }
    for (index, step) in closed_steps.iter().enumerate() {
        let call_id = format!("interrupted_closed_{index}");
        messages.push(tool_call_message(&call_id, &step.name));
        messages.push(json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": step.result_summary,
        }));
    }
    for (index, call) in pending_calls.iter().enumerate() {
        let call_id = format!("interrupted_pending_{index}");
        messages.push(tool_call_message(&call_id, &call.name));
        messages.push(json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": pending_tool_result_note(&call.name),
        }));
    }
    // 末条 assistant 必须带 reasoning：`first_legacy_reasoning_gap` 会把「tool 之后的无
    // reasoning assistant」判成 legacy 缺口并整段清空 transcript。这里写入的不是伪造的
    // 模型思考，而是**中断事实本身**（下一轮据此知道上一轮停在工具往返之后）。
    //
    // 正文同样经持久化侧清洗：已交付正文里的 DSML 协议块不得经 transcript 二次进入上下文。
    let interrupt_note = crate::agent::session::UPSTREAM_INTERRUPTED_NOTE;
    let delivered = persisted_assistant_text(delivered_text);
    let final_text = if delivered.is_empty() {
        interrupt_note.to_string()
    } else {
        delivered
    };
    messages.push(json!({
        "role": "assistant",
        "content": final_text,
        "reasoning_content": interrupt_note,
    }));
    Some(messages)
}

/// 把「已宣告未执行」的调用以**闭合对**追加到既有协议族 transcript 尾部（L3-A #5）。
///
/// 背景：调用方已有真实 hop 片段（真实 `call_id` / `arguments` / 完整结果）时，**必须优先
/// 保留真实片段**；合成片段只补它没有的信息（被取消跳过的调用）。追加后仍以 assistant 收口，
/// 保证 `tool` 消息不是末条（否则下一请求非法）。
///
/// 返回是否发生了追加（`pending_calls` 为空 ⇒ `false`，调用方保持原片段逐字不变）。
pub fn append_pending_closures_to_transcript(
    transcript: &mut Vec<Value>,
    delivered_text: &str,
    pending_calls: &[PendingToolCall],
) -> bool {
    if pending_calls.is_empty() {
        return false;
    }
    for (index, call) in pending_calls
        .iter()
        .filter(|call| !pending_call_is_blank(call))
        .enumerate()
    {
        let call_id = format!("interrupted_pending_{index}");
        transcript.push(tool_call_message(&call_id, &call.name));
        transcript.push(json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": pending_tool_result_note(&call.name),
        }));
    }
    let interrupt_note = crate::agent::session::UPSTREAM_INTERRUPTED_NOTE;
    let delivered = persisted_assistant_text(delivered_text);
    transcript.push(json!({
        "role": "assistant",
        "content": if delivered.is_empty() { interrupt_note.to_string() } else { delivered },
        "reasoning_content": interrupt_note,
    }));
    transcript.len() > 1
}

/// 单条 assistant 工具调用消息（OpenAI 家族形态：`tool_calls[].function.name/arguments`）。
///
/// 必须同时写 `reasoning_content`：`session::store::first_legacy_reasoning_gap` 把
/// 「reasoning 会话里带 tool_calls 却没有 reasoning 的 assistant」判成 legacy 缺口，
/// 命中即 `provider_native_transcript.clear()` **整段清空**（事故 RC1 的放大器，见 O2）。
/// 这里写入的不是伪造的模型思考，而是**中断事实本身**——下一轮据此知道这一跳的
/// 工具调用来自被中断的轮次。
fn tool_call_message(call_id: &str, tool_name: &str) -> Value {
    json!({
        "role": "assistant",
        "content": "",
        "reasoning_content": crate::agent::session::UPSTREAM_INTERRUPTED_NOTE,
        "tool_calls": [{
            "id": call_id,
            "type": "function",
            "function": {
                "name": tool_name.trim(),
                "arguments": "{}",
            }
        }]
    })
}

/// 历史节点的 `turn_id` 归属：**优先本轮**，缺失时才回退 trace 尾。
pub fn history_node_turn_id(
    current_turn_id: Option<&str>,
    trace_last_turn_id: Option<&str>,
) -> Option<String> {
    current_turn_id
        .filter(|turn_id| !turn_id.trim().is_empty())
        .or_else(|| trace_last_turn_id.filter(|turn_id| !turn_id.trim().is_empty()))
        .map(str::to_string)
}

/// 持久化侧 assistant 正文：整块丢弃 DSML 协议文本，**不写占位符**，保留其余正文。
pub fn persisted_assistant_text(raw_text: &str) -> String {
    crate::agent::provider::strip_dsml_protocol_text(raw_text)
        .trim()
        .to_string()
}

// ── 以下为 T0 增量补签（Lead 已冻结的 5 个函数之上，仅补 C1/C5/C7 的判定接缝）──

/// 中断原因（C7 判定面）：取消/失败/上游收口必须走**同一**闭合路径，仅事实措辞不同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptedTurnCause {
    /// 用户主动终止（Ctrl+C / Esc 双击）。
    UserCancelled,
    /// 本轮失败（错误原因逐字保留，禁止丢弃）。
    TurnFailed { reason: String },
    /// 上游中断/进程收口。
    UpstreamAborted { reason: String },
}

/// 中断原因 → 一行事实注记（失败原因必须原样带出，取消轮不得伪装成失败）。
pub fn interrupted_cause_note(cause: &InterruptedTurnCause) -> String {
    match cause {
        // 用户终止的整句注记复用单一真源（`session::USER_CANCELLED_NOTE`），
        // 避免「同一事实两处文案」随文案微调漂移。
        InterruptedTurnCause::UserCancelled => {
            crate::agent::session::USER_CANCELLED_NOTE.to_string()
        }
        InterruptedTurnCause::TurnFailed { reason } => format!(
            "（本轮失败，未产出最终回复；失败原因：{}。上面的工具往返是已执行的真实步骤。）",
            one_line(reason)
        ),
        InterruptedTurnCause::UpstreamAborted { reason } => format!(
            "（本轮上游中断，未产出最终回复；原因：{}。上面的工具往返是已执行的真实步骤。）",
            one_line(reason)
        ),
    }
}

/// 非 native 族的可见步骤投影（C1 的文本腿）：闭合步骤 + 未开始步骤，逐行有界。
pub fn visible_step_lines(
    closed_steps: &[ClosedToolStep],
    pending_calls: &[PendingToolCall],
) -> Vec<String> {
    let mut lines = Vec::with_capacity(closed_steps.len() + pending_calls.len());
    for step in closed_steps {
        lines.push(format!("- {}", closed_step_line(step)));
    }
    for call in pending_calls {
        lines.push(format!("- {}", pending_step_line(call)));
    }
    lines
}

/// 展示侧折叠（C5 的展示腿）：DSML 协议块不进入可见文本，可与
/// [`persisted_assistant_text`] 的"整块丢弃且不留占位符"对照断言。
///
/// 与 TUI 的 `sanitize_model_protocol_text` 同源折叠，但**不注入占位符**：本函数输出的是
/// 供"展示/下游文本"消费的纯正文投影，占位符只属于 TUI 交互层（连排已由交互层收敛）。
pub fn display_assistant_text(raw_text: &str) -> String {
    crate::agent::provider::strip_dsml_protocol_text(raw_text)
}

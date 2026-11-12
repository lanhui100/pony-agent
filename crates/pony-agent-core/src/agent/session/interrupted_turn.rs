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

// T0 空桩期：签名参数一律未使用（实现期删除本行）。
#![allow(unused_variables)]

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
    unimplemented!("interrupted_turn_assistant_text")
}

/// 悬空调用补齐的 error 结果文本（进 native transcript 的 `role: tool` 内容）。
pub fn pending_tool_result_note(tool_name: &str) -> String {
    unimplemented!("pending_tool_result_note")
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
) -> Option<Vec<serde_json::Value>> {
    unimplemented!("interrupted_turn_transcript_closure")
}

/// 历史节点的 `turn_id` 归属：**优先本轮**，缺失时才回退 trace 尾。
pub fn history_node_turn_id(
    current_turn_id: Option<&str>,
    trace_last_turn_id: Option<&str>,
) -> Option<String> {
    unimplemented!("history_node_turn_id")
}

/// 持久化侧 assistant 正文：整块丢弃 DSML 协议文本，**不写占位符**，保留其余正文。
pub fn persisted_assistant_text(raw_text: &str) -> String {
    unimplemented!("persisted_assistant_text")
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
    unimplemented!("interrupted_cause_note")
}

/// 非 native 族的可见步骤投影（C1 的文本腿）：闭合步骤 + 未开始步骤，逐行有界。
pub fn visible_step_lines(
    closed_steps: &[ClosedToolStep],
    pending_calls: &[PendingToolCall],
) -> Vec<String> {
    unimplemented!("visible_step_lines")
}

/// 展示侧折叠（C5 的展示腿）：DSML 协议块不进入可见文本，可与
/// [`persisted_assistant_text`] 的"整块丢弃且不留占位符"对照断言。
pub fn display_assistant_text(raw_text: &str) -> String {
    unimplemented!("display_assistant_text")
}

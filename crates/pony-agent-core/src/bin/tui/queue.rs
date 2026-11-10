//! 用户消息队列与派发判据（原 `tui.rs` 内联实现，机械外移）。
//! 队列/派发/重灌的所有判定真值源；输入侧与事件循环共用同一组纯函数。

use super::*;

/// C6：入队原语。返回 `false` 表示**拒绝入队**（调用方必须给出提示，不得静默丢弃）。
///
/// 拒绝口径：空串/纯空白；斜杠命令串（走 C3/C4 分发，不当作聊天消息排队）；
/// 已达 `MAX_QUEUED_MESSAGES` 上限（绝不扩容、绝不覆盖既有条目）。
pub fn push_queued_message(queue: &mut Vec<String>, text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    // 命令串不入队：整串命中命令表（`/clear 这个需求怎么拆` 这类带正文的串不算命令，
    // 与主循环的分发口径 `parse_command_input` 同源，静态下它本来也是当普通消息发出去的）。
    if parse_command_input(trimmed).is_some() {
        return false;
    }
    // C18①：命令表之外的**历史别名**同样是命令语义，不得当成聊天消息排队
    //（`/exit` 走退出分支、`/effort` 走 /thinking 面板分支）。
    if input_is_alias(trimmed, "/exit") || input_is_alias(trimmed, "/effort") {
        return false;
    }
    if queue.len() >= MAX_QUEUED_MESSAGES {
        return false;
    }
    queue.push(trimmed.to_string());
    true
}

/// 取走队首一条待派发消息（FIFO）。空队列立即 `None`，不阻塞。
/// 单飞不变量：同一时刻最多 1 条排队消息在飞（NFR `queue_drain_single_flight`）。
pub fn drain_next_queued(state: &mut TuiState) -> Option<QueuedUserMessage> {
    if state.queued_user_messages.is_empty() {
        return None;
    }
    Some(state.queued_user_messages.remove(0))
}

/// C15：**唯一派发取件点** —— 闸门判定 + 取件收成一个生产符号
/// （主循环只能经它取件；A7 直接断言本符号，而不是测试内自建的复刻品）。
///
/// 三道前哨，任一不过即 `None`（且**不**动队列）：
/// 1. 输入框非空 ⇒ `None`：绝不覆盖用户草稿，也不覆盖「已回灌但尚未被消费」的上一条派发正文；
/// 2. [`should_drain_queued`] 五判据不过 ⇒ `None`（单飞 / 空闲 / 非流式 / 无在飞轮次线程）；
/// 3. 否则 `drain_next_queued`（FIFO、一次 1 条、空队列 `None`）。
///
/// 本符号**只取件、不写回**：写回输入框由调用点负责，且必须经
/// [`composer_buffer_for_dispatch`] 净化（C14 修订裁定：净化口径独立成纯函数，便于机械断言）。
pub fn take_next_queued_for_dispatch(
    state: &mut TuiState,
    is_streaming: bool,
    ui_idle: bool,
    turn_in_flight: bool,
    dispatch_pending: bool,
) -> Option<QueuedUserMessage> {
    if !state.input_buffer.trim().is_empty() {
        return None;
    }
    if !should_drain_queued(
        is_streaming,
        ui_idle,
        state.queued_count(),
        turn_in_flight,
        dispatch_pending,
    ) {
        return None;
    }
    drain_next_queued(state)
}

/// C14：派发正文写回输入框前的**净化缓冲区**（纯函数，契约裁定 2026-11-06）。
///
/// 返回「正文 + 一个尾部空格」，**不改正文内容**（不 trim、不转义）——提交路径自己 `trim()`，
/// 所以发给模型的正文与入队快照逐字一致。尾部空格消除两条「正文永远发不出去」的自救路径：
/// ① 正文以 `\` 结尾 ⇒ 主循环的「行尾反斜杠 + Enter = 换行」分支会吞掉提交；
/// ② 正文是 `/hel` 这类命令前缀 ⇒ `matching_commands()` 补全会把 Enter 劫持成另一条命令。
pub fn composer_buffer_for_dispatch(message: &str) -> String {
    format!("{message} ")
}

/// C20：流式期 Enter 优先级（纯函数）。
///
/// 顺序即优先级：
/// 1. `input` 以 `\` 结尾 ⇒ [`StreamingEnterPre::Newline`]（走原「反斜杠 + Enter = 换行」分支）；
/// 2. 补全可见 + `/` 开头 + 不是完整命令 ⇒ [`StreamingEnterPre::Complete`]
///    （否则 `/he`+Enter 会把命令前缀当聊天消息排进队列）；
/// 3. 其余 ⇒ [`StreamingEnterPre::Submit`]（交给 C2/C3/C4 裁决）。
pub fn streaming_enter_pre_action(input: &str, completion_visible: bool) -> StreamingEnterPre {
    if input.ends_with('\\') {
        return StreamingEnterPre::Newline;
    }
    if completion_visible
        && input.trim_start().starts_with('/')
        && parse_command_input(input).is_none()
    {
        return StreamingEnterPre::Complete;
    }
    StreamingEnterPre::Submit
}

/// C22：反馈文案与通道的唯一真值（主循环不再手写文案）。
///
/// 通道口径：C4/C6 是「用户操作被拒绝」，必须走消息区（`SystemMessage`）；
/// C2 是「已受理」的短回执，走状态行（`StatusText`）。
pub fn queue_feedback(kind: QueueFeedbackKind) -> (QueueFeedbackChannel, String) {
    match kind {
        QueueFeedbackKind::SessionMutating => (
            QueueFeedbackChannel::SystemMessage,
            "本轮结束后可用 · 流式生成中不切换会话".to_string(),
        ),
        QueueFeedbackKind::QueueFull => (
            QueueFeedbackChannel::SystemMessage,
            format!(
                "排队已满（上限 {MAX_QUEUED_MESSAGES} 条）· 本条未排队，请本轮结束后再发送"
            ),
        ),
        QueueFeedbackKind::Enqueued => (
            QueueFeedbackChannel::StatusText,
            "已排队 · 本轮结束后自动发送".to_string(),
        ),
    }
}

/// C21：合成自动提交是否可以安全回灌（纯函数）。
///
/// 派发闸门用 `UiEvent::QueuedAutoSubmit` 而不是直接注入 `InputKey(Enter)`：
/// 收到该事件时若界面已进入模态/待作答态，回灌的 Enter 会被面板吃掉
/// （甚至误触发「同意审批」），故必须重新校验一次。
pub fn should_reinject_auto_submit(
    mode_is_modal: bool,
    has_pending_request: bool,
    has_ask: bool,
    has_permission: bool,
) -> bool {
    !mode_is_modal && !has_pending_request && !has_ask && !has_permission
}

/// C23：与 mode / `runtime_mode` **无关**的排队指示行（唯一渲染点真值）。
///
/// 为什么必须与运行态无关：`status_word` 在 `RuntimeMode::ExecutingTool(_)` 期整行不渲染，
/// 若指示器只挂在它上面，「流式 + 工具执行期」就一个渲染点都没有（L3-A 新 BLOCKING）。
/// 语义：`queued > 0` 或 `notice` 非空 ⇒ **必** `Some`（两个 bool 不参与判定）；
/// notice 与计数**同时存在时拼接**（notice 在前、计数在后，两者都是用户需要的信息）；
/// 两者都无 ⇒ `None`。
pub fn queue_indicator_line(
    queued: usize,
    notice: Option<&str>,
    is_streaming: bool,
    executing_tool: bool,
) -> Option<String> {
    // 两个运行态布尔只用于「语义自证」：本函数对它们**不做任何分支**，
    // 这正是契约要求的「任何运行态都必须有渲染点」。
    let _runtime_state = (is_streaming, executing_tool);
    let notice = notice.map(str::trim).filter(|text| !text.is_empty());
    let count_label = queued_status_label(queued);
    match (notice, count_label) {
        (Some(text), Some(label)) => Some(format!("{text} · {label}")),
        (Some(text), None) => Some(text.to_string()),
        (None, Some(label)) => Some(format!("{label} · 本轮结束后自动发送")),
        (None, None) => None,
    }
}

/// C24：单飞闩是否仍「在飞」（活性 = `Some(deadline) && now < deadline`）。
pub fn queue_dispatch_latch_active(deadline: Option<std::time::Instant>) -> bool {
    matches!(deadline, Some(deadline) if std::time::Instant::now() < deadline)
}

/// C24：把一条已取出/已写回的排队消息**回队到队首**（FIFO 复原），杜绝静默丢正文。
///
/// 使用场景：① `QueuedAutoSubmit` 校验不通过（界面已进入模态/待作答态）；
/// ② 闩窗口内用户按静态 Esc / Ctrl+U 显式撤销本次派发。
/// 长度不变量：调用点总是「刚取走 1 条」或「输入框里那 1 条」，故回队后必然 ≤ `MAX_QUEUED_MESSAGES`。
pub fn requeue_front(state: &mut TuiState, message: QueuedUserMessage) {
    if message.trim().is_empty() {
        return;
    }
    state.queued_user_messages.insert(0, message);
}

/// C26：派发快照与当前输入框是否**已分歧**（纯函数，A25 真值矩阵）。
///
/// 语义：`dispatched` 为 `None`（没有在飞派发）⇒ `false`；
/// 否则 `dispatched.trim() != composer.trim()` ⇒ `true`。
/// 用途：`QueuedAutoSubmit` 消费点据此判断「用户是否在窗口内改过输入框」——
/// 分歧时回队**原快照**（而不是此刻的输入框字节），否则 Ctrl+W/Backspace/Delete
/// 会把已派发正文擦掉且不回队 ⇒ 静默丢正文。
pub fn queued_dispatch_diverged(dispatched: Option<&str>, composer: &str) -> bool {
    match dispatched {
        None => false,
        Some(payload) => payload.trim() != composer.trim(),
    }
}

/// C27：**孤儿载荷收割**（纯函数，A27 真值矩阵）。
///
/// 场景：时间窗闩 2s 过期后闸门把「仍有 QAS 在飞」当成空闲，若此刻再派发就会覆盖单槽
/// `queue_dispatch_payload` ⇒ 前一条在 `remove(0)` 后既未提交也未回队（静默丢正文）。
/// 语义：闩**已过期**（`!latch_active`）但 payload 仍非空 ⇒ 它是孤儿，返回它（trim 后非空才返回；
/// 纯空白 ⇒ `None`）；闩仍活跃（QAS 真在飞）或没有 payload ⇒ `None`。
pub fn orphan_dispatch_payload(payload: Option<&str>, latch_active: bool) -> Option<String> {
    if latch_active {
        return None;
    }
    let payload = payload?.trim();
    if payload.is_empty() {
        return None;
    }
    Some(payload.to_string())
}

/// C25：派发日志的结构化字段（纯函数，A23 直接断言字段名与顺序）。
pub fn queue_dispatch_fields(
    before: usize,
    after: usize,
) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("queue_depth_before", serde_json::json!(before)),
        ("queue_depth_after", serde_json::json!(after)),
    ]
}

/// 面板类斜杠命令（只读快照类；流式期允许打开，不排队、不拒绝）：
/// `/agents` `/tasks` `/plan` `/models` `/thinking` `/settings` `/provider` `/help` `/update`。
pub fn is_panel_command(command: &str) -> bool {
    const PANEL_COMMANDS: &[&str] = &[
        "/agents", "/teams", "/tasks", "/plan", "/models", "/thinking", "/settings", "/provider",
        "/help", "/update",
    ];
    let trimmed = command.trim();
    PANEL_COMMANDS
        .iter()
        .any(|cmd| cmd.eq_ignore_ascii_case(trimmed))
}

/// 会话变更类斜杠命令（会拆掉在飞轮次；流式期拒绝，不得就地执行也不入队）：
/// `/clear` `/compact` `/resume`。`/quit` `/exit` 不属本类（走原退出分支）。
pub fn is_session_mutating_command(command: &str) -> bool {
    const MUTATING_COMMANDS: &[&str] = &["/clear", "/compact", "/resume", "/rewind"];
    let trimmed = command.trim();
    MUTATING_COMMANDS
        .iter()
        .any(|cmd| cmd.eq_ignore_ascii_case(trimmed))
}

/// C7：排队数量的可见文案（`count == 0` ⇒ `None`，不渲染）。
///
/// 渲染点：`status_word` 把它追加到流式活动行（流式期 `status_notice` 按契约 P-5 让位，
/// 故排队信息必须走这一条**流式期也可见**的通道）。
pub fn queued_status_label(count: usize) -> Option<String> {
    if count == 0 {
        None
    } else {
        Some(format!("排队 {count} 条"))
    }
}

/// C10：终态后是否该派发队首（单飞判据，五谓词全部显式入参以便机械断言）。
///
/// `turn_in_flight` = **轮次线程存活标志**（`TurnLoopInFlightGuard` 维护），不是「有没有
/// armed goal」：goal 自动续轮期间 `is_streaming` 不为真，只看 goal 是否存在会让
/// 「armed 但续轮没在跑」的会话排队永不自动派发（契约 C10 修订，2026-11-06）。
/// 只有「静态 + 界面空闲 + 队列非空 + 无在飞轮次线程 + 无在飞派发」才放行。
pub fn should_drain_queued(is_streaming: bool, ui_idle: bool, queued: usize, turn_in_flight: bool, dispatch_pending: bool) -> bool {
    !is_streaming && ui_idle && queued > 0 && !turn_in_flight && !dispatch_pending
}

/// C17：界面是否真的「空闲」到可以派发排队消息（派发闸门的第 2 判据真值）。
///
/// 取景范围（诚实登记，本仓没有单一 `awaiting_answer` 布尔字段，故按现有状态组合）：
/// 1. `mode ∈ {Normal, PlanMode}`：PlanMode 是非阻塞执行模式开关（第三轮裁定，允许派发）；
///    其余模态面板（Selecting* / Ask / Permission / TaskBoard / AgentRoster / AddingProvider）
///    占用时一律不派发；
/// 2. `pending_request.is_none()`：没有待作答/待审批的宿主请求；
/// 3. `ask_user_state.is_none()`：Ask 面板没有未答完的问题；
/// 4. `permission_state.is_none()`：审批面板没有未决选项。
///
/// 为什么必须排除「挂起期」：`UiEvent::TurnSuspended` 会把 `is_streaming` 置 false，但该 run
/// **并未收口**（等用户回答）。此时派发会让新 turn 与挂起的 run 交叠写同一会话历史。
/// 上述 1–4 覆盖了 TUI 侧全部「等用户输入」的可达状态；若将来引入新的挂起通道，
/// 必须同步扩展本判据（这是本函数唯一的真实约束）。
pub fn is_ui_idle_for_dispatch(state: &TuiState) -> bool {
    // 第三轮裁定：`PlanMode` 是**非阻塞执行模式开关**（不限制工具执行、不占输入框），
    // 不是模态对话框 ⇒ 允许派发；其余非 Normal 模式一律排除。
    matches!(state.mode, InteractionMode::Normal | InteractionMode::PlanMode)
        && state.pending_request.is_none()
        && state.ask_user_state.is_none()
        && state.permission_state.is_none()
}

/// C21：该模式是否属于**模态对话框**（占用输入框、Enter 语义被面板改写）。
///
/// 与 [`is_ui_idle_for_dispatch`] 同源但用途不同：后者判「能否开始派发」，
/// 本函数判「已发出的合成 Enter 会不会被面板吃掉」。`AddingProvider` 虽然不在派发闸门的
/// 允许集合里，这里仍算模态（防御「派发途中弹面板」的窗口）。
pub fn is_modal_mode(mode: &InteractionMode) -> bool {
    matches!(
        mode,
        InteractionMode::SelectingSession
            | InteractionMode::SelectingProvider
            | InteractionMode::SelectingModel
            | InteractionMode::SelectingReasoningEffort
            | InteractionMode::AddingProvider
            | InteractionMode::PermissionApproval
            | InteractionMode::AskUserQuestion
            | InteractionMode::TaskBoard
            | InteractionMode::AgentRoster
            | InteractionMode::Settings
    )
}

//! 键盘/输入判定层（原 `tui.rs` 内联实现，机械外移）。
//! 只做「事件 → 动作」的纯判定，不触碰终端、不持有状态机。

use super::*;

/// Ctrl+Z 是否为「挂起 TUI」快捷键（纯函数，便于在无 TTY 环境断言）。
///
/// 只认「恰好 CONTROL + `Char('z')`」：带 Shift 时字符会变成 `Char('Z')`，
/// 带 Alt 时 modifiers 不等，故重做类组合（Ctrl+Shift+Z）不会被误判为挂起。
/// 非 unix 无 POSIX 作业控制，恒 `false` —— Ctrl+Z 必须保持原样插入输入框。
pub fn is_suspend_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    if !cfg!(unix) {
        return false;
    }
    modifiers == KeyModifiers::CONTROL && code == KeyCode::Char('z')
}

/// Ctrl+O 是否为「切换隐藏 Agent 过程」快捷键（纯函数，便于在无 TTY 环境断言）。
///
/// 只认「恰好 CONTROL + `Char('o')`」：带 Shift 时字符会变成 `Char('O')`、带 Alt 时
/// modifiers 不等，故不会被误判，避免与终端复制类组合键冲突。
///
/// **全局语义**：本判定不依赖 `InteractionMode`。事件循环把它放在**所有模态分支之前**
/// （与 F2/F3 同级），因此设置面板、各选择器、审批/问答等模态打开时同样生效——
/// 这正是它与“面板内才可用”的 `Ctrl+T`/`Ctrl+R` 的关键区别（模型面板另有 `Ctrl+M` 别名，
/// 见 `tui.rs` 面板快捷键分支，仅 kitty 协议终端生效）。与 `/settings` 读写
/// 同一 `hide_process` 字段并共用同一落盘点（契约 C12）。
pub fn is_hide_process_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::CONTROL && code == KeyCode::Char('o')
}

/// Ctrl+P 是否为「轮转切换工具权限级别」快捷键。
pub fn is_permission_mode_shortcut(code: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::CONTROL && code == KeyCode::Char('p')
}

pub fn composer_enter_action(key: &crossterm::event::KeyEvent) -> ComposerEnterAction {
    // Shift+Enter / Alt+Enter 是「换行」的显式语义；其余键（含裸 Enter）落到 Enter 分支的提交语义。
    if key.code == KeyCode::Enter
        && (key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT))
    {
        ComposerEnterAction::Newline
    } else {
        ComposerEnterAction::Submit
    }
}

pub fn composer_control_action(key: &crossterm::event::KeyEvent) -> Option<ComposerControlAction> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char('u') => Some(ComposerControlAction::ClearInput),
        KeyCode::Char('j') => Some(ComposerControlAction::InsertNewline),
        KeyCode::Char('w') => Some(ComposerControlAction::DeleteWordBackward),
        KeyCode::Char('a') => Some(ComposerControlAction::LineHome),
        KeyCode::Char('e') => Some(ComposerControlAction::LineEnd),
        _ => None,
    }
}

/// 壳层历史回填判据：**仅当输入框为空 + 裸 Up** 才命中（shell 风格）。
///
/// 口径：
/// - 输入框非空 ⇒ 一律 `false`：Up 保持原生（桌面=光标移动 / TUI=滚动消息），
///   多行编辑与历史翻页互不干扰；
/// - 带任何修饰键（Shift/Alt/Ctrl）⇒ `false`：Shift+Up 选中等原生行为优先；
/// - 仅「空输入框 + 裸 Up」⇒ `true`：调用方回填最近一条用户消息，**不再**滚动消息区
///   （在 TUI 中即覆盖原有的「Up=上翻消息」行为）。
///
/// 纯函数，便于在无 TTY 环境断言（见 `tui/tests/composer_history_recall_tests.rs`）。
pub fn composer_history_recall(key: &crossterm::event::KeyEvent, composer_is_empty: bool) -> bool {
    composer_is_empty
        && key.code == KeyCode::Up
        && !key
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL)
}

/// 流式期该按键是否应被丢弃：只有**会话控制**类按键要等本轮结束，
/// 输入框编辑类快捷键不等（回归护栏见 `tests/tui_six_fixes_acceptance.rs`）。
///
/// 契约变更（`tui-streaming-input` C1，2026-11-06）：主循环已改由
/// [`streaming_key_disposition`] 裁决，本函数**保留为公共符号**（契约 §2 要求不删），
/// 既是既有测试的回归护栏，也是「编辑类快捷键不受流式影响」这一不变量的可测入口。
#[allow(dead_code)]
pub fn should_skip_while_streaming(key: &crossterm::event::KeyEvent) -> bool {
    composer_control_action(key).is_none()
}

/// C1–C4 纯函数裁决：按键 + 「输入框是否有正文」⇒ 处置。
///
/// 口径（与契约 §1 逐条对齐）：
/// - **Enter**（裸键 / 非 Shift/Alt）＝唯一可能「提交」的键：有正文 ⇒ `QueueSubmit`
///   （流式中排队，绝不并发 spawn 第二个 turn）；无正文 ⇒ `Reject`（不入队空气泡）。
/// - **Shift/Alt+Enter** ＝ 换行（编辑动作）⇒ `Forward`。
/// - 其余按键（可打印字符、Backspace/Delete、方向键、Home/End、Tab 补全、
///   Ctrl+U/J/W/A/E 编辑快捷键、Ctrl+T/Ctrl+R 面板快捷键（模型面板另有 Ctrl+M 别名，
///   仅 kitty 协议终端上报为 `Char('m')`+CONTROL；其余终端降级为 Enter）、F2/F3、Esc）一律 `Forward`
///   —— 流式守卫不再吞键（C1/C3）。Esc / Ctrl+C / 面板模态分支在调用点**先于**本裁决生效。
pub fn streaming_key_disposition(
    key: &crossterm::event::KeyEvent,
    composer_has_text: bool,
) -> StreamingKeyDisposition {
    if key.code != KeyCode::Enter {
        return StreamingKeyDisposition::Forward;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT) {
        // Shift+Enter / Alt+Enter = 换行（编辑语义，不是提交）。
        return StreamingKeyDisposition::Forward;
    }
    if composer_has_text {
        StreamingKeyDisposition::QueueSubmit
    } else {
        StreamingKeyDisposition::Reject
    }
}

/// C19：PlanMode 面板真正要「吞掉」的按键（Esc / Up / Down）。
///
/// 其余按键**不得**被面板分支吞掉：计划面板只是界面提示（不限制工具执行），
/// 打开时输入框仍应完整可用（旧实现 `_ => {}` + `continue` 让整段输入失效）。
pub fn plan_mode_consumes_key(key: &crossterm::event::KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc | KeyCode::Up | KeyCode::Down)
}

/// C25：按键的**简化分类**（绝不把用户输入的正文字符写进日志）。
pub fn key_class_label(key: &crossterm::event::KeyEvent) -> String {
    match key.code {
        KeyCode::Char(_) => "Char".to_string(),
        other => format!("{other:?}"),
    }
}

/// wave-40 R2（契约冻结空桩）：终端支持键盘增强时返回要 push 的标志。
pub fn keyboard_enhancement_flags(supported: bool) -> Option<KeyboardEnhancementFlags> {
    // kitty 协议的 DISAMBIGUATE_ESCAPE_CODES：把 Shift+Enter 与裸 Enter 区分开。
    if supported {
        Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    } else {
        None
    }
}

/// Shift+Enter 换行依赖 kitty 键盘协议的 DISAMBIGUATE 标志。
///
/// 旧实现用 `supports_keyboard_enhancement()` 做阻塞式探测（发 `CSI ? u` 等最多 2s 回执）：
/// 在 tmux、VS Code 内置终端与部分 VTE 上会被误判为 `false`，标志从未 push，于是 Shift+Enter
/// 被终端降级成裸 Enter（＝发送）。新口径：只要不是 `TERM=dumb` 就无条件 push
/// （未知 CSI 会被终端静默忽略；push 失败只降级告警，不改变其他行为）。
pub fn keyboard_enhancement_should_push() -> bool {
    keyboard_enhancement_should_push_for(std::env::var("TERM").ok().as_deref())
}

/// [`keyboard_enhancement_should_push`] 的纯函数内核：只有 `TERM` 缺失或为 `dumb` 才放弃 push
/// （纯函数便于断言，避免测试里改全局环境变量）。
pub fn keyboard_enhancement_should_push_for(term: Option<&str>) -> bool {
    !matches!(
        term.unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "" | "dumb"
    )
}

/// ADR 0075：探测当前进程是否运行在复用器 / pty 代理层下（读真实环境变量）。
pub fn terminal_multiplexer() -> Option<String> {
    terminal_multiplexer_for_env(|key| std::env::var(key).ok())
}

/// [`terminal_multiplexer`] 的纯函数内核：注入式读取环境变量，便于单测不改全局 `env`。
/// 命中第一个**非空白**值即返回该变量名；全空 / 缺失返回 `None`（空白值不算复用器）。
pub fn terminal_multiplexer_for_env<F>(get: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    TERMINAL_MULTIPLEXER_ENV_KEYS
        .iter()
        .find(|key| {
            get(key)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        })
        .map(|key| key.to_string())
}

/// ADR 0075：push 键盘增强标志的**唯一决策点**（按终端拓扑分级）。
///
/// - `TERM` 缺失 / `dumb` ⇒ 不 push；
/// - 复用器 / pty 代理层（`HERDR_*`/`TMUX`/`ZELLIJ`/`STY`/`ABDUCO`，见
///   [`TERMINAL_MULTIPLEXER_ENV_KEYS`]）⇒ **只** `DISAMBIGUATE`：
///   Shift+Enter 照常可用，而「pop 丢失」的后果从「全键盘乱码」降级为「修饰键异常」；
/// - 直连终端 ⇒ `DISAMBIGUATE | REPORT_ALL_KEYS`（保留既有全键位上报能力）。
pub fn keyboard_enhancement_push_flags() -> Option<KeyboardEnhancementFlags> {
    keyboard_enhancement_push_flags_for(
        std::env::var("TERM").ok().as_deref(),
        terminal_multiplexer().as_deref(),
    )
}

/// [`keyboard_enhancement_push_flags`] 的纯函数内核（可注入 TERM 与复用器视图）。
pub fn keyboard_enhancement_push_flags_for(
    term: Option<&str>,
    multiplexer: Option<&str>,
) -> Option<KeyboardEnhancementFlags> {
    if !keyboard_enhancement_should_push_for(term) {
        return None;
    }
    if multiplexer
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return Some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES);
    }
    Some(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
    )
}

/// W3b：子会话内 Ctrl+↑/←/→ 的裁决（纯函数，可单测）。
///
/// 只认 **Ctrl + 方向键**：裸方向键是光标移动、Ctrl+Char 是编辑动作、Alt+Up 等一律
/// `None` —— 调用方据此**不消费**这些键（主会话里 Ctrl+←/→ 仍是原光标行为）。
pub fn sub_session_shortcut(key: &crossterm::event::KeyEvent) -> Option<SubSessionShortcut> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Up => Some(SubSessionShortcut::ReturnToMain),
        KeyCode::Right => Some(SubSessionShortcut::Switch(SubSessionStep::Next)),
        KeyCode::Left => Some(SubSessionShortcut::Switch(SubSessionStep::Prev)),
        _ => None,
    }
}

/// 契约 F-E：输入泵轮询结果的处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPumpAction {
    Continue,
    Stop,
}

/// 契约 F-E：`event::poll` 结果的确定性分流。
///
/// - `Ok(_)`（含 `Ok(false)` = 本轮无事件）：继续泵；
/// - `Err(_)`：输入源已不可用（stdin 关闭 / 非 TTY / EINTR 等），必须退出线程——
///   旧实现 `unwrap_or(false)` 会把它当成"无事件"，于是 50ms 一轮永久热旋，且用户毫无提示。
pub fn classify_input_poll(result: std::io::Result<bool>) -> InputPumpAction {
    match result {
        Ok(_) => InputPumpAction::Continue,
        Err(_) => InputPumpAction::Stop,
    }
}

/// 契约 P-4：Ctrl+C 的三态语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrlCAction {
    InterruptTurn,
    ClearDraft,
    Exit,
}

/// 契约 P-4：Ctrl+C 行为判定（纯函数）。
///
/// 流式中 ⇒ 中断本轮（等价 Esc），**不退出**；否则有草稿 ⇒ 先清草稿（防误触丢整段输入）；
/// 否则才退出。旧实现把「流式中 Ctrl+C」直接映射为 `break`，等于一次误触就杀掉整个 TUI。
pub fn ctrl_c_action(is_streaming: bool, has_draft: bool) -> CtrlCAction {
    if is_streaming {
        CtrlCAction::InterruptTurn
    } else if has_draft {
        CtrlCAction::ClearDraft
    } else {
        CtrlCAction::Exit
    }
}

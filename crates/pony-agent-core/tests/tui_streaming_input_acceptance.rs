//! L2-T 功能契约 · TUI 流式期间输入 / 面板 / 排队 / 派发闸门 / 反馈可见性
//! （A1–A6 + A9 + A11–A27）
//!
//! 真值源（只读，Lead 冻结 + 裁决修订）：`.dev-team/contract-matrix-tui-streaming-input.md`
//!   §1 C1–C27 · §2 权威符号表 · §4 A1–A27。
//!
//! 第二轮修订（2026-11-06，契约扩到 C12–C18）：
//!   A11=C12 反馈可见性（契约级可测面）/ A12=C13 会话切换清队列 / A13=C14 派发写回净化（载体 composer_buffer_for_dispatch）/
//!   A14=C17 挂起期禁止派发 / A15=C18 别名不入队 + 容量语义不变。
//! 第三/四轮修订（2026-11-06）：A11=C12 改锁 queue_feedback 的 kind→channel 映射（旧版对 B1 无检测力）；
//!   A16=C19 计划面板吞并面 / A17=C20 Enter 前置优先级（streaming_enter_pre_action）/
//!   A18=C21 回灌自动提交闸门 / A19=C22 满队列文案跟随 MAX / A20=C23 排队指示器渲染点 /
//!   A21=C13 补口 clear_messages 清队列。
//!   C15 的单飞生产符号断言在 `tui_streaming_input_adversarial.rs::a7_*`。
//!
//! 权威符号（裁决口径，勿再引用作废的 `streaming_input_allowed` / `composer_enter_intent`）：
//!   `StreamingKeyDisposition{Forward,QueueSubmit,Reject}`、`streaming_key_disposition(&KeyEvent, bool)`、
//!   `is_panel_command`、`is_session_mutating_command`、`push_queued_message`、
//!   `drain_next_queued(&mut TuiState)`、`queued_status_label`、`should_drain_queued(..)`、
//!   `MAX_QUEUED_MESSAGES`、`TuiState::{queued_user_messages, queued_count}`。
//!
//! 手法与 `tui_six_fixes_acceptance.rs` 一致：`#[path]` 把 `src/bin/tui.rs` 编进测试进程，
//! 直接断言契约符号的**真实返回值**与真实状态突变（AAA；零假测试、不吞异常）。
//!
//! 缺陷真值（红相必须复现）：`tui.rs:9058` 流式守卫 + `tui.rs:9491` paste 早退。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui::StreamingKeyDisposition;

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn plain_char(c: char) -> KeyEvent {
    key(KeyCode::Char(c), KeyModifiers::NONE)
}

// ── A1 · L2-T ─ C1：流式中编辑类按键必须逐项放行（Forward = 走既有按键处理链）──

#[test]
fn a1_streaming_edit_keys_are_forwarded() {
    // Arrange：C1 全表（可打印字符 + 光标/编辑动作键 + Tab 补全 + Up/Down 选择）。
    let printable_and_edit = [
        plain_char('x'),
        plain_char('中'),
        key(KeyCode::Backspace, KeyModifiers::NONE),
        key(KeyCode::Delete, KeyModifiers::NONE),
        key(KeyCode::Left, KeyModifiers::NONE),
        key(KeyCode::Right, KeyModifiers::NONE),
        key(KeyCode::Home, KeyModifiers::NONE),
        key(KeyCode::End, KeyModifiers::NONE),
        key(KeyCode::Tab, KeyModifiers::NONE),
        key(KeyCode::Up, KeyModifiers::NONE),
        key(KeyCode::Down, KeyModifiers::NONE),
    ];

    // Act + Assert：流式期每一个都必须 Forward（无论输入框当前有无正文）。
    for k in printable_and_edit {
        for has_text in [false, true] {
            assert_eq!(
                tui::streaming_key_disposition(&k, has_text),
                StreamingKeyDisposition::Forward,
                "C1：流式中 {:?}（has_text={has_text}）必须 Forward 到输入框处理链\
                 （Reject/QueueSubmit = 按键被吞，缺陷复现）",
                k.code
            );
        }
    }

    // Ctrl+U/J/W/A/E：C1 明列，放行之外还必须真有编辑语义（否则只是换种方式吞键）。
    for c in ['u', 'j', 'w', 'a', 'e'] {
        let k = key(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(
            tui::streaming_key_disposition(&k, true),
            StreamingKeyDisposition::Forward,
            "C1：流式中 Ctrl+{c} 必须 Forward"
        );
        assert!(
            tui::composer_control_action(&k).is_some(),
            "C1：Ctrl+{c} 放行后必须有编辑动作"
        );
    }

    // 真实状态突变：被放行的可打印字符必须真能写进输入框（而不是只返回 Forward）。
    let mut state = tui::TuiState::default();
    state.is_streaming = true;
    let typed = plain_char('x');
    assert_eq!(
        tui::streaming_key_disposition(&typed, false),
        StreamingKeyDisposition::Forward,
        "C1：流式中 Char('x') 必须 Forward"
    );
    let KeyCode::Char(typed_char) = typed.code else {
        unreachable!("构造的按键必须是 Char")
    };
    state.insert_char(typed_char);
    assert_eq!(
        state.input_buffer, "x",
        "C1：放行的字符必须真正落进输入框（流式中不能空转）"
    );
    assert_eq!(state.input_cursor, 1, "光标必须随插入前移");
}

// ── A2 · L2-T ─ C2：流式中 Enter = 入队，绝不并发派发第二个 turn ─────────────

#[test]
fn a2_enter_intent_queues_while_streaming() {
    let enter = key(KeyCode::Enter, KeyModifiers::NONE);

    // 流式中且输入框有正文 ⇒ 入队（不是直接发送）。
    assert_eq!(
        tui::streaming_key_disposition(&enter, true),
        StreamingKeyDisposition::QueueSubmit,
        "C2：流式中 Enter + 有正文必须 QueueSubmit，而不是 spawn 第二个并发 turn"
    );

    // 空输入 ⇒ 拒绝（C6 同口径：空串/纯空白不入队）。
    assert_eq!(
        tui::streaming_key_disposition(&enter, false),
        StreamingKeyDisposition::Reject,
        "C6：空输入时 Enter 必须 Reject（不得入队空气泡）"
    );

    // 命令串 ⇒ 走 C3/C4 分发，绝不进入入队路径。
    // 两条互补断言：(a) 命令串不可能被 push 进队列；(b) 命令串都能被分类函数识别。
    let mut queue: Vec<String> = Vec::new();
    for cmd in ["/help", "/agents", "/models", "/clear", "/compact", "/resume"] {
        assert!(
            !tui::push_queued_message(&mut queue, cmd),
            "C2/C6：命令串 {cmd} 不得入队（必须走命令分发）"
        );
        assert!(
            tui::is_panel_command(cmd) || tui::is_session_mutating_command(cmd),
            "C2/C3/C4：命令串 {cmd} 必须能被分类函数识别后分发"
        );
    }
    assert!(
        queue.is_empty(),
        "C2：命令串被拒绝后不得留下排队残渣: {queue:?}"
    );

    // 硬断言：流式中任何带正文的 Enter 都不得被判定为 Forward（Forward=静态直接发送）。
    assert_ne!(
        tui::streaming_key_disposition(&enter, true),
        StreamingKeyDisposition::Forward,
        "C2：流式中 Enter 严禁 Forward（Forward 会走静态发送路径 ⇒ turn 并发）"
    );
}

// ── A3 · L2-T ─ C3/C4：面板类 vs 会话变更类命令分类 + 快捷键不被守卫拦截 ─────

#[test]
fn a3_panel_and_session_mutating_command_classification() {
    // C3（裁决后口径）：流式中必须照常执行的面板/只读命令（`/resume` 已移出）。
    let panel_commands = [
        "/agents",
        "/tasks",
        "/plan",
        "/models",
        "/thinking",
        "/provider",
        "/help",
        "/update",
    ];
    for cmd in panel_commands {
        assert!(
            tui::is_panel_command(cmd),
            "C3：{cmd} 是面板命令，流式中必须可用"
        );
        assert!(
            !tui::is_session_mutating_command(cmd),
            "C3/C4：{cmd} 不得被误判为会话变更命令（否则流式中会被拒绝）"
        );
    }

    // C4（裁决后口径）：流式中必须拒绝的会话变更类命令（含 `/resume`：中途换 session 会撕裂 turn）。
    let mutating_commands = ["/clear", "/compact", "/resume"];
    for cmd in mutating_commands {
        assert!(
            tui::is_session_mutating_command(cmd),
            "C4：{cmd} 流式中必须拒绝执行"
        );
        assert!(
            !tui::is_panel_command(cmd),
            "C3/C4：{cmd} 不得被误判为面板命令（分类必须互斥）"
        );
    }

    // C4 末句：/quit /exit 仍走原退出分支 ⇒ 既非面板亦非会话变更。
    for cmd in ["/quit", "/exit"] {
        assert!(
            !tui::is_session_mutating_command(cmd),
            "C4：{cmd} 流式中必须仍允许退出，不得被拦成会话变更命令"
        );
        assert!(!tui::is_panel_command(cmd), "C3：{cmd} 不是面板命令");
    }

    // 非命令文本不得被误判（否则普通聊天会被当成命令吞掉）。
    for text in [
        "hi",
        "",
        "/nonexistent-command",
        "hello /help",
        "/clear 这个需求怎么拆",
    ] {
        assert!(
            !tui::is_panel_command(text),
            "C3：{text:?} 不是面板命令，不得误判"
        );
        assert!(
            !tui::is_session_mutating_command(text),
            "C4：{text:?} 不是会话变更命令，不得误判"
        );
    }

    // C3：Ctrl+T（任务板）/ Ctrl+R（模型面板）不得被流式守卫拦掉（必须 Forward 才能到达面板分支）。
    // Ctrl+M 是模型面板的别名：非 kitty 协议终端由终端层降级为 Enter（既有提交行为），
    // 但只要上报为 Char('m')+CONTROL，流式守卫同样必须 Forward（不得被当作提交/换行吞掉）。
    for c in ['t', 'r', 'm'] {
        let shortcut = key(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(
            tui::streaming_key_disposition(&shortcut, false),
            StreamingKeyDisposition::Forward,
            "C3：流式中 Ctrl+{c} 必须 Forward 到面板分支（否则面板打不开）"
        );
    }
}

// ── A4 · L2-T ─ C5/C6：入队拒绝规则 + FIFO 一次性 drain（队列在 TuiState 上）──

#[test]
fn a4_queue_push_rejects_and_drains_fifo_once() {
    // Arrange
    assert_eq!(
        tui::MAX_QUEUED_MESSAGES, 20,
        "C6：队列容量上限冻结为 20"
    );
    let mut state = tui::TuiState::default();
    assert_eq!(state.queued_count(), 0, "默认态必须无排队");

    // Act + Assert：空串 / 纯空白 / 命令串一律拒绝，且不留残渣。
    assert!(
        !tui::push_queued_message(&mut state.queued_user_messages, ""),
        "C6：空串不得入队"
    );
    assert!(
        !tui::push_queued_message(&mut state.queued_user_messages, "   \n\t "),
        "C6：纯空白不得入队"
    );
    for cmd in ["/help", "/clear", "/quit"] {
        assert!(
            !tui::push_queued_message(&mut state.queued_user_messages, cmd),
            "C6：命令串 {cmd} 不得入队（走 C3/C4 分发）"
        );
    }
    assert_eq!(
        state.queued_count(),
        0,
        "C6：被拒绝的载荷不得留下残渣: {:?}",
        state.queued_user_messages
    );

    // 正常入队：载荷即提交时刻的正文快照。
    assert!(tui::push_queued_message(
        &mut state.queued_user_messages,
        "第一条"
    ));
    assert!(tui::push_queued_message(
        &mut state.queued_user_messages,
        "  第二条  "
    ));
    assert_eq!(state.queued_count(), 2, "两次合法入队后 count 必须为 2");
    assert_eq!(
        state.queued_user_messages[0].trim(),
        "第一条",
        "入队顺序必须保持"
    );
    assert_eq!(
        state.queued_user_messages[1].trim(),
        "第二条",
        "入队顺序必须保持"
    );

    // 填到上限：未满必须成功。
    while state.queued_count() < tui::MAX_QUEUED_MESSAGES {
        let text = format!("fill-{}", state.queued_count());
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, &text),
            "C6：未达上限时 push 必须成功: {text}"
        );
    }
    assert_eq!(state.queued_count(), tui::MAX_QUEUED_MESSAGES);

    // 满则拒绝：绝不静默丢弃既有条目，也不扩容。
    assert!(
        !tui::push_queued_message(&mut state.queued_user_messages, "第21条溢出"),
        "C6：达上限必须拒绝入队"
    );
    assert_eq!(
        state.queued_count(),
        tui::MAX_QUEUED_MESSAGES,
        "C6：拒绝后队列长度不得变化（不得静默丢弃）"
    );

    // drain：FIFO、一次只吐一条、走 TuiState 访达面。
    assert_eq!(
        tui::drain_next_queued(&mut state).as_deref(),
        Some("第一条"),
        "C5：drain 必须取队首（FIFO）"
    );
    assert_eq!(
        state.queued_count(),
        tui::MAX_QUEUED_MESSAGES - 1,
        "C5：一次 drain 只派发 1 条"
    );
    assert_eq!(
        tui::drain_next_queued(&mut state).as_deref(),
        Some("第二条"),
        "C5：第二条必须按入队顺序取出"
    );

    // 一次性：每条载荷只能被派发一次，吐完立即 None（不得阻塞/panic）。
    let mut drained = 2usize;
    while tui::drain_next_queued(&mut state).is_some() {
        drained += 1;
        assert!(
            drained <= tui::MAX_QUEUED_MESSAGES,
            "drain 总次数不得超过入队总数（出现重复派发）"
        );
    }
    assert_eq!(
        drained, tui::MAX_QUEUED_MESSAGES,
        "每条入队载荷必须且只能被派发一次"
    );
    assert_eq!(state.queued_count(), 0, "drain 干净后队列必须为空");
    assert_eq!(
        tui::drain_next_queued(&mut state),
        None,
        "C5：空队列 drain 必须立即返回 None（不得阻塞/panic）"
    );
}

// ── A5 · L2-T ─ C7：排队数量用户可见 ────────────────────────────────────────

#[test]
fn a5_queued_status_label_visible_only_when_nonzero() {
    assert_eq!(
        tui::queued_status_label(0),
        None,
        "C7：无排队时不得占用状态区文案"
    );
    for n in [1usize, 3, tui::MAX_QUEUED_MESSAGES] {
        let label = tui::queued_status_label(n).expect("C7：有排队时状态必须可见");
        assert!(
            label.contains("排队"),
            "C7：文案必须含「排队」字样: {label:?}"
        );
        assert!(
            label.contains(&n.to_string()),
            "C7：文案必须含真实数量 {n}: {label:?}"
        );
    }

    // 状态侧同源：queued_count 必须反映真实队列长度，且与 label 一一对应。
    let mut state = tui::TuiState::default();
    assert_eq!(state.queued_count(), 0, "默认态必须无排队");
    assert_eq!(
        tui::queued_status_label(state.queued_count()),
        None,
        "queued_count==0 时 label 必须为 None"
    );
    assert!(tui::push_queued_message(
        &mut state.queued_user_messages,
        "a"
    ));
    assert!(tui::push_queued_message(
        &mut state.queued_user_messages,
        "b"
    ));
    assert_eq!(
        state.queued_count(),
        2,
        "queued_count 必须等于 queued_user_messages.len()"
    );
    let label = tui::queued_status_label(state.queued_count()).expect("排队中必须可见");
    assert!(
        label.contains('2'),
        "label 与 queued_count 必须同源: {label:?}"
    );
}

// ── A6 · L2-T ─ C8 静态无回归 + C1 旧契约断言迁移 ────────────────────────────

#[test]
fn a6_static_path_unchanged_and_legacy_predicate_migrated() {
    // C8：静态口径完全不变 —— Enter 提交 / Shift+Enter 换行 / Ctrl+U 清空。
    assert_eq!(
        tui::composer_enter_action(&key(KeyCode::Enter, KeyModifiers::NONE)),
        tui::ComposerEnterAction::Submit,
        "C8：裸 Enter 仍是提交"
    );
    assert_eq!(
        tui::composer_enter_action(&key(KeyCode::Enter, KeyModifiers::SHIFT)),
        tui::ComposerEnterAction::Newline,
        "C8：Shift+Enter 仍是换行"
    );
    assert_eq!(
        tui::composer_control_action(&key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        Some(tui::ComposerControlAction::ClearInput),
        "C8：Ctrl+U 仍是清空输入框"
    );

    // C8：静态态不排队、不流式。
    let state = tui::TuiState::default();
    assert!(!state.is_streaming, "默认态必须是静态");
    assert_eq!(state.queued_count(), 0, "静态态不得有排队残留");
    assert_eq!(
        tui::queued_status_label(state.queued_count()),
        None,
        "C8：静态空队列不得渲染排队文案"
    );

    // C1 迁移（Lead 裁决：契约演进，非测试放水）：
    // 旧断言「流式中 Char('x') 被 should_skip_while_streaming 丢弃」作废；
    // 新真值 = `streaming_key_disposition(Char('x'), _) == Forward`。
    for has_text in [false, true] {
        assert_eq!(
            tui::streaming_key_disposition(&plain_char('x'), has_text),
            StreamingKeyDisposition::Forward,
            "C1：旧断言作废 —— 流式中可打印字符必须 Forward（不再是「应被丢弃」）"
        );
    }

    // 静态口径等价：编辑键在流式裁决面同样是 Forward（Forward = 静态既有处理链）。
    for k in [
        key(KeyCode::Backspace, KeyModifiers::NONE),
        key(KeyCode::Left, KeyModifiers::NONE),
        key(KeyCode::Tab, KeyModifiers::NONE),
    ] {
        assert_eq!(
            tui::streaming_key_disposition(&k, false),
            StreamingKeyDisposition::Forward,
            "C8：静态既有按键链（{:?}）在裁决面必须 Forward",
            k.code
        );
    }
}

// ── A9 · L2-T ─ C10：终态派发闸门真值矩阵（机器可判）────────────────────────

#[test]
fn a9_should_drain_queued_truth_matrix() {
    // 参数顺序（C10 修订口径）：(is_streaming, ui_idle, queued, turn_in_flight, dispatch_pending)
    // 「turn_in_flight」= **轮次线程存活标志**（不是「有 armed goal」）：续轮不置 is_streaming，
    // 只看 goal 存在会让「armed 但未跑」的会话排队永不自动派发。
    // 全部「拒绝」组合逐项断言（真值矩阵）。
    let denied = [
        ((true, true, 3usize, false, false), "流式中不派发"),
        ((false, false, 3, false, false), "有面板/模态占用不派发"),
        ((false, true, 0, false, false), "空队列不派发"),
        ((false, true, 3, true, false), "轮次线程仍存活（turn_in_flight）⇒ 禁止派发"),
        ((false, true, 3, false, true), "已有一条在派发中 ⇒ 单飞"),
    ];
    for ((is_streaming, ui_idle, queued, turn_in_flight, dispatch_pending), why) in denied {
        assert!(
            !tui::should_drain_queued(is_streaming, ui_idle, queued, turn_in_flight, dispatch_pending),
            "C10：{why} ⇒ 必须 false（实参 streaming={is_streaming}, idle={ui_idle}, queued={queued}, turn_in_flight={turn_in_flight}, pending={dispatch_pending}）"
        );
    }

    // 唯一放行组合。
    assert!(
        tui::should_drain_queued(false, true, 3, false, false),
        "C10：静态 + 界面空闲 + 有排队 + 无在飞轮次线程 + 无在派发 ⇒ 必须 true"
    );

    // 队列边界（同一规则的 queued==1 下界）。
    assert!(
        tui::should_drain_queued(false, true, 1, false, false),
        "C10：queued==1 是最小可派发边界，必须 true"
    );
}

// ── A11 · L2-T ─ C12：流式期反馈必须走可见通道 ───────────────────────────────

#[test]
fn a11_streaming_feedback_routes_to_visible_channel() {
    // C12（L3-B B1）回归锁：**channel 决定主循环走系统消息通道还是 `status_text`**。
    // 流式期 `status_notice` 的渲染点被 `should_render_status_notice` 的 `!is_streaming` 抑制 ⇒
    // C4（会话变更被拒）/ C6（队列已满）必须落到流式可见通道，否则用户永远看不到反馈。
    // 旧版 A11 只间接断言 C7/C4/C6 判定，L3-B 反向实验证明它抓不住 B1 回归（删掉两处
    // `messages.push` 后仍全绿）——故本版直接锁「kind → channel」映射。
    //
    // C23 修订（第四轮）：流式期 C4/C6 的实际渲染点改为 `state.queue_notice`
    // ⇒ 由 `queue_indicator_line` 每帧呈现（见 A20）；`messages.push(System)` 只在静态期留痕。
    // 因此本用例锁的是「kind → channel 映射」这条口径本身，渲染点由 A20 负责断言。
    use tui::{QueueFeedbackChannel, QueueFeedbackKind};

    let (session_mutating_channel, session_mutating_text) =
        tui::queue_feedback(QueueFeedbackKind::SessionMutating);
    assert_eq!(
        session_mutating_channel,
        QueueFeedbackChannel::SystemMessage,
        "C12：C4「本轮结束后可用」必须走 SystemMessage（流式可见），走 StatusText 会被流式期抑制"
    );
    assert!(
        !session_mutating_text.trim().is_empty(),
        "C12：SessionMutating 文案不得为空（否则用户只看到空白系统消息）"
    );

    let (queue_full_channel, queue_full_text) = tui::queue_feedback(QueueFeedbackKind::QueueFull);
    assert_eq!(
        queue_full_channel,
        QueueFeedbackChannel::SystemMessage,
        "C12：C6「队列已满」必须走 SystemMessage（流式可见）"
    );
    assert!(
        !queue_full_text.trim().is_empty(),
        "C12：QueueFull 文案不得为空"
    );
    assert!(
        queue_full_text.contains("20") || queue_full_text.contains("已满") || queue_full_text.contains("满"),
        "C12：QueueFull 文案必须含上限数字或「满」类词: {queue_full_text:?}"
    );

    let (enqueued_channel, enqueued_text) = tui::queue_feedback(QueueFeedbackKind::Enqueued);
    assert_eq!(
        enqueued_channel,
        QueueFeedbackChannel::StatusText,
        "C12：C2「已排队 N 条」走 StatusText（配合 C7 指示器），不得挤占系统消息区"
    );
    assert!(
        !enqueued_text.trim().is_empty(),
        "C12：Enqueued 文案不得为空"
    );
    assert_ne!(
        session_mutating_text, queue_full_text,
        "C12：不同反馈类型的文案必须可区分（否则用户分不清是哪个原因）"
    );
}

// ── A12 · L2-T ─ C13：会话作用域复位必须清空排队 ─────────────────────────────

#[test]
fn a12_reset_session_scope_drops_queued_messages() {
    // C13（L3-A#1 / L3-P#3）：`/clear`、`/resume`、hydrate 共用 `reset_session_scope`，
    // 必须清空 `queued_user_messages`，禁止旧会话排队消息被派发进新会话。
    let mut state = tui::TuiState::default();
    for message in ["旧会话-1", "旧会话-2", "旧会话-3"] {
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, message),
            "Arrange：{message} 必须入队"
        );
    }
    assert_eq!(state.queued_count(), 3, "Arrange：必须真的排了 3 条");

    // Act
    state.reset_session_scope();

    // Assert
    assert_eq!(
        state.queued_count(),
        0,
        "C13：reset_session_scope 必须清空排队（否则旧会话消息会派发进新会话）"
    );
    assert_eq!(
        tui::queued_status_label(state.queued_count()),
        None,
        "C7/C13：清空后不得残留排队指示器"
    );
    assert_eq!(
        tui::drain_next_queued(&mut state),
        None,
        "C13：清空后 drain 必须拿不到任何旧会话消息"
    );
}

// ── A13 · L2-T ─ C14：派发写回正文净化（载体：composer_buffer_for_dispatch）────

#[test]
fn a13_dispatch_write_back_neutralizes_newline_escape_and_command_prefix() {
    // C14 载体裁定（lead, 2026-11-06）：净化由**纯函数** `composer_buffer_for_dispatch(&str) -> String`
    // 承载（返回「正文 + 一个尾部空格」，正文本身不 trim）；主循环用
    // `set_input(&composer_buffer_for_dispatch(&msg))` 写回；`take_next_queued_for_dispatch`
    // **只负责取件、不写回**（取件语义与写回语义解耦）。
    //
    // 旧 A13 断言（取件后直接检查 state.input_buffer 被写回）**作废**：它把两种职责耦合在
    // 同一个符号上，与 C14/C15 的职责划分冲突（不是测试放水，是载体改判）。

    // ① 行尾 `\` 的自救路径（Enter ⇒ 换行，正文永远发不出去）
    let escaped = tui::composer_buffer_for_dispatch("hello\\");
    assert!(
        !escaped.ends_with('\\'),
        "C14：写回缓冲必须以尾部空格断开 `\\` 结尾（否则 Enter 走「换行」分支永不提交）: {escaped:?}"
    );
    assert!(
        escaped.ends_with(' '),
        "C14：写回缓冲必须恰好以尾部空格结尾: {escaped:?}"
    );
    assert_eq!(
        escaped.trim(),
        "hello\\",
        "C14：正文内容不得被改写（提交路径 trim 后仍是原文）: {escaped:?}"
    );

    // ② 命令前缀的自救路径（`/hel` 被补全劫持，正文永远发不出去）
    let prefix = tui::composer_buffer_for_dispatch("/hel");
    assert_eq!(prefix, "/hel ", "C14：命令前缀正文只追加一个尾部空格");
    let mut state = tui::TuiState::default();
    state.set_input(&prefix);
    assert!(
        state.matching_commands().is_empty(),
        "C14：写回后不得被命令补全劫持（matching_commands 必须为空）: {:?}",
        state.input_buffer
    );
    assert_eq!(
        state.input_buffer.trim(),
        "/hel",
        "C14：正文不得丢失（trim 后等于原始排队正文）: {:?}",
        state.input_buffer
    );

    // ③ 取件与写回解耦：take 只取件、不改输入框；返回正文与净化函数的输入一一对应。
    let mut state = tui::TuiState::default();
    assert!(
        tui::push_queued_message(&mut state.queued_user_messages, "hello\\"),
        "Arrange：以 `\\` 结尾的正文必须能入队"
    );
    assert!(
        tui::push_queued_message(&mut state.queued_user_messages, "/hel"),
        "Arrange：命令前缀正文（非完整命令）必须能入队"
    );
    let first = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false)
        .expect("C15：放行组合必须取到消息");
    assert_eq!(
        first, "hello\\",
        "C14/C15：取件必须返回**原始正文**（净化不属于取件职责）"
    );
    assert!(
        state.input_buffer.trim().is_empty(),
        "C14/C15：取件不得写回输入框（写回由 composer_buffer_for_dispatch + set_input 负责）: {:?}",
        state.input_buffer
    );
    let second = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false)
        .expect("C15：第二条仍可取件");
    assert_eq!(second, "/hel", "C14/C15：第二条正文同样原样返回");
    for message in [first.as_str(), second.as_str()] {
        let written = tui::composer_buffer_for_dispatch(message);
        assert_eq!(
            written.trim(),
            message.trim(),
            "C14：净化函数必须与取件正文一一对应（trim 后内容一致）: {message:?} -> {written:?}"
        );
        assert!(written.ends_with(' '), "C14：净化结果必须带尾部空格: {written:?}");
    }
}

// ── A14 · L2-T ─ C17：挂起/待作答期禁止派发 ─────────────────────────────────

#[test]
fn a14_suspended_turn_blocks_dispatch() {
    // C17（L3-A#2）：`TurnSuspended` 会置 `is_streaming=false` 而 run 未收口；
    // `ui_idle` 必须同时要求「无待作答」，否则派发会造成 run 交叠。
    let mut state = tui::TuiState::default();
    assert!(
        tui::push_queued_message(&mut state.queued_user_messages, "挂起期的排队消息"),
        "Arrange：必须能入队"
    );
    assert_eq!(state.queued_count(), 1);

    // Act：ui_idle=false（挂起/待作答）
    let blocked = tui::take_next_queued_for_dispatch(&mut state, false, false, false, false);

    // Assert
    assert_eq!(
        blocked, None,
        "C17：挂起/待作答期（ui_idle=false）必须禁止派发，否则 run 交叠"
    );
    assert_eq!(state.queued_count(), 1, "C17：被拒绝的派发不得消费队列");
}

// ── A15 · L2-T ─ C18：别名命令不入队 + 容量语义不变 ─────────────────────────

#[test]
fn a15_alias_commands_never_enter_queue() {
    // C18 ①：`push_queued_message` 拒绝集补别名 `/exit` `/effort`（走原分发/退出分支，不排队）。
    let mut queue: Vec<String> = Vec::new();
    for alias in ["/exit", "/effort", "/quit"] {
        assert!(
            !tui::push_queued_message(&mut queue, alias),
            "C18：别名 {alias} 不得入队"
        );
    }
    assert!(queue.is_empty(), "C18：被拒绝的别名不得留下残渣: {queue:?}");

    // C6/C18 ③：容量语义不变（上限 20，未满接受，满则拒绝）。
    assert_eq!(
        tui::MAX_QUEUED_MESSAGES, 20,
        "C6：队列容量上限语义不变"
    );
    let mut state = tui::TuiState::default();
    let mut accepted = 0usize;
    while state.queued_count() < tui::MAX_QUEUED_MESSAGES {
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, "正文"),
            "C6：未达上限必须接受"
        );
        accepted += 1;
    }
    assert_eq!(
        accepted, tui::MAX_QUEUED_MESSAGES,
        "C6：上限前必须恰好接受 20 条"
    );
    assert!(
        !tui::push_queued_message(&mut state.queued_user_messages, "第21条"),
        "C6：达上限必须拒绝"
    );
}

// ── A16 · L2-T ─ C19：计划面板的按键吞并面 ──────────────────────────────────

#[test]
fn a16_plan_mode_consumes_only_navigation_keys() {
    // C19：计划面板打开时只吞「导航/退出」类按键，不得吞掉编辑与提交键
    // （否则面板一开，输入框就废了 —— 与 C1 同源的防御）。
    for code in [KeyCode::Esc, KeyCode::Up, KeyCode::Down] {
        assert!(
            tui::plan_mode_consumes_key(&key(code, KeyModifiers::NONE)),
            "C19：计划面板必须吞掉 {code:?}（导航/退出）"
        );
    }
    for code in [
        KeyCode::Char('a'),
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Left,
        KeyCode::Tab,
    ] {
        assert!(
            !tui::plan_mode_consumes_key(&key(code, KeyModifiers::NONE)),
            "C19：计划面板不得吞掉 {code:?}（编辑/提交会被抢走）"
        );
    }
}

// ── A17 · L2-T ─ C20：流式期 Enter 的前置优先级 ──────────────────────────────

#[test]
fn a17_streaming_enter_pre_action_precedes_queue_submit() {
    // C20：主循环的 Enter 有**前置分支**——① 输入以 `\` 结尾 ⇒ 换行；
    // ② 补全可见 ⇒ 补全；两者都不成立才轮到 C2 的 QueueSubmit。
    // 载体：`streaming_enter_pre_action(input, completion_visible) -> StreamingEnterPre`。
    use tui::StreamingEnterPre;

    let enter = key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        tui::streaming_key_disposition(&enter, true),
        StreamingKeyDisposition::QueueSubmit,
        "C2 基准：disposition 仍判 QueueSubmit（本用例测的是它**之前**的前置分支）"
    );
    assert_eq!(
        tui::streaming_enter_pre_action("hello\\", false),
        StreamingEnterPre::Newline,
        "C20：行尾 `\\` 必须前置换行，不得入队（否则正文永远发不出去）"
    );
    assert_eq!(
        tui::streaming_enter_pre_action("/hel", true),
        StreamingEnterPre::Complete,
        "C20：补全可见必须前置补全，不得入队"
    );
    assert_eq!(
        tui::streaming_enter_pre_action("hello", false),
        StreamingEnterPre::Submit,
        "C20：无前置条件才允许走 C2 入队"
    );
    // C20 ②的「非完整命令」限定：`/help` 是完整命令 ⇒ Submit（走命令分发），不是 Complete。
    assert_eq!(
        tui::streaming_enter_pre_action("/help", true),
        StreamingEnterPre::Submit,
        "C20：完整命令（/help）即使补全可见也必须 Submit（走命令分发），不得被当成补全目标"
    );
    // `/` 前缀 + 补全可见 + 非完整命令 ⇒ Complete（`/hel` 不是完整命令）。
    assert_eq!(
        tui::streaming_enter_pre_action("/hel", true),
        StreamingEnterPre::Complete,
        "C20：/hel 是前缀而非完整命令 ⇒ 接受补全"
    );
    // 静态 Enter 语义不得被前置分支改动。
    assert_eq!(
        tui::composer_enter_action(&enter),
        tui::ComposerEnterAction::Submit,
        "C8/C20：裸 Enter 的提交语义不变"
    );
}

// ── A18 · L2-T ─ C21：回灌自动提交只在界面空闲时放行 ─────────────────────────

#[test]
fn a18_auto_submit_reinjection_only_when_ui_is_free() {
    // C21：回灌合成 Enter 若在模态/待答/审批期间触发，会把排队正文打进对话框。
    assert!(
        tui::should_reinject_auto_submit(false, false, false, false),
        "C21：无模态/无待决请求/无提问/无审批 ⇒ 必须允许回灌自动提交"
    );
    let blocked = [
        (true, false, false, false, "模态面板占用"),
        (false, true, false, false, "有待决宿主控制请求"),
        (false, false, true, false, "有 AskUser 提问待答"),
        (false, false, false, true, "有权限审批待决"),
    ];
    for (mode_is_modal, has_pending_request, has_ask, has_permission, why) in blocked {
        assert!(
            !tui::should_reinject_auto_submit(
                mode_is_modal,
                has_pending_request,
                has_ask,
                has_permission
            ),
            "C21：{why} ⇒ 禁止回灌自动提交"
        );
    }
}

// ── A19 · L2-T ─ C22 边界：满队列文案必须跟随 MAX_QUEUED_MESSAGES ─────────────

#[test]
fn a19_queue_full_feedback_tracks_max_capacity() {
    // C22：文案里的上限数字必须是常量来源，不得硬编码漂移。
    use tui::QueueFeedbackKind;

    // 真实达上限（与文案同源验证）：填满后第 21 条被拒。
    let mut state = tui::TuiState::default();
    while state.queued_count() < tui::MAX_QUEUED_MESSAGES {
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, "填充"),
            "C6：未达上限必须接受"
        );
    }
    assert!(
        !tui::push_queued_message(&mut state.queued_user_messages, "第21条"),
        "C6：达上限必须拒绝"
    );
    assert_eq!(state.queued_count(), tui::MAX_QUEUED_MESSAGES);

    let (channel, text) = tui::queue_feedback(QueueFeedbackKind::QueueFull);
    assert_eq!(
        channel,
        tui::QueueFeedbackChannel::SystemMessage,
        "C12/C22：满队列反馈必须走可见通道"
    );
    let numbers: Vec<&str> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|chunk| !chunk.is_empty())
        .collect();
    assert!(
        !numbers.is_empty() || text.contains("满"),
        "C22：满队列文案必须含上限数字或「满」字样: {text:?}"
    );
    for number in numbers {
        assert_eq!(
            number,
            tui::MAX_QUEUED_MESSAGES.to_string(),
            "C22：文案中的数字必须等于 MAX_QUEUED_MESSAGES（防硬编码漂移）: {text:?}"
        );
    }
}

// ── A20 · L2-T ─ C23：排队指示器在流式 / 工具执行期必须有渲染点 ───────────────

#[test]
fn a20_queue_indicator_line_covers_streaming_and_tool_execution() {
    // C23（L3-A 复审 BLOCKING）：流式 + 工具执行期「排队 N 条」此前**没有任何渲染点**。
    // 真值矩阵：只要 `queued>0 || notice.is_some()` 就必须 Some，且与两个 bool 取值无关。
    assert_eq!(
        tui::queue_indicator_line(0, None, false, false),
        None,
        "C23：(0, None, 静态, 非工具) ⇒ None（不占行）"
    );
    assert_eq!(
        tui::queue_indicator_line(0, None, true, true),
        None,
        "C23：(0, None, 流式, 工具执行) ⇒ None（无排队无 notice 不占行）"
    );

    // 关键回归锁：四组 bool 组合下 queued=2 都必须有渲染点（executing_tool=true 尤其不能漏）。
    for (is_streaming, executing_tool) in [(true, false), (true, true), (false, true), (false, false)] {
        let line = tui::queue_indicator_line(2, None, is_streaming, executing_tool);
        assert!(
            line.is_some(),
            "C23：queued=2 时 (is_streaming={is_streaming}, executing_tool={executing_tool}) 必须有渲染点\
             —— executing_tool=true 缺渲染点正是 BLOCKING 缺陷"
        );
        let line = line.expect("上面已断言 Some");
        assert!(
            line.contains('2'),
            "C23：指示器文案必须含排队数量 2: {line:?}"
        );
        assert!(
            line.contains("排队"),
            "C23：指示器文案必须含「排队」字样: {line:?}"
        );
    }

    // notice 单独存在（无排队）⇒ 仍必须可见，且文案包含 notice 内容。
    let notice_line =
        tui::queue_indicator_line(0, Some("本轮结束后可用：/clear"), true, true)
            .expect("C23：仅有 notice 时也必须有渲染点（流式+工具执行期）");
    assert!(
        notice_line.contains("本轮结束后可用：/clear"),
        "C23：指示器必须包含 notice 内容: {notice_line:?}"
    );

    // notice 与计数并存 ⇒ 二者必须**同时出现**且**notice 在前、计数在后**。
    // （L3-B ADVISORY：旧断言 `contains("排队") || !trim().is_empty()` 是装饰性的，抓不住「notice 吞掉计数」。）
    // 用不含数字的 notice 才能把「计数串」与「notice 自带数字」区分开。
    let notice_only = "本轮结束后可用：/clear";
    let both_line = tui::queue_indicator_line(2, Some(notice_only), true, true)
        .expect("C23：notice+计数并存时必须有渲染点");
    assert!(
        both_line.contains(notice_only),
        "C23：并存时 notice 文案必须保留: {both_line:?}"
    );
    let count_text = "2 条";
    assert!(
        both_line.contains(count_text),
        "C23：并存时计数必须以「2 条」形式同时出现（不得被 notice 吞掉）: {both_line:?}"
    );
    let notice_pos = both_line
        .find(notice_only)
        .expect("上面已断言 notice 存在");
    let count_pos = both_line
        .find(count_text)
        .expect("上面已断言计数存在");
    assert!(
        notice_pos < count_pos,
        "C23：顺序必须是 notice 在前、计数在后（先说原因再说数量）: {both_line:?}"
    );

    // 两位数计数同样必须可见（防止只对个位数成立）。
    let line_two_digit = tui::queue_indicator_line(12, Some(notice_only), false, true)
        .expect("C23：两位数计数必须有渲染点");
    assert!(
        line_two_digit.contains("12 条"),
        "C23：两位数计数必须完整可见（不得截断/丢位）: {line_two_digit:?}"
    );
}

// ── A21 · L2-T ─ C13 补口：clear_messages 也必须清空排队 ─────────────────────

#[test]
fn a21_clear_messages_drops_queued_messages() {
    // C13 补口：`clear_messages()` 是换 session 的另一入口（与 `reset_session_scope` 并列）。
    // 现状核实（读 src 得到，非猜测）：`clear_messages` 目前只清 `messages` 并轮换 `session_id`，
    // **未**调用 `reset_session_scope`；因此本用例断言的是它自身的端到端行为：
    // 清空正文的同时必须清空 `queued_user_messages`，否则旧会话排队消息会被派发进新会话。
    // （若实现方改为内部调用 `reset_session_scope`，本用例同样成立——断言的是可观测结果。）
    let mut state = tui::TuiState::default();
    assert!(
        tui::push_queued_message(&mut state.queued_user_messages, "旧会话-甲"),
        "Arrange：第一条必须入队"
    );
    assert!(
        tui::push_queued_message(&mut state.queued_user_messages, "旧会话-乙"),
        "Arrange：第二条必须入队"
    );
    assert_eq!(state.queued_count(), 2, "Arrange：必须真的排了 2 条");

    // Act
    state.clear_messages();

    // Assert
    assert_eq!(
        state.queued_count(),
        0,
        "C13：clear_messages 必须清空排队（换 session 的另一入口）"
    );
    assert_eq!(
        tui::drain_next_queued(&mut state),
        None,
        "C13：清空后不得再取出旧会话消息"
    );
    assert!(
        state.queue_notice.is_none(),
        "C23/C13：clear_messages 必须一并清空 queue_notice（否则旧会话提示会残留显示）"
    );
}

// ── A22 · L2-T ─ C24：放弃派发后正文回队首，不得丢序也不得停摆 ────────────────

#[test]
fn a22_requeue_front_restores_fifo_and_never_stalls() {
    // C24：回灌自动提交被放弃（模态/待答/审批期间，或用户改了草稿）时，正文必须**回到队首**，
    // 既不能丢、也不能插到队尾打乱 FIFO，更不能让队列永久停摆。
    let mut state = tui::TuiState::default();
    assert!(tui::push_queued_message(&mut state.queued_user_messages, "a"));
    assert!(tui::push_queued_message(&mut state.queued_user_messages, "b"));
    assert_eq!(state.queued_count(), 2);

    // Act 1：生产取件拿队首 "a"（取件不写回，见 A13）。
    let taken = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false)
        .expect("C15：放行组合必须能取件");
    assert_eq!(taken, "a", "Arrange：必须取到队首");
    assert_eq!(state.queued_count(), 1);

    // Act 2：放弃派发 ⇒ 回队首。
    tui::requeue_front(&mut state, taken);

    // Assert 1：队列恢复成 ["a","b"]（回队首、不丢序）。
    assert_eq!(
        state.queued_user_messages,
        vec!["a".to_string(), "b".to_string()],
        "C24：requeue_front 必须把正文插回队首，保持 FIFO"
    );
    assert_eq!(state.queued_count(), 2);

    // Assert 2：放弃路径 = 清空输入框 + 回队 ⇒ 输入框干净、队列完整。
    let taken = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false)
        .expect("C15：回队后必须仍可取件");
    state.set_input("a "); // 模拟用户已把正文写回输入框（C14 写回形态）
    state.clear_input(); // 放弃：清空输入框
    tui::requeue_front(&mut state, taken);
    assert_eq!(state.queued_count(), 2, "C24：放弃路径后队列必须仍是 2 条");
    assert!(
        state.input_buffer.is_empty(),
        "C24：放弃路径必须清空输入框: {:?}",
        state.input_buffer
    );

    // C26 等价性（A22 增补，Lead 指定）：撤销路径后「派发令牌 → 载荷」必须被判为**分歧**，
    // 即消费端不得把已被抹空/被改写的正文当成「用户改过的草稿」而静默丢弃。
    // （实现若无 `queue_dispatch_payload` 字段对外暴露，本等价断言即其语义覆盖：
    //   dispatched=Some("a") 对空 input_buffer 必须判 true；且回队后总条数不变。）
    assert!(
        tui::queued_dispatch_diverged(Some("a"), &state.input_buffer),
        "C26：撤销路径（输入框已被清空）必须判定为载荷分歧"
    );
    assert_eq!(
        state.queued_count(),
        2,
        "C26：判定分歧本身不得改队列（恢复由 requeue_front 负责）"
    );

    // Assert 3：不得停摆 —— 回队后生产闸门必须仍能取到 "a"。
    let again = tui::take_next_queued_for_dispatch(&mut state, false, true, false, false);
    assert_eq!(
        again.as_deref(),
        Some("a"),
        "C24：回队后闸门必须重新能取到队首（不得停摆）"
    );

    // 补充（C24 活性）：派发闩有 TTL，过期后必须放行，避免 body 若丢事件导致永久停摆。
    assert!(
        tui::queue_dispatch_latch_active(Some(
            std::time::Instant::now() + std::time::Duration::from_secs(2)
        )),
        "C24：闩截止时间在未来 ⇒ 视为仍生效"
    );
    assert!(
        !tui::queue_dispatch_latch_active(Some(std::time::Instant::now())),
        "C24：闩到期时刻（now == deadline，严格小于）⇒ 必须失效放行"
    );
    assert!(
        !tui::queue_dispatch_latch_active(None),
        "C24：无闩 ⇒ 必须放行"
    );
    assert!(
        tui::QUEUE_DISPATCH_LATCH_MS > 0,
        "C24：闩 TTL 必须为正数（0 会导致每次派发都被立刻判过期/或永不过期）"
    );
}

// ── A23 · L2-T ─ C25：派发/拒绝日志字段承载（纯函数 + 结构契约）───────────────

/// bin 源码原文（结构契约断言用；这是**源码结构断言**，不是行为断言）。
const TUI_SRC: &str = include_str!("../src/bin/tui.rs");

/// `tui/input.rs` 源码原文：`99b3adce` 把输入层从 `tui.rs` 原地拆出，`key_class_label` 随之外移。
/// 结构契约断言的扫描范围必须显式覆盖被拆出的模块文件，否则函数搬家后断言会静默失配；
/// 读不到即 panic（fail-closed，模块再次搬家时用例立刻红，而不是去测一段空字符串）。
fn tui_input_src() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/tui/input.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("必须能读取 {}: {e}", path.display()))
}

/// 取 `signature_prefix` 起、到该函数体结束（列 0 的 `}`）为止的源码切片。
fn fn_body<'a>(src: &'a str, signature_prefix: &str) -> &'a str {
    let start = src
        .find(signature_prefix)
        .unwrap_or_else(|| panic!("源码中找不到 `{signature_prefix}`"));
    let rest = &src[start..];
    let end = rest.find("\n}\n").unwrap_or(rest.len());
    &rest[..end]
}

/// 取 `needle` 出现处起的 `len` 字节窗口（按字符边界收缩）。
fn window_after<'a>(src: &'a str, needle: &str, len: usize) -> &'a str {
    let start = src
        .find(needle)
        .unwrap_or_else(|| panic!("源码中找不到 `{needle}`"));
    let mut end = (start + len).min(src.len());
    while end > start && !src.is_char_boundary(end) {
        end -= 1;
    }
    &src[start..end]
}

#[test]
fn a23_dispatch_and_rejection_logs_carry_depth_and_key_class() {
    // C25：结构化日志必须可被机器判读 —— 派发事件带队列深度前后值，拒绝事件带按键分类。
    // ① 纯函数承载（行为断言）：字段名集合与顺序固定。
    let before = 5usize;
    let after = 4usize;
    let fields = tui::queue_dispatch_fields(before, after);
    let names: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        vec!["queue_depth_before", "queue_depth_after"],
        "C25：queue_dispatch_fields 必须恒定返回这两个字段且顺序固定，实得 {names:?}"
    );
    assert_eq!(
        fields[0].1,
        serde_json::json!(before),
        "C25：queue_depth_before 必须等于派发前队列深度"
    );
    assert_eq!(
        fields[1].1,
        serde_json::json!(after),
        "C25：queue_depth_after 必须等于派发后队列深度"
    );

    // ② 结构契约断言（源码文本）：调用点必须真的用上这些字段，避免函数写了却零引用。
    let dispatch_window = window_after(TUI_SRC, "\"composer_queue_dispatch\"", 700);
    assert!(
        dispatch_window.contains("tui_queue_log"),
        "C25：composer_queue_dispatch 附近必须有结构化日志调用"
    );
    assert!(
        dispatch_window.contains("queue_dispatch_fields("),
        "C25：composer_queue_dispatch 必须经 queue_dispatch_fields 写入深度字段: {dispatch_window:?}"
    );
    assert!(
        fn_body(TUI_SRC, "fn tui_queue_log").contains("session_id")
            && fn_body(TUI_SRC, "fn tui_queue_log").contains("turn_id"),
        "C16：日志底座必须含 session_id 与 turn_id 标识"
    );

    // ③ 拒绝事件必须带按键分类（且不得记录正文字符）。
    let rejected_window = window_after(TUI_SRC, "\"composer_key_rejected\"", 700);
    assert!(
        rejected_window.contains("key_class"),
        "C25：composer_key_rejected 必须带 key_class 字段: {rejected_window:?}"
    );

    // ④ 按键分类函数口径：Char 只记类别不记正文（防把用户输入写进日志）。
    // 该函数已随 `99b3adce` 从 `tui.rs` 外移到 `tui/input.rs`，扫描范围同步覆盖该模块文件。
    let input_src = tui_input_src();
    let classifier = fn_body(&input_src, "fn key_class_label");
    assert!(
        classifier.contains("\"Char\""),
        "C25：key_class_label 对字符键必须只记类别 \"Char\": {classifier:?}"
    );
}

// ── A24 · L2-T ─ C23 接线护栏：metrics_line 必须真的渲染排队指示器 ────────────

#[test]
fn a24_metrics_line_wires_queue_indicator_rendering() {
    // L3-B ADVISORY：A16/A19–A21 只锁纯函数，接线无护栏（把 metrics_line 里的渲染块删掉仍全绿）。
    // 本用例用源码结构断言锁住「渲染函数体 → queue_indicator_line」这条接线，删块即红。
    // ADR 0078 把底面行契约从「工作区 + 指标」改为「一次性提示/排队反馈 + 模型身份」，
    // 渲染函数同步由 `metrics_line` 更名为 `status_line`（tui.rs `fn status_line`）。
    // 断言目标仍是同一个**每帧渲染**的输入框下方行，接线护栏不变。
    let status_body = fn_body(TUI_SRC, "fn status_line");
    assert!(
        status_body.contains("queue_indicator_line("),
        "C23/C16：status_line 渲染函数体必须调用 queue_indicator_line（删掉渲染块会让流式期排队不可见）"
    );
    // 指示器必须由 state 的实时队列/notice 驱动，而不是常量或其他字段。
    assert!(
        status_body.contains("queued_count()"),
        "C23：status_line 必须以 state.queued_count() 作为排队计数来源"
    );
    assert!(
        status_body.contains("queue_notice"),
        "C23：status_line 必须读取 state.queue_notice（C4/C6 反馈的流式渲染点）"
    );
    // status_word 不得重复渲染排队指示器（C18 ③ 去重要求）。
    let status_word_body = fn_body(TUI_SRC, "fn status_word");
    assert!(
        !status_word_body.contains("queue_indicator_line("),
        "C18/C23：status_word 不得重复渲染排队指示器（去重口径）"
    );
}

// ── A25 · L2-T ─ C26：派发令牌与载荷绑定（分歧判定）────────────────────────

#[test]
fn a25_queued_dispatch_diverged_matrix() {
    // C26（L3-A round4 BLOCKING）：派发令牌记录了「派发了哪条正文」，消费端必须能判定
    // 输入框里的正文是否还等于该载荷；不等即「分歧」，必须走恢复（回队首）而不是静默丢弃。
    assert!(
        !tui::queued_dispatch_diverged(None, "任意"),
        "C26：无派发令牌 ⇒ 谈不上分歧（false）"
    );
    assert!(
        !tui::queued_dispatch_diverged(Some("m1"), "m1 "),
        "C26：仅尾部空格差异（trim 相等）⇒ 不分歧"
    );
    assert!(
        tui::queued_dispatch_diverged(Some("m1"), ""),
        "C26：输入被 Ctrl+W/Backspace/paste 抹空 ⇒ 必须判分歧（trace-B 静默丢正文的入口）"
    );
    assert!(
        tui::queued_dispatch_diverged(Some("m1"), "m1 用户追加"),
        "C26：用户在派发后又追加内容 ⇒ 必须判分歧（正文已不再等于载荷）"
    );
    assert!(
        !tui::queued_dispatch_diverged(Some("  m1  "), "m1"),
        "C26：两侧 trim 后相等 ⇒ 不分歧（令牌侧带空白也不误判）"
    );
    // 同一规则的换行/制表符变体同样视为空白差异，不误判。
    assert!(
        !tui::queued_dispatch_diverged(Some("m1"), "\n m1 \t"),
        "C26：空白类差异（换行/制表符）trim 后相等 ⇒ 不分歧"
    );
}

// ── A27 · L2-T ─ C27：闩过期后的孤儿派发载荷必须回队 ─────────────────────────

#[test]
fn a27_orphan_dispatch_payload_matrix() {
    // C27 · 语义（本用例锁的就是这条语义）：
    //   「闩已过期（latch_active=false）但仍有 QueuedAutoSubmit 在飞」时，若直接把输入框正文丢掉，
    //   下一次派发就会**覆盖**这条未被消费的正文 ⇒ 静默丢消息（trace-B 的变体）。
    //   因此必须先把孤儿 payload 回队（要求非空白、trim 后非空），再允许下一次派发取件。
    //   反过来，闩仍生效（latch_active=true）时该 payload 仍归在飞派发所有，**不得**当孤儿回队，
    //   否则会出现「队列里有一条 + 在飞还有一条 = 重复派发」。
    assert_eq!(
        tui::orphan_dispatch_payload(Some("m1"), false),
        Some("m1".to_string()),
        "C27：闩已过期 ⇒ 孤儿 payload 必须回队（trim 后原文）"
    );
    assert_eq!(
        tui::orphan_dispatch_payload(Some("m1"), true),
        None,
        "C27：闩仍生效 ⇒ payload 归在飞派发所有，不得当孤儿回队（防重复派发）"
    );
    assert_eq!(
        tui::orphan_dispatch_payload(None, false),
        None,
        "C27：无 payload ⇒ 无孤儿可回队"
    );
    assert_eq!(
        tui::orphan_dispatch_payload(Some("   "), false),
        None,
        "C27：纯空白不算正文，不得回队（否则队列里多一条空气泡）"
    );
    assert_eq!(
        tui::orphan_dispatch_payload(Some(" m1 "), false),
        Some("m1".to_string()),
        "C27：两端空白必须被 trim（与 push_queued_message 的正文口径一致）"
    );
    // 与队列原语的联动：孤儿 payload 回队后，队列必须真的多一条且可被出队。
    let mut state = tui::TuiState::default();
    if let Some(orphan) = tui::orphan_dispatch_payload(Some("  孤儿正文  "), false) {
        assert!(
            tui::push_queued_message(&mut state.queued_user_messages, &orphan),
            "C27：回队必须经由入队原语且成功"
        );
    } else {
        panic!("C27：非空白孤儿 payload 必须返回 Some");
    }
    assert_eq!(state.queued_count(), 1, "C27：回队后队列深度必须 +1");
    assert_eq!(
        tui::drain_next_queued(&mut state).as_deref(),
        Some("孤儿正文"),
        "C27：回队内容必须是 trim 后的正文（可直接再派发）"
    );
}

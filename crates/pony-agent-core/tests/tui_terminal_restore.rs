//! 终端还原加固的验收测试（缺陷：退出 TUI 后终端输入变乱码）。
//!
//! 缺陷机理：退出时若未把 kitty 键盘增强协议栈弹掉 / 未关鼠标追踪 / 未退备用屏，
//! 终端会继续把每个按键编码成 `ESC[117u`、`ESC[9;5u` 这类转义序列；而 shell 已回到
//! 普通行模式，于是这些序列被当正文回显 —— 表现为"不管敲什么都变成乱码"。
//!
//! 沿用 `tests/tui_batch2_regression.rs` 的 bin-as-module 模式（`#[path = "../src/bin/tui.rs"]`），
//! 断言的是**真实实现里的还原串**，不是复制粘贴的常量副本。
//! 铁律：每条用例都有真实断言；无 `#[ignore]`/skip；不修改被测实现。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

/// 断言 `haystack` 含 `needle`（字节级子串，避免 `str` 转换与转义歧义）。
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| window == needle)
}

// ── C1：kitty 键盘增强协议栈必须被弹掉（两种写法都发，覆盖实现差异） ──────────────────

#[test]
fn c1_keyboard_enhancement_stack_is_popped_in_both_accepted_forms() {
    let seq = tui::TERMINAL_INPUT_MODES_RESET;
    assert!(
        contains(seq, b"\x1b[<1u"),
        "C1: 必须发 crossterm 形式的弹栈 ESC[<1u（缺了它 kitty/wezterm 会一直把按键编码成 ESC[..u）"
    );
    assert!(
        contains(seq, b"\x1b[<u"),
        "C1: 必须同时发不带计数的弹栈 ESC[<u（部分终端只认这一种写法）"
    );
    assert!(
        !contains(tui::TERMINAL_SCREEN_RESET, b"\x1b[<"),
        "C1: 弹栈属于输入侧还原，不得混进屏幕侧还原串（两段写入顺序是契约的一部分）"
    );
}

// ── C2：各级鼠标追踪必须全部关闭（残留 any-motion 会持续往 shell 灌事件） ──────────────

#[test]
fn c2_every_mouse_tracking_mode_is_disabled() {
    let seq = tui::TERMINAL_INPUT_MODES_RESET;
    for mode in [
        b"\x1b[?1000l".as_slice(), // 按键/点击事件
        b"\x1b[?1002l".as_slice(), // 按住拖动
        b"\x1b[?1003l".as_slice(), // any-motion（不按键也上报）
        b"\x1b[?1006l".as_slice(), // SGR 扩展坐标
        b"\x1b[?1015l".as_slice(), // urxvt 扩展坐标
    ] {
        assert!(
            contains(seq, mode),
            "C2: 还原串缺少鼠标关闭项 {:?}（残留后鼠标移动会持续往 shell 写转义序列）",
            String::from_utf8_lossy(mode)
        );
    }
}

// ── C3：bracketed paste 与 modifyOtherKeys 一并撤销 ──────────────────────────────────

#[test]
fn c3_bracketed_paste_and_modify_other_keys_are_disabled() {
    let seq = tui::TERMINAL_INPUT_MODES_RESET;
    assert!(
        contains(seq, b"\x1b[?2004l"),
        "C3: 必须关闭 bracketed paste（ESC[?2004l），否则 shell 会收到粘贴定界符"
    );
    assert!(
        contains(seq, b"\x1b[>4;0m"),
        "C3: 必须关闭 xterm modifyOtherKeys（ESC[>4;0m），否则按键仍被改写成 CSI 序列"
    );
}

// ── C4：屏幕侧还原必须退出备用屏并重新显示光标 ─────────────────────────────────────

#[test]
fn c4_screen_reset_leaves_alternate_screen_and_shows_cursor() {
    let seq = tui::TERMINAL_SCREEN_RESET;
    assert!(
        contains(seq, b"\x1b[?1049l"),
        "C4: 必须退出备用屏（ESC[?1049l），否则用户回到 shell 后看到的还是 TUI 画面"
    );
    assert!(
        contains(seq, b"\x1b[?25h"),
        "C4: 必须重新显示光标（ESC[?25h），否则 shell 里光标不可见"
    );
    assert!(
        !contains(seq, b"\x1b[?1003l") && !contains(seq, b"\x1b[?2004l"),
        "C4: 鼠标/粘贴的撤销属于输入侧串，不得混进屏幕侧串（保持两阶段写入可分离）"
    );
}

// ── C5：还原入口必须是可直接交给 TerminalGuard 的 `fn()`（不再经共享标志分支） ─────────

#[test]
fn c5_restore_entry_point_is_a_plain_fn_usable_as_the_guard_closure() {
    // 类型断言：`restore_terminal_modes` 必须能直接作为 `impl FnMut() + 'static` 传进守卫
    // （旧实现是捕获共享 AtomicBool 的闭包：push 判定为假时整段还原会被跳过）。
    let restore: fn() = tui::restore_terminal_modes;
    assert!(
        std::mem::size_of_val(&restore) == std::mem::size_of::<usize>(),
        "C5: 还原入口必须是普通函数指针（无捕获 ⇒ 无条件执行，不看 push 是否成功）"
    );
    // 行为断言：函数指针调用不 panic（无 tty 环境下走 stdout 兜底，不得把异常抛给 Drop）。
    let outcome = std::panic::catch_unwind(restore);
    assert!(
        outcome.is_ok(),
        "C5: 还原入口必须永不 panic（它在 Drop / panic hook 里执行，再 panic 会 abort 进程）"
    );
}

/// 返回 `needle` 首次出现的字节偏移（用于断言"置零在弹栈之后"这类顺序契约）。
fn position_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ── C6：kitty 键盘协议必须被「显式置零」，且弹栈要有重复兜底 ───────────────────────

#[test]
fn c6_keyboard_flags_are_explicitly_zeroed_after_repeated_pops() {
    let seq = tui::TERMINAL_INPUT_MODES_RESET;
    let first_pop = position_of(seq, b"\x1b[<1u")
        .expect("C6: 缺少 crossterm 弹栈 ESC[<1u");
    let zero_at = position_of(seq, b"\x1b[=0u").expect(
        "C6: 必须显式置零 kitty 键盘标志（ESC[=0u）——栈深与代理层转发错位时，弹栈是相对操作，永远清不干净",
    );
    assert!(
        first_pop < zero_at,
        "C6: 置零必须排在弹栈之后（弹栈会把栈里保存的旧标志写回终端，顺序反了等于没清）"
    );
    let pop_form = b"\x1b[<1u";
    let pops = seq
        .windows(pop_form.len())
        .filter(|window| *window == pop_form)
        .count();
    assert_eq!(
        pops, 3,
        "C6: 弹栈必须重复三次兜底（复用器可能吞掉或错配一次 pop，只发一次会留下残栈），实际 = {pops}"
    );
}

// ── C7：复用器/代理层下只 push DISAMBIGUATE；直连终端保持全键位上报 ────────────────

#[test]
fn c7_multiplexer_pushes_only_disambiguate_while_direct_terminal_keeps_all_keys() {
    use crossterm::event::KeyboardEnhancementFlags as Flags;

    let direct = tui::keyboard_enhancement_push_flags_for(Some("xterm-256color"), None)
        .expect("C7: 直连终端必须 push 键盘增强标志");
    assert!(
        direct.contains(Flags::DISAMBIGUATE_ESCAPE_CODES),
        "C7: 直连终端必须带 DISAMBIGUATE_ESCAPE_CODES（Shift+Enter 依赖它），实际 = {direct:?}"
    );
    assert!(
        direct.contains(Flags::REPORT_ALL_KEYS_AS_ESCAPE_CODES),
        "C7: 直连终端保持既有全键位上报能力，实际 = {direct:?}"
    );

    let multiplexed = tui::keyboard_enhancement_push_flags_for(Some("xterm-256color"), Some("HERDR"))
        .expect("C7: 复用器下仍要 push（否则 Shift+Enter 退化）");
    assert_eq!(
        multiplexed,
        Flags::DISAMBIGUATE_ESCAPE_CODES,
        "C7: 复用器下只许发 DISAMBIGUATE(1)：REPORT_ALL_KEYS(8) 会把每个键都编码成 CSI 序列，\
         pop 一旦被代理层吞掉，用户退出后敲任何键都吐出 `u`/`3u`/`7u`"
    );
    assert!(
        !multiplexed.contains(Flags::REPORT_ALL_KEYS_AS_ESCAPE_CODES),
        "C7: 复用器下不得带 REPORT_ALL_KEYS_AS_ESCAPE_CODES，实际 = {multiplexed:?}"
    );

    assert_eq!(
        tui::keyboard_enhancement_push_flags_for(Some("dumb"), None),
        None,
        "C7: TERM=dumb 不得 push"
    );
    assert_eq!(
        tui::keyboard_enhancement_push_flags_for(None, None),
        None,
        "C7: TERM 缺失不得 push"
    );
}

// ── C8：复用器环境探测（纯函数内核，不读全局环境） ─────────────────────────────────

#[test]
fn c8_multiplexer_detection_reads_known_env_keys() {
    let tmux_only = |key: &str| (key == "TMUX").then(|| "/tmp/tmux-1000/default,4242,0".to_string());
    assert_eq!(
        tui::terminal_multiplexer_for_env(tmux_only).as_deref(),
        Some("TMUX"),
        "C8: TMUX 环境变量存在时必须识别为复用器"
    );

    let herdr_only = |key: &str| (key == "HERDR").then(|| "0.7.5".to_string());
    assert_eq!(
        tui::terminal_multiplexer_for_env(herdr_only).as_deref(),
        Some("HERDR"),
        "C8: herdr pane 必须被识别（本缺陷的实际现场）"
    );

    // 复用器退出后环境变量仍可能残留为空值：空白值不得判成复用器（否则无谓降级键位）。
    let blank = |_: &str| Some("   ".to_string());
    assert_eq!(
        tui::terminal_multiplexer_for_env(blank),
        None,
        "C8: 空白值不得判定为复用器"
    );

    let absent = |_: &str| None;
    assert_eq!(tui::terminal_multiplexer_for_env(absent), None);
}

// ── C9：还原不完整时，退出提示必须附带可直接执行的手工恢复命令 ─────────────────────

#[test]
fn c9_recovery_hint_is_single_line_and_copy_pastable() {
    let line = tui::terminal_recovery_hint_line();
    assert!(
        !line.contains('\n'),
        "C9: 恢复提示必须单行（多行会被终端还原序列打散），实际 = {line:?}"
    );
    assert!(
        line.contains("reset"),
        "C9: 首选手段必须是 `reset`（同时复位 raw mode 与终端私有模式），实际 = {line:?}"
    );
    assert!(
        line.contains("printf") && line.contains("=0u"),
        "C9: 应急串必须用 POSIX printf 且含显式置零 ESC[=0u，实际 = {line:?}"
    );
    assert!(
        !line.contains("$'"),
        "C9: 不得使用 `$'...'`（bash/zsh 扩展，dash/sh 不认），实际 = {line:?}"
    );
}

#[test]
fn c9_hint_block_appends_recovery_line_only_when_restore_incomplete() {
    let complete = tui::resume_hint_block("session-abcd1234", false);
    assert!(
        complete.contains("pony resume session-abcd1234"),
        "C9: 续接行必须恒在，实际 = {complete:?}"
    );
    assert!(
        !complete.contains("reset"),
        "C9: 还原成功时不得多印恢复命令（会误导用户怀疑终端坏了），实际 = {complete:?}"
    );

    let incomplete = tui::resume_hint_block("session-abcd1234", true);
    let lines: Vec<&str> = incomplete
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "C9: 还原不完整时必须恰好两行（续接 + 恢复），实际 = {lines:?}"
    );
    assert!(
        lines[0].contains("pony resume session-abcd1234"),
        "C9: 续接行必须在首行（用户最先看到可复制的入口），实际 = {:?}",
        lines[0]
    );
    assert!(
        lines[1].contains("reset"),
        "C9: 第二行必须是手工恢复命令，实际 = {:?}",
        lines[1]
    );
}

// ── C10：还原状态可观测且幂等（重复还原不得让标记漂移） ────────────────────────────

#[test]
fn c10_restore_reports_completeness_idempotently() {
    tui::restore_terminal_modes();
    let first = tui::terminal_restore_incomplete();
    tui::restore_terminal_modes();
    let second = tui::terminal_restore_incomplete();
    assert_eq!(
        first, second,
        "C10: 还原必须幂等——重复调用不得让「还原是否完整」标记漂移（退出提示据此决定是否附恢复命令）"
    );
}

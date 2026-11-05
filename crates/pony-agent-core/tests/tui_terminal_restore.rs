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

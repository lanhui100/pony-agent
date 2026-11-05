//! wave-tui-session-resilience：L2-T 功能契约 + L2-AT 确定性破坏注入**双模态红相**验收。
//!
//! 契约真值源：`.dev-team/contract-matrix-tui-session-resilience.md` §2/§3（C1–C8 / S1–S7）
//! 与 `.dev-team/nfr-baseline-tui-session-resilience.json`。
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。测试经既有
//! `#[path = "../src/bin/tui.rs"] mod tui;` 模式仅访问 `pub` 纯函数与 `pub const` 还原串，
//! 不新增生产代码、不吞异常、不放宽断言、不 `#[ignore]`。
//!
//! 红相：S1–S7 当前均为 `unimplemented!()` 空桩，逐用例以 Rust 原生 panic 失败；
//! PTY 动态链路本环境未拉起真实 `pony` 进程，采用静态字节/退出计划状态机断言，
//! 显式标注 `[RED DEGRADED: STATIC-ONLY]`，不谎称动态通过。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use tui::{
    exit_cleanup_plan, interrupted_turn_checkpoint, is_transient_provider_error,
    resume_from_checkpoint, resume_hint_line, sanitize_model_protocol_text,
    should_fold_error_detail, user_facing_error_summary, TERMINAL_SCREEN_RESET,
};

const DSML_BLOCK: &str = "<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n<｜｜DSML｜｜ parameter name=\"command\">cargo test</｜｜DSML｜｜ parameter>\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>";
const DSML_MARKERS: [&str; 3] = ["｜DSML｜｜", "invoke name=", "parameter name="];
const NFR_EXTERNAL_CALL_TIMEOUT_MS: u64 = 3000;
const NFR_RETRY_MAX_ATTEMPTS: u64 = 5;

fn assert_no_dsml(out: &str, ticket: &str) {
    for marker in DSML_MARKERS {
        assert!(
            !out.contains(marker),
            "{ticket}：用户可见文本不得含 DSML 协议片段 {marker:?}，实际 = {out:?}"
        );
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack.windows(needle.len()).any(|window| window == needle)
}

// ── L2-T 功能契约（≥8 个 #[test]，每条均有真实 assert） ───────────────────────

/// C1 / S1-A01：流式完整 DSML 块必须被清洗为用户可读占位，不得泄漏协议原文。
#[test]
fn c1_dsml_stream_is_filtered() {
    let (out, incomplete) = sanitize_model_protocol_text(DSML_BLOCK);
    assert_no_dsml(&out, "C1/S1-A01");
    assert!(
        out.contains("工具调用"),
        "C1/S1-A01：清洗后必须含面向用户的「工具调用」占位，实际 = {out:?}"
    );
    assert!(
        !incomplete,
        "C1/S1-A01：完整 DSML 块不得被标记为 incomplete"
    );
}

/// C1 / S1-A02：普通 markdown 代码块与含尖括号正文逐字保真（禁止误伤）。
#[test]
fn c1b_plain_markdown_and_angle_brackets_are_untouched() {
    let prose = "```rust\nlet x = \"<a>\";\n// <b> 与 <｜不是 DSML\n```\n正文 <tag attr=\"v\"> 保留";
    let (out, incomplete) = sanitize_model_protocol_text(prose);
    assert_eq!(out, prose, "C1/S1-A02：普通文本必须逐字保真");
    assert!(!incomplete, "C1/S1-A02：普通尖括号文本不得触发 incomplete");
    let trailing_angle = "普通正文以尖括号开头但无 DSML 前缀：<";
    let (out, incomplete) = sanitize_model_protocol_text(trailing_angle);
    assert_eq!(out, trailing_angle, "C1/S1-A02：以 '<' 结尾的普通正文逐字保真");
    assert!(!incomplete, "C1/S1-A02：普通正文以 '<' 结尾不得触发 incomplete");
    let (partial, incomplete) = sanitize_model_protocol_text("<｜｜DSML｜｜ inv");
    assert!(
        incomplete,
        "C1/S1-A02：已识别 DSML 前缀的半截形态必须标记 incomplete=true"
    );
    assert_no_dsml(&partial, "C1/S1-A02-partial");
}

/// C2 / S1-A03：历史回放与流式共用同一清洗入口；旧会话无检查点字段仍可逐字回放。
#[test]
fn c2_dsml_history_replay_is_filtered_and_legacy_history_is_preserved() {
    let (out, _) = sanitize_model_protocol_text(DSML_BLOCK);
    assert_no_dsml(&out, "C2/S1-A03");
    let (legacy, incomplete) = sanitize_model_protocol_text("旧会话纯文本，无检查点字段");
    assert_eq!(legacy, "旧会话纯文本，无检查点字段", "C2：旧会话纯文本不得被改写");
    assert!(!incomplete, "C2：旧会话纯文本不得触发 incomplete");
    let no_checkpoint = resume_from_checkpoint(&legacy, None);
    assert_eq!(
        no_checkpoint, legacy,
        "C2/C6：无检查点字段时必须逐字返回 completed_history（向后兼容），实际 = {no_checkpoint:?}"
    );
}

/// C3 / S2-A04：瞬态错误在既有预算内可重试；非瞬态错误不得重试。
#[test]
fn c3_transient_errors_are_retryable_within_budget_and_permanent_errors_are_not() {
    for transient in [
        "upstream timeout after 45014ms",
        "HTTP 429 Too Many Requests",
        "HTTP 503 Service Unavailable",
        "connection refused / dns failure",
    ] {
        assert!(
            is_transient_provider_error(transient),
            "C3/S2-A04：瞬态错误必须判为可重试，实际 false：{transient:?}"
        );
    }
    for permanent in [
        "decision:attempt=1/5 failed class=NonRetryable 401",
        "HTTP 403 Forbidden",
        "HTTP 400 Bad Request",
        "missing API key",
    ] {
        assert!(
            !is_transient_provider_error(permanent),
            "C3/S2-A04：非瞬态错误不得重试，实际 true：{permanent:?}"
        );
    }
    assert!(
        NFR_RETRY_MAX_ATTEMPTS <= 5 && NFR_EXTERNAL_CALL_TIMEOUT_MS <= 3000,
        "C3：注入参数不得超 NFR 预算上限（5 次 / 3000ms）"
    );
}

/// C4 / S3-A05 + S4-A06：错误详情默认折叠；用户可见摘要不得反显响应体/URL/堆栈。
#[test]
fn c4_error_detail_is_folded_and_summary_is_user_facing() {
    let raw = "error sending request for url (http://127.0.0.1:1/v1/chat/completions) body={\"error\":{\"code\":401,\"message\":\"missing API key\"}} stacktrace: at provider::call at runtime::turn";
    let summary = user_facing_error_summary(raw);
    assert!(!summary.is_empty(), "C4/S3-A05：摘要不得为空");
    assert!(
        summary.chars().count() <= 200,
        "C4/S3-A05：摘要必须长度有界，实际 {} 字：{summary:?}",
        summary.chars().count()
    );
    for forbidden in [
        "error sending request for url",
        "http://127.0.0.1:1",
        "{\"error\"",
        "stacktrace",
        "missing API key",
    ] {
        assert!(
            !summary.contains(forbidden),
            "C4/S3-A05：摘要不得含内部详情 {forbidden:?}，实际 = {summary:?}"
        );
    }
    assert!(
        should_fold_error_detail(raw),
        "C4/S4-A06：终态错误详情必须默认折叠"
    );
    assert!(
        !should_fold_error_detail(&summary),
        "C4/S4-A06：面向用户摘要本身不得再被当作可折叠详情"
    );
}

/// C5 / S5-A07：中断检查点保留已落盘步骤；空输入返回 None（旧会话向后兼容）。
#[test]
fn c5_interrupt_checkpoint_keeps_persisted_steps_and_empty_input_is_none() {
    let activities = vec![
        "Read package.json".to_string(),
        "Bash cargo test".to_string(),
    ];
    let checkpoint = interrupted_turn_checkpoint("已完成的分析正文", &activities)
        .expect("C5：非空正文 + 工具活动必须生成 Some(检查点)");
    assert!(
        checkpoint.contains("已完成的分析正文"),
        "C5/S5-A07：检查点必须保留已落盘正文，实际 = {checkpoint:?}"
    );
    assert!(
        checkpoint.contains("Read package.json") && checkpoint.contains("Bash cargo test"),
        "C5/S5-A07：检查点必须保留两条已落盘工具步骤，实际 = {checkpoint:?}"
    );
    assert_eq!(
        interrupted_turn_checkpoint("", &[]),
        None,
        "C5：空输入必须返回 None，旧会话无检查点字段方可加载"
    );
}

/// C6 / S6-A08：resume 从检查点接续且不重复已完成 turn。
#[test]
fn c6_resume_continues_from_checkpoint_without_replaying_completed_turn() {
    let completed = "turn-1 用户问题\nturn-1 助手结论";
    let checkpoint = Some("中断点：已读取配置，尚未输出最终建议");
    let resumed = resume_from_checkpoint(completed, checkpoint);
    assert!(
        resumed.starts_with(completed),
        "C6/S6-A08：返回值必须是拼接后的全文，以已完成历史开头，实际 = {resumed:?}"
    );
    assert!(
        resumed.contains("中断点：已读取配置"),
        "C6/S6-A08：resume 必须在全文末尾从检查点接续，实际 = {resumed:?}"
    );
    assert_eq!(
        resumed.matches("turn-1 用户问题").count(),
        1,
        "C6/S6-A08：已完成 turn 在拼接全文中只能出现一次，实际 = {resumed:?}"
    );
    assert_eq!(
        resume_from_checkpoint(completed, None),
        completed,
        "C6：无检查点时必须逐字返回 completed_history，不得伪造续传增量"
    );
}

/// C7 / S7-A09：退出路径先 cancel 在飞 turn，再等待/还原/提示/退出；超时钳到 NFR 上限。
#[test]
fn c7_exit_cancels_inflight_turn_restores_terminal_then_prints_resume_hint() {
    let plan = exit_cleanup_plan(true, 9_000);
    assert_eq!(
        plan,
        vec!["cancel", "wait", "restore", "hint", "exit"],
        "C7/S7-A09：在飞 turn 退出计划必须严格为 cancel→wait→restore→hint→exit，实际 = {plan:?}"
    );
    assert_eq!(
        plan[0], "cancel",
        "C7：取消在飞 turn 必须是首步"
    );
    let idle = exit_cleanup_plan(false, 50);
    assert!(
        !idle.iter().any(|step| *step == "cancel"),
        "C7：无在飞 turn 时不得伪造 cancel 步骤，实际 = {idle:?}"
    );
    assert!(
        idle.windows(2).any(|pair| pair == ["restore", "hint"]),
        "C7：无在飞 turn 时仍必须保证还原后才打印提示，实际 = {idle:?}"
    );
    assert!(
        NFR_EXTERNAL_CALL_TIMEOUT_MS == 3000 && 9_000 > NFR_EXTERNAL_CALL_TIMEOUT_MS,
        "C7：注入 timeout 必须超过 NFR 上限以验证钳制输入，且计划仍受 3000ms 约束"
    );
}

/// C8 / S7-A10 [RED DEGRADED: STATIC-ONLY]：真实 PTY 未在本环境拉起，
/// 用生产还原字节 + resume 提示文案 + 退出计划状态机做静态断言，不谎称动态通过。
#[test]
fn c8_terminal_restore_bytes_and_resume_hint_order_are_statically_enforced() {
    assert!(
        contains(TERMINAL_SCREEN_RESET, b"\x1b[?1049l"),
        "[RED DEGRADED: STATIC-ONLY] C8：屏幕字节必须含 LeaveAlternateScreen（ESC[?1049l）"
    );
    assert!(
        contains(TERMINAL_SCREEN_RESET, b"\x1b[?25h"),
        "[RED DEGRADED: STATIC-ONLY] C8：屏幕字节必须含 ShowCursor（ESC[?25h）"
    );
    let hint = resume_hint_line("session-83e412b0");
    assert!(
        hint.contains("pony resume session-83e412b0") && !hint.contains('\n'),
        "[RED DEGRADED: STATIC-ONLY] C8：退出提示必须是单行且可复制执行，实际 = {hint:?}"
    );
    let plan = exit_cleanup_plan(true, NFR_EXTERNAL_CALL_TIMEOUT_MS);
    let restore = plan.iter().position(|step| *step == "restore")
        .expect("[RED DEGRADED: STATIC-ONLY] C8：计划必须含 restore");
    let hint_at = plan.iter().position(|step| *step == "hint")
        .expect("[RED DEGRADED: STATIC-ONLY] C8：计划必须含 hint");
    let exit_at = plan.iter().position(|step| *step == "exit")
        .expect("[RED DEGRADED: STATIC-ONLY] C8：计划必须含 exit");
    assert!(
        restore < hint_at && hint_at < exit_at,
        "[RED DEGRADED: STATIC-ONLY] C8：提示必须在终端还原之后、进程退出之前，实际 = {plan:?}"
    );
    assert!(
        !plan.iter().any(|step| *step == "detach"),
        "[RED DEGRADED: STATIC-ONLY] C8：禁止 detach 后台线程继续持有 tty，实际 = {plan:?}"
    );
}

// ── L2-AT 确定性破坏注入（固定迭代 + Barrier；无 sleep 概率等待） ─────────────

/// AT1 / S1-A11：畸形、超大、分片 DSML payload。分片按固定 byte 边界（含 UTF-8 多字节
/// 标记）逐片调用 S1，断言 never panic-泄漏半截标记；payload 长度 ≤ NFR 3000 上限。
#[test]
fn at1_malformed_oversized_and_chunk_split_dsml_never_leaks_partial_markers() {
    let oversized = format!("{DSML_BLOCK}{}", "X".repeat(2048));
    assert!(
        oversized.len() <= NFR_EXTERNAL_CALL_TIMEOUT_MS as usize,
        "AT1：注入 payload 不得超 NFR 声明上限 3000，实际 {} 字节",
        oversized.len()
    );
    let malformed = "<｜｜DSML｜｜ calls><｜｜DSML｜｜ invoke name=\"Run\"><｜｜DSML｜｜ parameter name=\"x\">unclosed";
    for (index, payload) in [DSML_BLOCK, oversized.as_str(), malformed].iter().enumerate() {
        let (out, _incomplete) = sanitize_model_protocol_text(payload);
        assert_no_dsml(&out, &format!("AT1#{index}"));
        assert!(
            !out.contains("<｜") && !out.contains("｜>"),
            "AT1#{index}：不得输出半截 DSML 标记，实际 = {out:?}"
        );
    }
    // 固定分片迭代：按 char_indices() 的合法 UTF-8 边界重建前缀（全角竖线是多字节字符，
    // 禁止按裸 byte offset 切片，否则 harness 会先于 SUT 触发 char boundary panic）。
    // 无 sleep；每次都调用同一个可编译的纯函数 S1。
    let mut legal_prefixes = 0usize;
    for (cut, _) in DSML_BLOCK.char_indices().skip(1).take(64) {
        legal_prefixes += 1;
        let chunk = &DSML_BLOCK[..cut];
        let (out, incomplete) = sanitize_model_protocol_text(chunk);
        assert_no_dsml(&out, &format!("AT1-split@{cut}"));
        if incomplete {
            assert!(
                !out.contains("<｜") && !out.contains("｜DSML"),
                "AT1-split@{cut}：incomplete 时不得把半截标记交给用户，实际 = {out:?}"
            );
        }
    }
    assert!(
        legal_prefixes > 0,
        "AT1：必须遍历至少一个合法 UTF-8 边界前缀（harness 自检，防止空断言）"
    );
}

/// AT2 / S5+S6-A12：中断与恢复交错。固定 8 轮、Barrier 同步两线程，
/// 每轮都把「已完成 turn → checkpoint → resume」串起来，禁止 sleep 概率等待。
#[test]
fn at2_interrupt_and_resume_interleaving_is_deterministic_and_idempotent() {
    const ITERATIONS: usize = 8;
    // mpsc 通道发布 writer 本轮真实检查点 + Barrier 固定两阶段；
    // reader 只消费同一值，无锁、无 sleep，恢复输入绝不与生成逻辑漂移。
    let start = Arc::new(Barrier::new(2));
    let consumed = Arc::new(Barrier::new(2));
    let (tx, rx) = mpsc::channel::<String>();
    let writer = {
        let start = Arc::clone(&start);
        let consumed = Arc::clone(&consumed);
        thread::spawn(move || {
            let mut checkpoints = Vec::with_capacity(ITERATIONS);
            for iteration in 0..ITERATIONS {
                let text = format!("turn-{iteration} 已落盘正文");
                let tools = vec![format!("Read file-{iteration}")];
                let checkpoint = interrupted_turn_checkpoint(&text, &tools)
                    .unwrap_or_else(|| panic!("AT2：第 {iteration} 轮必须生成检查点"));
                checkpoints.push(checkpoint.clone());
                tx.send(checkpoint).expect("AT2：检查点通道必须送达 reader");
                start.wait();
                consumed.wait();
            }
            checkpoints
        })
    };
    let reader = {
        let start = Arc::clone(&start);
        let consumed = Arc::clone(&consumed);
        thread::spawn(move || {
            let mut observed = Vec::with_capacity(ITERATIONS);
            for iteration in 0..ITERATIONS {
                start.wait();
                let checkpoint = rx.recv().expect("AT2：必须收到 writer 本轮真实检查点");
                let completed = format!("turn-{iteration} 用户问题\nturn-{iteration} 已完成结论");
                let resumed = resume_from_checkpoint(&completed, Some(&checkpoint));
                assert!(
                    resumed.starts_with(&completed),
                    "AT2：第 {iteration} 轮 resume 必须保留已完成历史全文，实际 = {resumed:?}"
                );
                assert!(
                    resumed.contains(&format!("turn-{iteration} 已落盘正文")),
                    "AT2：第 {iteration} 轮 resume 必须接续 writer 的真实检查点，实际 = {resumed:?}"
                );
                assert_eq!(
                    resumed.matches(&format!("turn-{iteration} 已完成结论")).count(),
                    1,
                    "AT2：第 {iteration} 轮已完成 turn 在拼接全文中只能出现一次，实际 = {resumed:?}"
                );
                observed.push(resumed);
                consumed.wait();
            }
            observed
        })
    };
    let checkpoints = writer.join().expect("AT2 writer 线程不得 panic（除被测原生失败外）");
    let resumed = reader.join().expect("AT2 reader 线程不得 panic（除被测原生失败外）");
    assert_eq!(checkpoints.len(), ITERATIONS, "AT2：检查点数必须固定为 8");
    assert_eq!(resumed.len(), ITERATIONS, "AT2：恢复数必须固定为 8");
    for (index, checkpoint) in checkpoints.iter().enumerate() {
        assert!(
            checkpoint.contains(&format!("turn-{index} 已落盘正文")),
            "AT2：第 {index} 轮检查点必须保留已落盘步骤，实际 = {checkpoint:?}"
        );
        assert!(
            resumed[index].contains(&format!("turn-{index} 已落盘正文")),
            "AT2：第 {index} 轮恢复文本必须与检查点同源，实际 = {:?}",
            resumed[index]
        );
    }
}

/// AT3 / S7-A13：并发退出信号与在飞 turn 竞态。固定 4 轮 × 2 线程 + Barrier；
/// timeout 注入 3000/9000 均不得超 NFR 钳制语义，异常映射为 Rust 原生 join Result。
#[test]
fn at3_concurrent_exit_signals_and_inflight_turn_race_have_one_bounded_plan() {
    const ROUNDS: usize = 4;
    let barrier = Arc::new(Barrier::new(2));
    let signaler = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut plans = Vec::with_capacity(ROUNDS);
            for round in 0..ROUNDS {
                barrier.wait();
                let timeout = if round % 2 == 0 { 3000 } else { 9000 };
                plans.push(exit_cleanup_plan(true, timeout));
                barrier.wait();
            }
            plans
        })
    };
    let inflight = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut plans = Vec::with_capacity(ROUNDS);
            for _ in 0..ROUNDS {
                barrier.wait();
                barrier.wait();
                plans.push(exit_cleanup_plan(true, NFR_EXTERNAL_CALL_TIMEOUT_MS));
            }
            plans
        })
    };
    let signaled = signaler.join().expect("AT3：退出信号线程不得 panic（除被测原生失败外）");
    let raced = inflight.join().expect("AT3：在飞 turn 线程不得 panic（除被测原生失败外）");
    assert_eq!(signaled.len(), ROUNDS, "AT3：退出计划数必须固定为 4");
    for (round, plan) in signaled.iter().chain(raced.iter()).enumerate() {
        assert_eq!(
            plan.first().copied(),
            Some("cancel"),
            "AT3#{round}：并发竞态下首步必须始终是 cancel，实际 = {plan:?}"
        );
        assert_eq!(
            plan,
            &vec!["cancel", "wait", "restore", "hint", "exit"],
            "AT3#{round}：退出计划必须唯一且有界（≤3000ms），实际 = {plan:?}"
        );
    }
    assert!(
        NFR_EXTERNAL_CALL_TIMEOUT_MS <= 3000,
        "AT3：注入参数不得超 NFR baseline 上限"
    );
}

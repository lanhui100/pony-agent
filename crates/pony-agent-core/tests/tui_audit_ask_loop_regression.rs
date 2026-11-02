//! 批8（task-19）验收：审计用 Ask 引导 **fail-closed** / 真实 CAS 闭环 / 场景接线。
//!
//! ## 三条
//! - **P1 fail-closed**：`tui::audit_ask_bootstrap_enabled()` 在**没有** `PONY_AUDIT_ASK_BOOTSTRAP`
//!   时必须返回 false；设置后（debug 测试构建）才 true。并做**静态**断言：该引导逻辑必须位于
//!   `#[cfg(debug_assertions)]` 守卫内 —— 否则 release 包也可能被环境变量打开，等于把
//!   「审计引导」变成生产后门。
//! - **P2 真实 CAS 闭环（核心）**：不依赖终端，直接驱动**真实** governed dispatcher：
//!   `dispatch_governed`（host-mediated `builtin:ask_user`）⇒ `WaitingHost` ⇒
//!   `list_pending_asks` 恰 1 条且 `kind == Interaction` ⇒ `ControlRequestAuthorization::for_request`
//!   ⇒ `answer_ask` 成功 ⇒ 再列必须为空 ⇒ **重复 answer 同一 request 必须 Err**（CAS 防重放）。
//! - **P3 接线静态证据**：`run_audit.sh` 场景清单 + `case` 臂 + `scn_ask_loop()`；
//!   `checks.py` 的 `def check_ask_loop(` 与 `"ask-loop": check_ask_loop` 注册；
//!   且 `ask-panel` 断言函数体内**不再**出现 `documented-gap`（缺口已被真断言替代）。
//!
//! ## 关于进程级 env（P1）
//! `PONY_AUDIT_ASK_BOOTSTRAP` 是**进程级**环境变量，而 cargo 会在同一测试二进制里并行跑用例，
//! 因此本文件用一把 `static Mutex` 把「设置 → 断言 → 复原」整段夹紧（serialize），并在结束前
//! `remove_var` 复原，避免污染同进程其它用例。其余用例（P2/P3）完全不读该变量。
//!
//! 铁律：真实断言、无 `#[ignore]`/skip；对 `tools/**` 只读；临时目录隔离。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use pony_agent_core::agent::ask_control::{answer_ask, list_pending_asks};
use pony_agent_core::agent::dispatcher::{
    ControlRequestAuthorization, DispatchContext, GovernedDispatcher,
};
use pony_agent_core::agent::governed_executor::LegacyCompatiblePolicyEvaluator;
use pony_agent_core::agent::tool_runtime::{
    FakeClock, InvocationOrigin, PendingControlRequestKind, PendingControlRequestState, RuntimeClock,
    ToolDispatchRequest,
};
use pony_agent_core::agent::tools::{ToolControlKind, ToolRegistrySnapshot};
use serde_json::json;

/// 环境变量名（与实现同源的契约常量）。
const BOOTSTRAP_ENV: &str = "PONY_AUDIT_ASK_BOOTSTRAP";

/// 串行化进程级 env 改动（cargo 默认并行跑用例）。
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tui_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/bin/tui.rs"
    ))
    .expect("必须能读取 tui.rs")
}

// ── P1：审计引导 fail-closed + debug 守卫 ────────────────────────────────────────

#[test]
fn p1a_audit_ask_bootstrap_is_closed_unless_env_is_explicitly_set() {
    let _guard = env_lock();
    // 前置：先把变量清干净（上一条用例可能遗留）。
    std::env::remove_var(BOOTSTRAP_ENV);
    assert!(
        !tui::audit_ask_bootstrap_enabled(),
        "P1a: 未设置 {BOOTSTRAP_ENV} 时必须 fail-closed 返回 false（默认关闭）"
    );

    std::env::set_var(BOOTSTRAP_ENV, "1");
    let enabled = tui::audit_ask_bootstrap_enabled();
    // 无论结果如何都先复原，避免 panic 时把环境留给同进程其它用例。
    std::env::remove_var(BOOTSTRAP_ENV);

    assert!(
        enabled,
        "P1a: 显式设置 {BOOTSTRAP_ENV} 后（debug 测试构建）必须启用审计引导"
    );
    // 复原后再断言一次：撤销 env 必须重新关闭（fail-closed 可重复验证）。
    assert!(
        !tui::audit_ask_bootstrap_enabled(),
        "P1a: 移除 {BOOTSTRAP_ENV} 后必须重新回到 false"
    );
}

/// 断言某个 `fn ...` 定义前**紧邻** `#[cfg(debug_assertions)]` 守卫。
///
/// 关键细节（Lead 裁决）：切片起点必须对齐到 `fn` 所在**行的行首**，否则 `pub fn` 会把
/// `pub` 切出来当残留行；白名单同时显式允许签名修饰片段（`pub` / `pub(crate)` / …），
/// 两者叠加保证「任何 `pub` 函数都不会被误判」——这是测试侧缺陷修正，不是放宽语义。
fn assert_debug_gated(source: &str, fn_marker: &str) -> usize {
    let mut search_from = 0usize;
    let mut found = 0usize;
    while let Some(relative) = source[search_from..].find(fn_marker) {
        let fn_index = search_from + relative;
        // ① 对齐到 `fn` 所在行的行首（`pub`/`async` 等修饰与 `fn` 同行时不被残留）。
        let line_start = source[..fn_index]
            .rfind('\n')
            .map(|offset| offset + 1)
            .unwrap_or(0);
        // ② 再按 char 边界回溯窗口（源码含 CJK）。
        let mut window_start = line_start.saturating_sub(1_200);
        while window_start < line_start && !source.is_char_boundary(window_start) {
            window_start += 1;
        }
        let head = &source[window_start..line_start];

        let cfg_marker = "#[cfg(debug_assertions)]";
        // 只认**真正的属性行**（行首 trim 后以属性开头），排除文档注释里对属性的引用
        // （例如「整条路径都在 `#[cfg(debug_assertions)]` 下」——那是说明文字，不是守卫；
        // 若不排除，删掉真守卫后本断言会假通过）。
        let real_attribute_positions: Vec<usize> = head
            .match_indices(cfg_marker)
            .filter(|(position, _)| {
                let line_start = head[..*position]
                    .rfind('\n')
                    .map(|offset| offset + 1)
                    .unwrap_or(0);
                head[line_start..*position].trim().is_empty()
            })
            .map(|(position, _)| position)
            .collect();
        let cfg_index = real_attribute_positions.last().copied().unwrap_or_else(|| {
            panic!(
                "P1b: `{fn_marker}` 之前必须紧邻 `{cfg_marker}` 守卫（否则 release 可被 env 打开）\n\
函数前 200 字符：{:?}",
                head.chars()
                    .skip(head.chars().count().saturating_sub(200))
                    .collect::<String>()
            )
        });

        // ③ 守卫与函数之间只允许空行/注释/其它属性/签名修饰（不得夹入新的 item）。
        for line in head[cfg_index..].lines() {
            let trimmed = line.trim();
            let signature_modifier = matches!(
                trimmed,
                "pub" | "pub(crate)" | "pub(super)" | "unsafe" | "async" | "const"
            );
            let allowed = trimmed.is_empty()
                || trimmed.starts_with("//")
                || trimmed.starts_with("#[")
                || signature_modifier
                || trimmed.starts_with("pub ")
                || trimmed.starts_with("pub(");
            assert!(
                allowed,
                "P1b: `{cfg_marker}` 与 `{fn_marker}` 之间夹入了其它代码行：{trimmed:?}（守卫必须直接作用在该函数上）"
            );
        }

        found += 1;
        search_from = fn_index + fn_marker.len();
    }
    found
}

#[test]
fn p1b_bootstrap_logic_lives_under_debug_assertions_guard() {
    let source = tui_source();

    // ① 整条引导路径（enabled / arguments / dispatch）都必须被 `#[cfg(debug_assertions)]` 守卫：
    //    release 里这些函数「不存在」，环境变量无法开后门。
    for marker in [
        "fn audit_ask_bootstrap_enabled",
        "fn audit_ask_bootstrap_arguments",
        "fn audit_ask_bootstrap(",
    ] {
        let count = assert_debug_gated(&source, marker);
        assert!(
            count >= 1,
            "P1b: tui.rs 必须定义 `{marker}`（审计引导路径），且位于 `#[cfg(debug_assertions)]` 下"
        );
    }

    // ② 开关函数体：必须真的读环境变量，且不得是恒真桩（如直接 `true`）。
    let fn_index = source
        .find("fn audit_ask_bootstrap_enabled")
        .expect("P1b: 必须定义 fn audit_ask_bootstrap_enabled");
    let body_start = fn_index;
    let body_end = source[body_start..]
        .find("\n}\n")
        .map(|offset| body_start + offset)
        .unwrap_or(source.len());
    let body = &source[body_start..body_end];
    assert!(
        body.contains(BOOTSTRAP_ENV),
        "P1b: 开关函数体必须读取 {BOOTSTRAP_ENV}，实际函数体 = {body:?}"
    );
    assert!(
        body.contains("env::var") || body.contains("env::var_os"),
        "P1b: 开关函数体必须真的读环境变量（env::var/var_os），不得是恒真桩，实际函数体 = {body:?}"
    );
    let normalized: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        !normalized.contains("{ true }") && !normalized.ends_with("true }"),
        "P1b: 开关函数体不得退化为无条件返回 true 的桩（那样 release/未注入 env 也会开启），实际 = {normalized}"
    );

    // ③ 说明：release 侧物理验证（`cargo check --release --bin pony`）由 exec-rust 侧执行，
    //    本条只做源代码级守卫断言（不自称验证了 release 行为，避免假断言）。
}

// ── P2：真实 governed dispatcher 上的 Ask CAS 闭环（核心） ────────────────────────

/// 与 `tests/ask_pending_contract.rs` 同源的集成级 harness（真实注册表 + 真实 dispatcher + 假时钟）。
fn harness() -> Arc<GovernedDispatcher> {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin tool registry must construct cleanly"),
    );
    let clock = Arc::new(FakeClock::new(1_000));
    let dispatcher =
        GovernedDispatcher::new(registry, Arc::clone(&clock) as Arc<dyn RuntimeClock>);
    // `builtin:ask_user` 是 host-mediated 描述符：legacy-compatible 评估器把 WaitingHost
    // 变成 Interaction kind 的待决控制请求（与生产 runtime 同一条接线）。
    dispatcher.register_policy_evaluator(Arc::new(LegacyCompatiblePolicyEvaluator));
    Arc::new(dispatcher)
}

fn dispatch_context() -> DispatchContext {
    DispatchContext {
        session_id: Some("ask-loop-session".to_string()),
        run_id: Some("ask-loop-run".to_string()),
        turn_id: Some("ask-loop-turn".to_string()),
        host_control_available: true,
        ..Default::default()
    }
}

#[test]
fn p2_ask_round_trip_is_cas_closed_and_rejects_replay() {
    let dispatcher = harness();

    // ① 真实 dispatch：host-mediated ask ⇒ WaitingHost 控制结果。
    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: "builtin:ask_user".to_string(),
            call_id: "ask-loop-call-1".to_string(),
            arguments: json!({
                "question": "请选择本次交付的执行架构",
                "options": ["方案 A", "方案 B"],
                "description": "批8 ask-loop 验收"
            }),
        },
        &dispatch_context(),
    );
    // 先取 Debug 字符串再移动字段（避免 partially moved 借用错误）。
    let outcome_debug = format!("{outcome:?}");
    let control = outcome
        .control_outcome
        .unwrap_or_else(|| panic!("P2: host-mediated ask 必须产生控制结果，实际 outcome = {outcome_debug}"));
    assert_eq!(
        control.kind,
        ToolControlKind::WaitingHost,
        "P2: ask 必须停在 WaitingHost（等宿主回答）"
    );

    // ② 待决列表恰 1 条，且类型是 Interaction（不是 Approval）。
    let pending_list = list_pending_asks(&dispatcher);
    assert_eq!(
        pending_list.len(),
        1,
        "P2: 派发后待决 ask 必须恰 1 条，实际 {} 条",
        pending_list.len()
    );
    let pending = pending_list.into_iter().next().expect("one pending ask");
    assert_eq!(
        pending.request_kind,
        PendingControlRequestKind::Interaction,
        "P2: ask 的请求类型必须是 Interaction"
    );
    assert_eq!(
        pending.request_id, control.request_id,
        "P2: 待决条目的 request_id 必须与 dispatch 产生的控制结果一致"
    );

    // ③ 用请求自身派生的授权回答：成功消费（CAS: Pending ⇒ Consumed）。
    let authorization = ControlRequestAuthorization::for_request(&pending, Some(json!("continue")));
    let consumed = answer_ask(&dispatcher, &pending.request_id, &authorization)
        .expect("P2: 首次回答必须成功消费待决 ask");
    assert_eq!(
        consumed.request.state,
        PendingControlRequestState::Consumed,
        "P2: 回答后状态必须迁移到 Consumed"
    );
    assert_eq!(
        consumed.answer,
        Some(json!("continue")),
        "P2: 回答内容必须被原样记录"
    );

    // ④ 消费后待决列表必须为空（不得把终态请求重新列为待答）。
    let after = list_pending_asks(&dispatcher);
    assert!(
        after.is_empty(),
        "P2: 回答后待决列表必须为空，实际仍有 {} 条：{:?}",
        after.len(),
        after.iter().map(|item| item.request_id.clone()).collect::<Vec<_>>()
    );

    // ⑤ CAS 防重放：同一个 request 再回答一次必须 Err（不得二次消费/覆盖答案）。
    let replay = answer_ask(&dispatcher, &pending.request_id, &authorization);
    let replay_error = replay.expect_err("P2: 重复回答同一 request 必须被 CAS 拒绝");
    assert!(
        !replay_error.trim().is_empty(),
        "P2: 重放拒绝必须带可诊断原因（当前为空串）"
    );
}

// ── P3：场景接线静态证据 ────────────────────────────────────────────────────────

fn repo_file(relative: &str) -> String {
    let path = format!("{}/../../{}", env!("CARGO_MANIFEST_DIR"), relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("必须能读取 {relative}：{error}"))
}

#[test]
fn p3_ask_loop_scenario_is_wired_and_gap_is_closed() {
    let runner = repo_file("tools/tui-audit/run_audit.sh");
    let checks = repo_file("tools/tui-audit/checks.py");

    // ① runner：场景清单 + case 分派臂 + 场景函数。
    assert!(
        runner.contains("ask-loop"),
        "P3: run_audit.sh 的场景清单必须包含 ask-loop"
    );
    assert!(
        runner.contains("ask-loop) scn_ask_loop"),
        "P3: run_audit.sh 必须有 `ask-loop) scn_ask_loop` 分派臂"
    );
    assert!(
        runner.contains("scn_ask_loop()"),
        "P3: run_audit.sh 必须定义 scn_ask_loop() 场景函数"
    );

    // ② checks.py：断言函数 + SCENARIOS 注册。
    assert!(
        checks.contains("def check_ask_loop("),
        "P3: checks.py 必须定义 def check_ask_loop(（不得只加场景名）"
    );
    assert!(
        checks.contains("\"ask-loop\": check_ask_loop"),
        "P3: checks.py 的 SCENARIOS 必须注册 `\"ask-loop\": check_ask_loop`"
    );

    // ③ ask-panel 的 documented-gap 已被真断言替代：该断言函数体内不得再出现 documented-gap。
    let panel_marker = "def check_ask_panel(";
    let panel_start = checks
        .find(panel_marker)
        .unwrap_or_else(|| panic!("P3: checks.py 必须仍保留 {panel_marker}"));
    let rest = &checks[panel_start..];
    let panel_end = rest[1..]
        .find("\ndef ")
        .map(|offset| offset + 1)
        .unwrap_or(rest.len());
    let panel_body = &rest[..panel_end];
    assert!(
        !panel_body.contains("documented-gap"),
        "P3: ask-panel 断言函数体内不得再出现 documented-gap（缺口必须由真断言替代），\
实际函数体片段 = {:?}",
        &panel_body.chars().take(400).collect::<String>()
    );
}

// ── P4：`ask_question_from_pending` 必须接受生产 schema 的**字符串选项** ─────────────
//
// 生产缺陷（exec-rust 同批修复）：`ask_user.options` 的 input_schema 是 **string 数组**，
// 而历史实现只认 `{label, description}` 对象 ⇒ 真实模型给的选项被整批丢弃、面板从来没显示过
// 模型选项（只剩「自定义输入」）。这条锁死两种形状都接受。

#[test]
fn p4_string_options_are_not_dropped() {
    let question = tui::ask_question_from_pending(&json!({
        "prompt": "请选择本次审计的 Ask 闭环分支",
        "options": ["A", "B"]
    }));

    assert_eq!(
        question.options.len(),
        2,
        "P4: 字符串数组形式的 options 必须被全部保留（历史缺陷会丢成 0 条），实际 = {:?}",
        question.options
    );
    let labels: Vec<&str> = question
        .options
        .iter()
        .map(|option| option.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["A", "B"],
        "P4: 字符串选项必须原样作为 label，实际 = {labels:?}"
    );
    assert!(
        question.options.iter().all(|option| option.description.is_none()),
        "P4: 字符串选项没有说明字段，description 必须为 None（不得伪造说明）"
    );
    assert!(
        question.question.contains("Ask 闭环分支"),
        "P4: prompt 必须映射为 question 文案，实际 = {:?}",
        question.question
    );

    // 对象形状仍未受影响（向后兼容）：`{label, description}` 必须同时被接受。
    let legacy = tui::ask_question_from_pending(&json!({
        "prompt": "旧形状",
        "options": [{"label": "X", "description": "说明 X"}]
    }));
    assert_eq!(
        legacy.options.len(),
        1,
        "P4: 对象形状的 options 必须继续被接受（向后兼容）"
    );
    assert_eq!(legacy.options[0].label, "X");
    assert_eq!(
        legacy.options[0].description.as_deref(),
        Some("说明 X"),
        "P4: 对象形状的 description 必须保留"
    );
}

//! 批5（task-13）验收测试：网关可见性脱敏 / Ask 提交判定 / 多问推进 / `/plan` 诚实文案。
//!
//! 契约真值源：`.dev-team/contract-matrix-slash-commands.md` 修订10（a）(b）(c)（Lead 批准·批5）。
//! 沿用 bin-as-module 模式（`#[path = "../src/bin/tui.rs"]`）：`src/bin/tui.rs` 是 `[[bin]]` 目标，
//! 只能引入**同一份真实源码**才能在编译期断言其行为；`tui.rs` 自身声明 `pub mod protocol;`，
//! 故 protocol 里的类型（`AskUserState` 等）经 `tui::` 路径即可见。
//!
//! 用例映射：
//! - **N20**（F-L）`gateway_display`：`/cost` 移除后 gateway 端点仍可见，但**必须剥离凭据**
//!   （userinfo / query 中的 api key / token），超长按 48 字符 ellipsize。
//! - **N21**（F-M①）`ask_submit_verdict`：无 options 的 ask 不再伪造「确认/取消」，改为「输入即提交」；
//!   有 options 但未选中且无输入 ⇒ 拒绝并给出理由。
//! - **N22**（F-M②）`next_question_index`：多问逐个推进，答完最后一个才 Finish；越界不得 panic。
//! - **N23**（F-N）`plan_mode_banner`：plan 模式不接线后端只读语义，界面必须**诚实**说明
//!   「不限制工具执行」，防止悄悄回退成只读暗示。
//! - **N24**（F-M②多问推进的可观测面）待 exec-rust 落地 `AskUserState` 的答案集合/推进入口后补齐
//!   （见任务回报；不写猜测式断言，也不写无断言用例）。
//!
//! 铁律：真实断言、无 `#[ignore]`/skip；不修改实现代码；不触碰真实 `~/.pony-agent`。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

// ── N20：网关端点可见 + 凭据脱敏（F-L） ────────────────────────────────────────────

#[test]
fn n20a_gateway_display_strips_userinfo_and_query_credentials() {
    let out = tui::gateway_display("https://user:pass@api.example.com/v1?key=sk-1");

    assert!(
        !out.contains("sk-1"),
        "N20a: 输出绝不得包含 query 里的 api key（sk-1），实际 = {out:?}"
    );
    assert!(
        !out.contains("user"),
        "N20a: 输出绝不得包含 userinfo 的用户名，实际 = {out:?}"
    );
    assert!(
        !out.contains("pass"),
        "N20a: 输出绝不得包含 userinfo 的密码，实际 = {out:?}"
    );
    assert!(
        out.contains("api.example.com"),
        "N20a: 端点主机必须可见（/cost 移除后 gateway 的唯一出口），实际 = {out:?}"
    );
    // scheme 与 path 属非敏感信息，保留有助于诊断。
    assert!(out.contains("https"), "N20a: 必须保留 scheme，实际 = {out:?}");
}

#[test]
fn n20b_gateway_display_keeps_port_and_handles_degenerate_inputs() {
    // 端口必须保留（否则多网关/本地调试时无法分辨端点）。
    let with_port = tui::gateway_display("https://h:8080/x");
    assert!(
        with_port.contains(":8080"),
        "N20b: 必须保留端口 :8080，实际 = {with_port:?}"
    );
    assert!(with_port.contains('h'), "N20b: 必须保留主机名，实际 = {with_port:?}");

    // 空串：不得 panic，且不得凭空渲染出凭据分隔符。
    let empty = tui::gateway_display("");
    assert!(
        !empty.contains('@'),
        "N20b: 空输入输出不得包含 userinfo 分隔符，实际 = {empty:?}"
    );

    // 无 scheme 输入：不得 panic，且必须保留 host（用户仍能看到自己配了什么）。
    let no_scheme = tui::gateway_display("api.example.com");
    assert!(
        no_scheme.contains("api.example.com"),
        "N20b: 无 scheme 输入必须保留 host，实际 = {no_scheme:?}"
    );
    assert!(
        !no_scheme.contains('@') && !no_scheme.contains("key="),
        "N20b: 无 scheme 输入输出不得含凭据片段，实际 = {no_scheme:?}"
    );
}

#[test]
fn n20c_gateway_display_ellipsizes_over_48_chars() {
    let long_path = "a".repeat(120);
    let url = format!("https://api.example.com/v1/{long_path}");
    assert!(
        url.chars().count() > 48,
        "N20c 前置条件：构造的 URL 必须超过 48 字符，实际 {}",
        url.chars().count()
    );

    let out = tui::gateway_display(&url);
    assert!(
        out.contains('…'),
        "N20c: 超长端点必须 ellipsize（含 …），实际 = {out:?}"
    );
    assert!(
        out.chars().count() <= 48,
        "N20c: ellipsize 后总长必须 ≤48 字符（含省略号），实际 {} 字符 = {out:?}",
        out.chars().count()
    );
    // 截断后仍应保留 host（否则用户连端点是谁都看不出来）。
    assert!(
        out.contains("api.example.com"),
        "N20c: 截断后必须仍保留 host，实际 = {out:?}"
    );

    // 短端点不得被省略号污染。
    let short = tui::gateway_display("https://api.example.com/v1");
    assert!(
        !short.contains('…'),
        "N20c: ≤48 字符的正常端点不得被截断，实际 = {short:?}"
    );
}

// ── N21：Ask 提交判定（F-M①） ────────────────────────────────────────────────────

#[test]
fn n21a_options_less_ask_rejects_blank_input_with_reason() {
    let verdict = tui::ask_submit_verdict(false, None, "   ");

    // 实现形状（exec-rust 已确认）：`AskSubmitVerdict::Reject(&'static str)` 为**元组变体**。
    match verdict {
        tui::AskSubmitVerdict::Reject(reason) => {
            assert!(
                !reason.trim().is_empty(),
                "N21a: 拒绝必须给出可读理由（否则用户不知道为何按 Enter 没反应），实际 = {reason:?}"
            );
        }
        other => panic!("N21a: 无 options + 空白输入必须 Reject，实际 = {other:?}"),
    }
}

#[test]
fn n21b_options_less_ask_submits_typed_answer() {
    // 无 options 的 ask：用户键入内容后 Enter 即提交（不得再伪造「确认/取消」两项）。
    assert!(
        matches!(tui::ask_submit_verdict(false, None, "我的回答"), tui::AskSubmitVerdict::Submit),
        "N21b: 无 options 且有输入必须 Submit"
    );
}

#[test]
fn n21c_options_ask_requires_selection_or_typed_input() {
    // 有 options 但既未选中又无输入 ⇒ Reject（不得把空选择当确认提交上去）。
    match tui::ask_submit_verdict(true, None, "  ") {
        tui::AskSubmitVerdict::Reject(reason) => {
            assert!(
                !reason.trim().is_empty(),
                "N21c: 拒绝必须给出可读理由，实际 = {reason:?}"
            );
        }
        other => panic!("N21c: 有 options + 未选中 + 空白输入必须 Reject，实际 = {other:?}"),
    }

    // 有 options 且已选中 ⇒ Submit。
    assert!(
        matches!(tui::ask_submit_verdict(true, Some(0), ""), tui::AskSubmitVerdict::Submit),
        "N21c: 有 options + 已选中必须 Submit"
    );
    // 有 options + 已选中 + 额外键入 ⇒ 仍应 Submit（自定义输入与选中项并存是合法输入）。
    assert!(
        matches!(
            tui::ask_submit_verdict(true, Some(0), "补充说明"),
            tui::AskSubmitVerdict::Submit
        ),
        "N21c: 有 options + 已选中 + 额外输入必须 Submit"
    );
}

// ── N22：多问推进（F-M②） ───────────────────────────────────────────────────────

#[test]
fn n22_next_question_index_advances_then_finishes() {
    // 只有 1 问：答完即结束。
    assert!(
        matches!(tui::next_question_index(0, 1), tui::NextQuestion::Finish),
        "N22: total=1 时必须 Finish（不得再推进到不存在的第 2 问）"
    );

    // 3 问的第 1 问之后：推进到索引 1。
    match tui::next_question_index(0, 3) {
        tui::NextQuestion::Advance(next) => assert_eq!(next, 1, "N22: 第 1 问后必须推进到索引 1"),
        other => panic!("N22: current=0,total=3 必须 Advance，实际 = {other:?}"),
    }

    // 3 问的第 3 问（索引 2）之后：结束。
    assert!(
        matches!(tui::next_question_index(2, 3), tui::NextQuestion::Finish),
        "N22: 最后一问（索引 2/total=3）之后必须 Finish"
    );

    // 越界（current >= total）：必须 Finish 且不得 panic。
    assert!(
        matches!(tui::next_question_index(5, 3), tui::NextQuestion::Finish),
        "N22: 越界索引必须 Finish（不得 panic、不得继续 Advance）"
    );
    // total = 0 的退化输入：同样不得 panic。
    assert!(
        matches!(tui::next_question_index(0, 0), tui::NextQuestion::Finish),
        "N22: total=0 的退化输入必须 Finish"
    );
}

// ── N23：`/plan` 诚实文案（F-N） ─────────────────────────────────────────────────

#[test]
fn n23_plan_mode_banner_is_honest_about_no_tool_restriction() {
    assert!(
        tui::plan_mode_banner(false).is_none(),
        "N23: 未激活 plan 模式时不得显示横幅"
    );

    let banner = tui::plan_mode_banner(true).expect("N23: 激活 plan 模式时必须显示横幅");
    assert!(
        banner.contains("不限制工具执行"),
        "N23: 横幅必须明示「不限制工具执行」（plan 模式未接线后端只读语义，防回退成只读暗示），实际 = {banner:?}"
    );
    // 反面护栏：不得出现「只读 / 禁止执行 / 不执行工具」这类会误导用户的暗示。
    for forbidden in ["只读", "禁止执行", "不执行工具"] {
        assert!(
            !banner.contains(forbidden),
            "N23: 横幅不得含误导性措辞 {forbidden:?}，实际 = {banner:?}"
        );
    }
}

// ── N24：多问推进的可观测状态面（F-M②，protocol 级） ───────────────────────────────
//
// 口径（exec-rust 已确认）：`AskUserState::record_and_advance(values) -> NextQuestion`
// **只记录答案并推进索引，不做任何提交**；`answer_ask` 仍只在事件循环里、且仅在返回
// `Finish` 时调用（配合全部 answers 一起提交）。因此本文件在 protocol 层断言：
// 「答完 1 问 ⇒ 索引推进 / answers 恰好 1 条 / all_answered()==false」——等价于
// 「未提前最终提交」；`answer_ask` 的调用面属事件循环，由 /plan 冒烟与手测覆盖（此处不伪造）。

fn question(id: &str, text: &str) -> tui::AskUserQuestion {
    tui::AskUserQuestion {
        id: id.to_string(),
        header: None,
        question: text.to_string(),
        multi_select: false,
        options: Vec::new(),
    }
}

#[test]
fn n24a_multi_question_ask_advances_without_early_submit() {
    let mut state = tui::AskUserState::new(
        "req-multi".to_string(),
        vec![question("q1", "第一个问题？"), question("q2", "第二个问题？")],
    );
    assert_eq!(state.answered_count(), 0, "N24a 前置条件：初始未答任何问题");
    assert!(!state.all_answered(), "N24a 前置条件：初始不得视为全部答完");
    assert_eq!(state.current_question_index, 0, "N24a 前置条件：从第一问开始");

    // 答第 1 问 → 必须推进到第 2 问，且**不**触发最终提交（all_answered 仍 false）。
    let first = state.record_and_advance(vec!["答案A".to_string()]);
    match first {
        tui::NextQuestion::Advance(next) => {
            assert_eq!(next, 1, "N24a: 答完第 1 问必须推进到索引 1")
        }
        other => panic!("N24a: 2 问场景答完第 1 问必须 Advance，实际 = {other:?}"),
    }
    assert_eq!(
        state.current_question_index, 1,
        "N24a: 推进后必须停在第 2 问（旧缺陷是只答第一问就关闭）"
    );
    assert_eq!(
        state.current_question().map(|q| q.id.as_str()),
        Some("q2"),
        "N24a: current_question 必须指向第 2 问"
    );
    assert_eq!(state.answers.len(), 1, "N24a: 已答集合必须恰好 1 条");
    assert_eq!(
        state.answers[0].question_id, "q1",
        "N24a: 已答记录必须绑定第 1 问的 id（不得错绑/漏绑）"
    );
    assert_eq!(
        state.answers[0].values,
        vec!["答案A".to_string()],
        "N24a: 已答记录必须原样保存提交值"
    );
    assert_eq!(state.answered_count(), 1, "N24a: answered_count 必须为 1");
    assert!(
        !state.all_answered(),
        "N24a: 还有一问未答，all_answered 必须为 false —— 否则事件循环会提前 answer_ask（旧缺陷）"
    );
}

#[test]
fn n24b_multi_question_ask_finishes_only_after_last_question() {
    let mut state = tui::AskUserState::new(
        "req-multi-2".to_string(),
        vec![question("q1", "第一问？"), question("q2", "第二问？")],
    );

    // 第 1 问
    let _ = state.record_and_advance(vec!["A".to_string()]);
    assert!(!state.all_answered(), "N24b: 答完第 1 问不得视为全部完成");

    // 第 2 问 → Finish（这才是 answer_ask 的唯一触发点）
    let second = state.record_and_advance(vec!["B".to_string()]);
    assert!(
        matches!(second, tui::NextQuestion::Finish),
        "N24b: 答完最后一问必须 Finish，实际 = {second:?}"
    );
    assert_eq!(state.answers.len(), 2, "N24b: 两问答案都必须被收集（不得只留最后一问）");
    assert_eq!(state.answers[0].question_id, "q1");
    assert_eq!(state.answers[1].question_id, "q2");
    assert_eq!(state.answers[1].values, vec!["B".to_string()]);
    assert_eq!(state.answered_count(), 2, "N24b: answered_count 必须为 2");
    assert!(
        state.all_answered(),
        "N24b: 全部答完后 all_answered 必须为 true（此时事件循环才提交 answer_ask）"
    );
}

#[test]
fn n24c_single_question_ask_finishes_immediately_without_advance() {
    let mut state = tui::AskUserState::new(
        "req-single".to_string(),
        vec![question("q1", "唯一一问？")],
    );

    let outcome = state.record_and_advance(vec!["只答这一问".to_string()]);
    assert!(
        matches!(outcome, tui::NextQuestion::Finish),
        "N24c: 单问场景答完必须立刻 Finish，实际 = {outcome:?}"
    );
    assert_eq!(state.answered_count(), 1);
    assert!(state.all_answered(), "N24c: 单问答完即 all_answered");
    assert_eq!(
        state.current_question_index, 0,
        "N24c: 单问场景不得把索引推进到越界位置"
    );
}

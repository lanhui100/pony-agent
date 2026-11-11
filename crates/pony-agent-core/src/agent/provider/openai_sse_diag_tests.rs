// B060：`sse-tool-call-missing-name` 诊断降噪的验收测试（Test Agent 依据
// `.dev-team/contract-matrix-b060.md` C1–C4 冻结）。
//
// 被测对象：`MissingNameDiagnostics`（pub(crate)，见 openai_sse.rs）。
// 契约：同一 index 每流至多 1 条诊断；重复观测只累计不打印；流结束至多 1 条汇总。
use super::*;

/// C1：同流同 index，`observe` 仅首次返回 `true`，其后恒 `false`（重复只累计静默）。
#[test]
fn c1_observe_same_index_true_only_on_first_call() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act + Assert：首次必须放行诊断行。
    assert!(
        diagnostics.observe(0),
        "C1：index=0 的首次观测必须返回 true（必须输出 1 条诊断行）"
    );
    // 其后连续重复观测一律静默。
    for attempt in 1..=4 {
        assert!(
            !diagnostics.observe(0),
            "C1：index=0 的第 {} 次重复观测必须返回 false（静默），否则缺陷日志继续刷屏",
            attempt + 1
        );
    }
}

/// C2：同流不同 index 互不吞噬——index 0 与 index 3 各自首次均返回 `true`。
#[test]
fn c2_observe_distinct_indexes_do_not_swallow_each_other() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act + Assert：不同 index 各自首见都必须放行。
    assert!(diagnostics.observe(0), "C2：index=0 首次观测必须返回 true");
    assert!(
        diagnostics.observe(3),
        "C2：index=3 首次观测必须返回 true（不得被 index=0 的已见状态吞掉）"
    );
    // 再回到已见 index 时各自静默。
    assert!(
        !diagnostics.observe(0),
        "C2：index=0 重复观测必须返回 false"
    );
    assert!(
        !diagnostics.observe(3),
        "C2：index=3 重复观测必须返回 false"
    );
}

/// C3：`suppressed_total()` 精确等于被静默的重复观测次数（不含首次放行的那次）。
#[test]
fn c3_suppressed_total_equals_exactly_the_suppressed_observation_count() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act：index 0 观测 1 次（0 次静默）、index 1 观测 3 次（2 次静默）、
    // index 7 观测 4 次（3 次静默）。
    assert!(diagnostics.observe(0), "C3：index=0 首见放行");
    assert!(diagnostics.observe(1), "C3：index=1 首见放行");
    assert!(!diagnostics.observe(1), "C3：index=1 第 2 次静默");
    assert!(!diagnostics.observe(1), "C3：index=1 第 3 次静默");
    assert!(diagnostics.observe(7), "C3：index=7 首见放行");
    assert!(!diagnostics.observe(7), "C3：index=7 第 2 次静默");
    assert!(!diagnostics.observe(7), "C3：index=7 第 3 次静默");
    assert!(!diagnostics.observe(7), "C3：index=7 第 4 次静默");

    // Assert：静默次数 = 2 + 3 = 5，精确相等。
    assert_eq!(
        diagnostics.suppressed_total(),
        5,
        "C3：suppressed_total 必须精确等于被静默的重复观测次数（2 + 3 = 5）"
    );
}

/// C3 补充：零重复观测（各自仅首见）时 `suppressed_total()` 必须为 0。
#[test]
fn c3_suppressed_total_is_zero_when_no_observation_is_suppressed() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act
    assert!(diagnostics.observe(0));
    assert!(diagnostics.observe(1));

    // Assert
    assert_eq!(
        diagnostics.suppressed_total(),
        0,
        "C3：全部为首次观测时不得产生任何抑制计数"
    );
}

/// C4-1：无任何观测时 `take_summary()` 必须为 `None`（流里没有缺名调用 ⇒ 无汇总行）。
#[test]
fn c4_take_summary_is_none_without_any_observation() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act + Assert
    assert!(
        diagnostics.take_summary().is_none(),
        "C4：零观测时不得输出汇总"
    );
}

/// C4-2：有观测但零抑制（每个 index 只见过一次）时 `take_summary()` 必须为 `None`。
#[test]
fn c4_take_summary_is_none_when_observed_but_nothing_suppressed() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();
    assert!(diagnostics.observe(0));
    assert!(diagnostics.observe(2));
    assert_eq!(diagnostics.suppressed_total(), 0);

    // Act + Assert
    assert!(
        diagnostics.take_summary().is_none(),
        "C4：有观测但零抑制时不得输出汇总行（否则降噪变成新增噪声）"
    );
}

/// C4-3：有抑制时 `take_summary()` 返回 `Some((抑制总数, 受抑制 index 数))` 且只返回一次。
#[test]
fn c4_take_summary_reports_suppressed_totals_exactly_once() {
    // Arrange：index 0 重复 3 次 → 2 次静默；index 5 重复 2 次 → 1 次静默。
    let mut diagnostics = MissingNameDiagnostics::default();
    assert!(diagnostics.observe(0));
    assert!(!diagnostics.observe(0));
    assert!(!diagnostics.observe(0));
    assert!(diagnostics.observe(5));
    assert!(!diagnostics.observe(5));
    assert_eq!(diagnostics.suppressed_total(), 3);

    // Act
    let first = diagnostics.take_summary();
    let second = diagnostics.take_summary();

    // Assert：统计口径精确（总静默 3 次、涉及 2 个 index），且汇总只出一次。
    assert_eq!(
        first,
        Some((3, 2)),
        "C4：汇总必须为 (抑制总数=3, 受抑制 index 数=2)"
    );
    assert_eq!(second, None, "C4：汇总只允许返回一次，二次调用必须为 None");
    assert_eq!(
        diagnostics.suppressed_total(),
        3,
        "C4：取汇总不得清零只读计数"
    );
}

/// C4-4：单 index 抑制场景下汇总口径为 `(静默次数, 1)`。
#[test]
fn c4_take_summary_single_index_reports_one_suppressed_index() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();
    assert!(diagnostics.observe(9));
    assert!(!diagnostics.observe(9));

    // Act + Assert
    assert_eq!(
        diagnostics.take_summary(),
        Some((1, 1)),
        "C4：仅 index=9 被抑制 1 次时应返回 (1, 1)"
    );
}

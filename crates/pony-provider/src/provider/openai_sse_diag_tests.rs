// B060：`sse-tool-call-missing-name` 诊断降噪的验收测试（Test Agent 依据
// `.dev-team/contract-matrix-b060.md` v1.1「v1.1 修订」节冻结：C1r / C2 / C3 / C4r / C7）。
//
// 被测对象：`MissingNameDiagnostics`（pub(crate)，见 openai_sse.rs）。
// 契约要点：同一 index 每流至多 1 条诊断行；重复观测只累计不打印；流结束至多 1 条汇总行；
// 汇总行前缀**不得**复用诊断巡检针 `sse-tool-call-missing-name`（C7 口径隔离）。
//
// 本文件断言的是**日志文本本身**（而非布尔判定），因此把"接线被降级为 `let _ = ...`
// 或 finish 汇总被删除"这类回归钉成机器可判的红灯。
use super::*;

/// 断言辅助：首次观测必须返回诊断行，返回其文本用于后续逐字段断言。
fn expect_first_line(
    diagnostics: &mut MissingNameDiagnostics,
    index: usize,
    preview: &str,
) -> String {
    diagnostics.diagnostic_line(index, preview).unwrap_or_else(|| {
        panic!("索引 {index} 的首次观测必须返回诊断行（Some(line)），实际为 None")
    })
}

const PREVIEW_A: &str = r#"{"index":0,"function":{"arguments":"{\"path\":\"src/lib.rs\"}"}}"#;
const PREVIEW_B: &str = r#"{"index":3,"arguments":"{\"calls\":[{\"tool\":\"Read\"}]}"}"#;

/// C1r：首见返回的 `Some(line)` 必须逐字承载巡检针前缀与 preview。
#[test]
fn c1r_diagnostic_line_first_observation_keeps_prefix_and_preview_verbatim() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act
    let line = expect_first_line(&mut diagnostics, 0, PREVIEW_A);

    // Assert：前缀含 index 值、字段名齐备、preview 逐字保留（末尾即原文）。
    assert!(
        line.starts_with("provider:sse-tool-call-missing-name index=0"),
        "C1r：诊断行必须以 `provider:sse-tool-call-missing-name index=0` 开头，实际 = {line:?}"
    );
    assert!(
        line.contains("item_preview="),
        "C1r：诊断行必须含 `item_preview=` 字段，实际 = {line:?}"
    );
    assert!(
        line.ends_with(PREVIEW_A),
        "C1r：诊断行必须逐字保留传入的 preview（末尾应为 {PREVIEW_A:?}），实际 = {line:?}"
    );
    assert!(
        line.contains("sse-tool-call-missing-name"),
        "C1r：诊断行必须保留巡检针前缀子串，否则 B060 巡检口径失效，实际 = {line:?}"
    );
    assert_eq!(
        diagnostics.suppressed_total(),
        0,
        "C1r：首次观测（已输出诊断行）不得计入抑制"
    );
}

/// C1r + C3：同 index 重复观测返回 `None`，且每次精确累计 1 次抑制。
#[test]
fn c1r_diagnostic_line_repeats_return_none_and_count_suppression() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();
    expect_first_line(&mut diagnostics, 0, PREVIEW_A);

    // Act + Assert：每次重复都必须静默 +1。
    for attempt in 2..=4 {
        assert_eq!(
            diagnostics.diagnostic_line(0, PREVIEW_A),
            None,
            "C1r：index=0 的第 {attempt} 次观测必须返回 None（静默），否则缺陷日志继续刷屏"
        );
        assert_eq!(
            diagnostics.suppressed_total(),
            attempt - 1,
            "C3：第 {attempt} 次观测完成后抑制计数必须精确等于 {}",
            attempt - 1
        );
    }
}

/// C2：同流不同 index 互不吞噬——各自首见均返回诊断行，且各带自己的 index/preview。
#[test]
fn c2_diagnostic_line_distinct_indexes_do_not_swallow_each_other() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act
    let line_zero = expect_first_line(&mut diagnostics, 0, PREVIEW_A);
    let line_three = expect_first_line(&mut diagnostics, 3, PREVIEW_B);

    // Assert：两条诊断行都必须产出，且各自身份可区分。
    assert!(
        line_zero.starts_with("provider:sse-tool-call-missing-name index=0"),
        "C2：index=0 的诊断行必须以 index=0 开头，实际 = {line_zero:?}"
    );
    assert!(
        line_three.starts_with("provider:sse-tool-call-missing-name index=3"),
        "C2：index=3 的首次观测必须产出自己的诊断行（不得被 index=0 的已见状态吞掉），实际 = {line_three:?}"
    );
    assert!(
        line_zero.ends_with(PREVIEW_A) && line_three.ends_with(PREVIEW_B),
        "C2：每个 index 的诊断行必须承载自己的 preview"
    );
    assert_eq!(
        diagnostics.suppressed_total(),
        0,
        "C2：不同 index 的首次观测均属放行，不得产生抑制计数"
    );
    // 回到已见 index 时静默，且不影响另一 index 的既有判定。
    assert_eq!(diagnostics.diagnostic_line(0, PREVIEW_A), None);
    assert_eq!(diagnostics.diagnostic_line(3, PREVIEW_B), None);
    assert_eq!(
        diagnostics.suppressed_total(),
        2,
        "C3：两次重复静默后计数必须精确为 2"
    );
}

/// C3：`suppressed_total()` 精确等于被静默的重复观测次数（不含首次放行的那次）。
#[test]
fn c3_suppressed_total_equals_exactly_the_suppressed_observation_count() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act：index 0 观测 1 次（0 次静默）、index 1 观测 3 次（2 次静默）、
    // index 7 观测 4 次（3 次静默）。
    expect_first_line(&mut diagnostics, 0, PREVIEW_A);
    expect_first_line(&mut diagnostics, 1, PREVIEW_A);
    assert_eq!(diagnostics.diagnostic_line(1, PREVIEW_A), None);
    assert_eq!(diagnostics.diagnostic_line(1, PREVIEW_A), None);
    expect_first_line(&mut diagnostics, 7, PREVIEW_B);
    assert_eq!(diagnostics.diagnostic_line(7, PREVIEW_B), None);
    assert_eq!(diagnostics.diagnostic_line(7, PREVIEW_B), None);
    assert_eq!(diagnostics.diagnostic_line(7, PREVIEW_B), None);

    // Assert：静默次数 = 2 + 3 = 5，精确相等。
    assert_eq!(
        diagnostics.suppressed_total(),
        5,
        "C3：suppressed_total 必须精确等于被静默的重复观测次数（2 + 3 = 5）"
    );
}

/// C3 边界：零重复观测（各自仅首见）时 `suppressed_total()` 必须为 0。
#[test]
fn c3_suppressed_total_is_zero_when_no_observation_is_suppressed() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act
    expect_first_line(&mut diagnostics, 0, PREVIEW_A);
    expect_first_line(&mut diagnostics, 2, PREVIEW_B);

    // Assert
    assert_eq!(
        diagnostics.suppressed_total(),
        0,
        "C3：全部为首次观测时不得产生任何抑制计数"
    );
}

/// C4r-1：无任何观测时 `summary_line()` 必须为 `None`（流里没有缺名调用 ⇒ 无汇总行）。
#[test]
fn c4r_summary_line_is_none_without_any_observation() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();

    // Act + Assert
    assert_eq!(
        diagnostics.summary_line(),
        None,
        "C4r：零观测时不得输出汇总行"
    );
}

/// C4r-2：有观测但零抑制（每个 index 只见过一次）时 `summary_line()` 必须为 `None`。
#[test]
fn c4r_summary_line_is_none_when_observed_but_nothing_suppressed() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();
    expect_first_line(&mut diagnostics, 0, PREVIEW_A);
    expect_first_line(&mut diagnostics, 2, PREVIEW_B);
    assert_eq!(diagnostics.suppressed_total(), 0);

    // Act + Assert
    assert_eq!(
        diagnostics.summary_line(),
        None,
        "C4r：有观测但零抑制时不得输出汇总行（否则降噪变成新增噪声）"
    );
}

/// C4r-3：有抑制时 `summary_line()` 返回含精确统计的文本，且只返回一次。
#[test]
fn c4r_summary_line_reports_exact_totals_exactly_once() {
    // Arrange：index 0 观测 3 次 → 2 次静默；index 5 观测 4 次 → 3 次静默（合计 5 次 / 2 个 index）。
    let mut diagnostics = MissingNameDiagnostics::default();
    expect_first_line(&mut diagnostics, 0, PREVIEW_A);
    assert_eq!(diagnostics.diagnostic_line(0, PREVIEW_A), None);
    assert_eq!(diagnostics.diagnostic_line(0, PREVIEW_A), None);
    expect_first_line(&mut diagnostics, 5, PREVIEW_B);
    assert_eq!(diagnostics.diagnostic_line(5, PREVIEW_B), None);
    assert_eq!(diagnostics.diagnostic_line(5, PREVIEW_B), None);
    assert_eq!(diagnostics.diagnostic_line(5, PREVIEW_B), None);
    assert_eq!(diagnostics.suppressed_total(), 5);

    // Act
    let summary = diagnostics.summary_line();

    // Assert：统计口径精确（总静默 5、涉及 2 个 index）。
    let summary = summary.unwrap_or_else(|| panic!("C4r：存在 5 次抑制时必须输出汇总行"));
    assert!(
        summary.contains("suppressed_total=5"),
        "C4r：汇总行必须含 `suppressed_total=5`，实际 = {summary:?}"
    );
    assert!(
        summary.contains("suppressed_indexes=2"),
        "C4r：汇总行必须含 `suppressed_indexes=2`，实际 = {summary:?}"
    );

    // Assert：汇总只出一次，且取汇总不清零只读计数。
    assert_eq!(
        diagnostics.summary_line(),
        None,
        "C4r：汇总行只允许返回一次，二次调用必须为 None"
    );
    assert_eq!(
        diagnostics.suppressed_total(),
        5,
        "C4r：取汇总不得清零只读计数"
    );
}

/// C4r-4：单 index 抑制场景下汇总行口径为 `suppressed_total=1` + `suppressed_indexes=1`。
#[test]
fn c4r_summary_line_single_index_reports_one_suppressed_index() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();
    expect_first_line(&mut diagnostics, 9, PREVIEW_A);
    assert_eq!(diagnostics.diagnostic_line(9, PREVIEW_A), None);

    // Act
    let summary = diagnostics
        .summary_line()
        .unwrap_or_else(|| panic!("C4r：存在 1 次抑制时必须输出汇总行"));

    // Assert
    assert!(
        summary.contains("suppressed_total=1"),
        "C4r：单次抑制的汇总行必须含 `suppressed_total=1`，实际 = {summary:?}"
    );
    assert!(
        summary.contains("suppressed_indexes=1"),
        "C4r：单 index 抑制的汇总行必须含 `suppressed_indexes=1`，实际 = {summary:?}"
    );
}

/// C7：汇总行**不得**包含诊断巡检针子串 `sse-tool-call-missing-name`（口径隔离），
/// 同时用诊断行做正对照，保证该否定断言不是空转。
#[test]
fn c7_summary_line_does_not_reuse_diagnostic_probe_prefix() {
    // Arrange
    let mut diagnostics = MissingNameDiagnostics::default();
    let diagnostic = expect_first_line(&mut diagnostics, 0, PREVIEW_A);
    assert_eq!(diagnostics.diagnostic_line(0, PREVIEW_A), None);
    assert_eq!(diagnostics.diagnostic_line(0, PREVIEW_A), None);

    // Act
    let summary = diagnostics
        .summary_line()
        .unwrap_or_else(|| panic!("C7：存在 2 次抑制时必须输出汇总行"));

    // Assert：正对照——诊断行本身必须命中巡检针。
    assert!(
        diagnostic.contains("sse-tool-call-missing-name"),
        "C7：正对照失败——诊断行必须含巡检针 `sse-tool-call-missing-name`，实际 = {diagnostic:?}"
    );
    // Assert：汇总行必须与巡检针口径隔离，否则 `grep -c 'sse-tool-call-missing-name'` 统计被污染。
    assert!(
        !summary.contains("sse-tool-call-missing-name"),
        "C7：汇总行不得包含 `sse-tool-call-missing-name` 子串（防止巡检计数被汇总行污染），实际 = {summary:?}"
    );
    // Assert：汇总行必须承载自身统计字段（独立可识别的汇总口径）。
    assert!(
        summary.contains("suppressed_total=2") && summary.contains("suppressed_indexes=1"),
        "C7：汇总行必须承载自身统计字段，实际 = {summary:?}"
    );
}

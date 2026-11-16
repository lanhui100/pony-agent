//! B223 workspace 外写拒绝的可执行提示（红相）。
//!
//! 现场：沙箱拒绝写到 workspace 之外时，用户只看到「拒绝写入」+ escalation 提示，但没有
//! 任何可执行替代——不知道往哪写、下一步做什么。契约：新增
//! `outside_workspace_write_denied_with_workspace(message, workspace_root)`，在既有
//! escalation 提示之外**附受控可写目录**（`<workspace_root>/.tmp`，见 `tools/paths.rs`
//! 的受控 tmp 约定）与一次可执行纠正建议；结构化错误码不换码。
//!
//! 本文件全部为真实断言；不读真实时钟、不 sleep、不吞异常。

use std::path::Path;

use pony_agent_core::agent::path_permission::{PermissionError, PermissionErrorCode};

/// 既有 escalation 提示的稳定前缀（旧构造器逐字写入的片段）。
const ESCALATION_MARKER: &str = "[sandbox: escalation available";

fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// 1. 结构信封不换码：仍是 `outside_workspace_write_denied`。
#[test]
fn b223_code_unchanged() {
    // Arrange
    let root = Path::new("/tmp/ws");

    // Act
    let error = PermissionError::outside_workspace_write_denied_with_workspace("拒绝写入。", root);

    // Assert
    assert_eq!(
        error.code,
        PermissionErrorCode::OutsideWorkspaceWriteDenied,
        "结构化错误码枚举不得更换，实际 = {:?}",
        error.code
    );
    assert_eq!(
        error.code.as_str(),
        "outside_workspace_write_denied",
        "wire 上必须仍是 outside_workspace_write_denied（前端审批 UI 依赖该码）"
    );
}

/// 2. 文案必须给出受控可写目录与一次可执行建议。
#[test]
fn b223_message_hints_controlled_tmp() {
    // Arrange
    let root = Path::new("/tmp/ws");
    let root_text = root.display().to_string();

    // Act
    let error = PermissionError::outside_workspace_write_denied_with_workspace("拒绝写入。", root);
    let message = &error.message;

    // Assert 1：可写替代目录（受控 tmp）。
    assert!(
        message.contains(".tmp"),
        "文案必须给出受控可写目录 .tmp，实际 = {message:?}"
    );

    // Assert 2：明确「可写」这一可执行语义（而不是只报错）。
    assert!(
        message.contains("可写"),
        "文案必须说明该目录可写，实际 = {message:?}"
    );

    // Assert 3：点名 workspace root 的文本，用户才能定位替代目录位置。
    assert!(
        message.contains(&root_text),
        "文案必须含 workspace root 路径文本 {root_text:?}，实际 = {message:?}"
    );

    // Assert 4：一次可执行纠正建议。
    assert!(
        message.contains("改写") || message.contains("重试"),
        "文案必须给出可执行建议（改写路径 / 重试），实际 = {message:?}"
    );
}

/// 3. 幂等提示：入参已含 escalation 片段时不得重复追加。
#[test]
fn b223_hint_is_idempotent() {
    // Arrange：调用方已把 escalation 提示写进 message（真实调用链会二次经过构造器）。
    let root = Path::new("/tmp/ws");
    let message_already_hinted =
        "拒绝写入。 [sandbox: escalation available — retry with explicit authorization]";

    // Arrange：同一 root 的「未预先带提示」基线（用于按字数比对，不写死措辞次数）。
    let baseline =
        PermissionError::outside_workspace_write_denied_with_workspace("拒绝写入。", root).message;
    let baseline_tmp = occurrences(&baseline, ".tmp");
    assert!(
        baseline_tmp >= 1,
        "前提：新构造器必须给出受控 tmp 提示，基线 = {baseline:?}"
    );

    // Act
    let error = PermissionError::outside_workspace_write_denied_with_workspace(
        message_already_hinted,
        root,
    );
    let message = &error.message;

    // Assert 1：escalation 提示只能出现一次。
    assert_eq!(
        occurrences(message, ESCALATION_MARKER),
        1,
        "escalation 提示不得重复追加，实际 = {message:?}"
    );

    // Assert 2：受控 tmp 提示不得叠加（次数必须与基线一致，而不是只写一遍的死板口径）。
    assert_eq!(
        occurrences(message, ".tmp"),
        baseline_tmp,
        "受控 tmp 提示不得重复追加（已带 escalation 片段时次数应与基线一致 = {baseline_tmp}），实际 = {message:?}"
    );
}

/// 4. 旧构造器行为不变：仍以 escalation 提示结尾（既有调用方零改动）。
#[test]
fn b223_legacy_constructor_still_appends_escalation() {
    // Act
    let error = PermissionError::outside_workspace_write_denied("x");
    let message = error.message.trim_end().to_string();

    // Assert 1：仍带 escalation 提示。
    assert_eq!(
        occurrences(&message, ESCALATION_MARKER),
        1,
        "旧构造器必须继续追加一次 escalation 提示，实际 = {message:?}"
    );

    // Assert 2：它以该提示结尾（不得在后面再挂其他片段）。
    assert!(
        message.ends_with(']'),
        "escalation 提示必须仍是文案结尾，实际 = {message:?}"
    );
    let tail = message
        .rsplit_once(ESCALATION_MARKER)
        .map(|(_, tail)| tail)
        .unwrap_or_default();
    assert!(
        tail.ends_with(']') && !tail.contains("可写"),
        "escalation 之后的尾段只能是提示本体，实际尾段 = {tail:?}"
    );

    // Assert 3：错误码不变。
    assert_eq!(
        error.code.as_str(),
        "outside_workspace_write_denied",
        "旧构造器的错误码不得改变"
    );
}

/// 5. 尾随空白 message：仍须给出替代目录提示（按 trim 后判定「是否已有提示」）。
#[test]
fn b223_trailing_whitespace_message_still_hinted() {
    // Arrange
    let root = Path::new("/tmp/ws");
    let message_with_trailing_space = "拒绝写入。  ";

    // Act
    let error = PermissionError::outside_workspace_write_denied_with_workspace(
        message_with_trailing_space,
        root,
    );

    // Assert
    assert!(
        error.message.contains(".tmp"),
        "尾随空白的 message 也必须追加受控 tmp 提示，实际 = {:?}",
        error.message
    );
    assert!(
        error.message.contains("可写"),
        "尾随空白的 message 也必须说明可写目录，实际 = {:?}",
        error.message
    );
}

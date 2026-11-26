//! B223 workspace 外写拒绝的可执行提示（红相）；B204 起同时锁定「不承诺升级审批」。
//!
//! 现场：沙箱拒绝写到 workspace 之外时，用户只看到「拒绝写入」+ 无落点的 escalation 提示，
//! 没有任何可执行替代——不知道往哪写、下一步做什么。
//! 契约：`outside_workspace_write_denied_with_workspace(message, workspace_root)` 在拒绝文案中
//! **附受控可写目录**（`<workspace_root>/.tmp`，见 `tools/paths.rs` 的受控 tmp 约定）与一次
//! 可执行纠正建议；结构化错误码不换码。
//!
//! B204 补充契约：拒绝路径**不产生**任何可批准的升级请求，故文案不得再出现
//! `[sandbox: escalation available …]` 之类无法兑现的承诺。
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

/// 3. 幂等提示：入参已含受控可写目录提示时不得重复追加。
#[test]
fn b223_hint_is_idempotent() {
    // Arrange：调用方已把可写目录提示写进 message（真实调用链会二次经过构造器）。
    let root = Path::new("/tmp/ws");
    let already_hinted = PermissionError::outside_workspace_write_denied_with_workspace(
        "拒绝写入。",
        root,
    )
    .message;
    let baseline_tmp = occurrences(&already_hinted, ".tmp");
    assert!(
        baseline_tmp >= 1,
        "前提：新构造器必须给出受控 tmp 提示，基线 = {already_hinted:?}"
    );

    // Act：把已带提示的文案再次送入构造器。
    let error =
        PermissionError::outside_workspace_write_denied_with_workspace(already_hinted, root);
    let message = &error.message;

    // Assert：受控 tmp 提示不得叠加（次数必须与基线一致）。
    assert_eq!(
        occurrences(message, ".tmp"),
        baseline_tmp,
        "受控 tmp 提示不得重复追加（基线 = {baseline_tmp}），实际 = {message:?}"
    );
}

/// 4. 旧构造器（无 workspace root）：只给通用可执行指引，不承诺升级审批（B204）。
#[test]
fn b204_legacy_constructor_never_promises_escalation() {
    // Act
    let error = PermissionError::outside_workspace_write_denied("拒绝写入。");
    let message = error.message.trim_end().to_string();

    // Assert 1：不得再出现无法兑现的 escalation 承诺。
    assert_eq!(
        occurrences(&message, ESCALATION_MARKER),
        0,
        "拒绝路径不产生审批请求 ⇒ 文案不得承诺可升级，实际 = {message:?}"
    );

    // Assert 2：必须给出可执行替代（工作区内路径 / 受控 tmp 目录）。
    assert!(
        message.contains(".tmp") || message.contains("工作区内"),
        "必须给出可执行的替代路径指引，实际 = {message:?}"
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

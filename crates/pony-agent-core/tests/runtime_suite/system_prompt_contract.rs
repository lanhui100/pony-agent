//! 黑盒契约测试：系统提示词 v2（System Prompt v2）
//!
//! 契约唯一来源：`.dev-team/contracts/system-prompt-v2.md`（FROZEN v2.0.0）
//! 读取入口固定为 `agent::context::base_system_prompt()`，禁止直接引用私有常量。
//!
//! 断言 ID 与契约第 4 节断言表一一对应（SP1–SP10）。
//! 红线原则：本文件为独立测试方（test-agent）交付物，实施者只读不可改。

use pony_agent_core::agent::context::{base_system_prompt, prompt_layers_are_ordered};
use pony_agent_core::agent::provider::ProviderMessage;

/// SP1：工具结果与网页内容必须被视为不可信数据；遇提示注入应上报用户而非执行。
#[test]
fn sp1_tool_results_are_untrusted_data_and_prompt_injection_is_flagged() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_untrusted_data = prompt.contains("untrusted data");
    let has_prompt_injection = prompt.contains("prompt injection");

    // Assert
    assert!(
        has_untrusted_data,
        "SP1 违约：稳定层提示词必须把工具结果与网页内容声明为 `untrusted data`（不可信数据），实际提示词未命中该片段。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
    assert!(
        has_prompt_injection,
        "SP1 违约：稳定层提示词必须显式处理 `prompt injection` 场景（上报用户而非执行），实际未命中该片段。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
}

/// SP2：禁止复述用户已看到的内容（tool output / plan / question）。
#[test]
fn sp2_no_restating_clause_is_present() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_no_restating = prompt.contains("No restating");

    // Assert
    assert!(
        has_no_restating,
        "SP2 违约：稳定层提示词必须包含 `No restating` 反冗余条款，实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
}

/// SP3：最终答复默认不超过 5 行（用户显式要求深度时例外）。
#[test]
fn sp3_final_answer_under_five_lines_clause_is_present() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_final_answer_limit = prompt.contains("Final answer under 5 lines");

    // Assert
    assert!(
        has_final_answer_limit,
        "SP3 违约：稳定层提示词必须包含 `Final answer under 5 lines` 篇幅上限条款，实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
}

/// SP4：决策文本阈值收敛为 2 行，且不得残留旧的 `2-3 lines` 表述。
#[test]
fn sp4_decision_text_threshold_is_two_lines_without_legacy_three_line_wording() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_two_line_threshold = prompt.contains("Decision text under 2 lines");
    let has_legacy_2_to_3_lines = prompt.contains("2-3 lines");

    // Assert
    assert!(
        has_two_line_threshold,
        "SP4 违约：决策文本阈值必须收敛为 `Decision text under 2 lines`，实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
    assert!(
        !has_legacy_2_to_3_lines,
        "SP4 违约：稳定层提示词不得残留 v1 旧表述 `2-3 lines`（与 2 行阈值互斥），实际仍命中"
    );
}

/// SP5：以机器凭证（exit code / 断言 / 编译器输出）判定进度，非零退出必须先排查。
#[test]
fn sp5_objective_receipts_reference_exit_code_and_investigation() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_exit_code = prompt.contains("exit code");
    let has_non_zero_investigation = prompt.contains("Investigate any non-zero exit");

    // Assert
    assert!(
        has_exit_code,
        "SP5 违约：稳定层提示词必须以 `exit code` 作为客观收据依据，实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
    assert!(
        has_non_zero_investigation,
        "SP5 违约：稳定层提示词必须包含 `Investigate any non-zero exit`（非零退出先排查再继续），实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
}

/// SP6：v1 的命令式直接作答与上下文封闭式表述必须已删除。
#[test]
fn sp6_legacy_direct_answer_and_context_closed_wording_are_absent() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_legacy_direct_answer = prompt.contains("Understand the latest request and solve it directly");
    let has_legacy_context_closed = prompt.contains("Only rely on the visible request context");

    // Assert
    // 空转防护：纯否定断言在空串桩上会假绿，故先要求稳定层提示词已实际注入。
    assert!(
        !prompt.is_empty(),
        "SP6 违约：稳定层提示词为空，否定断言在空串上会假绿（空转通过），未构成有效契约校验"
    );
    assert!(
        !has_legacy_direct_answer,
        "SP6 违约：稳定层提示词不得残留 v1 片段 `Understand the latest request and solve it directly`，实际仍命中"
    );
    assert!(
        !has_legacy_context_closed,
        "SP6 违约：稳定层提示词不得残留 v1 片段 `Only rely on the visible request context`，实际仍命中"
    );
}

/// SP7：多步任务用 `TodoWrite` 跟踪，且不得残留旧的初始化措辞。
#[test]
fn sp7_todo_contract_uses_todo_write_without_legacy_initialization_wording() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_todo_write = prompt.contains("TodoWrite");
    let has_legacy_initialization = prompt.contains("Initialize pending items");

    // Assert
    assert!(
        has_todo_write,
        "SP7 违约：稳定层提示词的 todo 契约必须指向工具 `TodoWrite`，实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
    assert!(
        !has_legacy_initialization,
        "SP7 违约：稳定层提示词不得残留 v1 初始化措辞 `Initialize pending items`，实际仍命中"
    );
}

/// SP8：更严格的工作区指令优先；不得臆造工具/能力。
#[test]
fn sp8_stricter_wins_and_no_capability_fabrication() {
    // Arrange
    let prompt = base_system_prompt();

    // Act
    let has_stricter_wins = prompt.contains("Stricter wins");
    let has_no_capability_fabrication =
        prompt.contains("Do not assume tools or capabilities beyond those provided");

    // Assert
    assert!(
        has_stricter_wins,
        "SP8 违约：稳定层提示词必须包含 `Stricter wins`（工作区指令更严时从严），实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
    assert!(
        has_no_capability_fabrication,
        "SP8 违约：稳定层提示词必须包含 `Do not assume tools or capabilities beyond those provided`，实际未命中。当前 base_system_prompt() 长度 = {} 字节",
        prompt.len()
    );
}

/// SP9：稳定层体量受控（<= 3900 字节，Wave-1 Lead 裁决值）。
///
/// 附加护栏：体积上限不得被空串桩"空转通过"，故同时要求提示词非空。
#[test]
fn sp9_base_system_prompt_size_is_within_budget() {
    // Arrange
    const MAX_BYTES: usize = 3900;
    let prompt = base_system_prompt();

    // Act
    let actual_bytes = prompt.len();
    let within_budget = actual_bytes <= MAX_BYTES;

    // Assert
    assert!(
        !prompt.is_empty(),
        "SP9 违约：稳定层提示词为空（空桩或未注入），体积上限被空转满足，契约面未实现"
    );
    assert!(
        within_budget,
        "SP9 违约：稳定层提示词体积 {} 字节，超出上限 {} 字节（契约第 4 节 SP9 / Lead 裁决 <= 3900）",
        actual_bytes,
        MAX_BYTES
    );
}

/// SP10：层序不变量 —— 稳定层（System）必须全部排在动态层（Developer）之前，
/// 且首个 System 消息必须以 `You are Pony Agent` 开头。
#[test]
fn sp10_prompt_layers_are_ordered_follows_contract_table() {
    // Arrange
    let only_system = vec![ProviderMessage::system("You are Pony Agent")];
    let system_then_developers = vec![
        ProviderMessage::system("You are Pony Agent"),
        ProviderMessage::developer("dynamic layer A"),
        ProviderMessage::developer("dynamic layer B"),
    ];
    let developer_before_system = vec![
        ProviderMessage::developer("dynamic layer"),
        ProviderMessage::system("You are Pony Agent"),
    ];
    let no_layers: Vec<ProviderMessage> = Vec::new();
    let system_without_identity_prefix = vec![ProviderMessage::system("hello")];

    // Act
    let result_system_then_developers = prompt_layers_are_ordered(&system_then_developers);
    let result_developer_before_system = prompt_layers_are_ordered(&developer_before_system);
    let result_no_layers = prompt_layers_are_ordered(&no_layers);
    let result_only_system = prompt_layers_are_ordered(&only_system);
    let result_system_without_identity_prefix =
        prompt_layers_are_ordered(&system_without_identity_prefix);

    // Assert
    assert!(
        result_system_then_developers,
        "SP10 违约：[System, Developer, Developer] 的动态层全部位于稳定层之后，应返回 true，实际返回 false"
    );
    assert!(
        !result_developer_before_system,
        "SP10 违约：[Developer, System] 动态层越位到稳定层之前，应返回 false，实际返回 true"
    );
    assert!(
        !result_no_layers,
        "SP10 违约：[] 无任何层可判定，不得视为合法层序，应返回 false，实际返回 true"
    );
    assert!(
        result_only_system,
        "SP10 违约：[System] 稳定层单一、无动态层越位，应返回 true，实际返回 false"
    );
    assert!(
        !result_system_without_identity_prefix,
        "SP10 违约：首条 System 内容 `hello` 不以 `You are Pony Agent` 开头，违反稳定层身份标识补充定义，应返回 false，实际返回 true"
    );
}

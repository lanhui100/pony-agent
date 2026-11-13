//! 提示词 ↔ 工具注册表契约面：提示词按名字点名了哪些模型可见工具。
//!
//! 稳定层提示词（`agent::context::BASE_SYSTEM_PROMPT` 与 `AGENT_TEAM_POLICY_PROMPT`）
//! 按**产品名**点名工具（`TodoWrite` / `create_goal` / `update_goal` / 团队工具 / `Run`），
//! 而工具名、别名与曝光面独立维护在同一模块的 `names` 里。两份事实源之间没有任何
//! 编译期约束：改名、降曝光、删工具都会让提示词继续指挥模型调用一个不存在或不可见的
//! 工具——模型侧的「提示词撒谎」，且不会让任何既有测试变红。
//!
//! 本模块存在的唯一目的：把「提示词点名了哪些工具」从自然语言里提成一份可被测试读取的
//! 清单。`tests/prompt_tool_consistency.rs`（PT1–PT5）双向校验本清单与提示词文本、
//! 工具注册表、冻结契约文档三者的一致性。
//!
//! 维护约定：改提示词里的工具名 → 同提交改本清单；改工具名/曝光面 → 同提交改清单或提示词。

/// 提示词里**按名字引用**的模型可见工具（一致性门禁的唯一真源）。
///
/// `tests/prompt_tool_consistency.rs` 做双向机械校验：
/// 登记项必须存在于工具注册表（PT1）且曝光面为 `ModelVisible`（PT2），
/// 且必须真的出现在提示词文本里（PT3）；反向地，提示词中出现的长工具名必须已登记（PT4）。
pub fn prompt_tool_references() -> &'static [&'static str] {
    &[
        // BASE_SYSTEM_PROMPT
        "TodoWrite",
        "create_goal",
        "update_goal",
        // AGENT_TEAM_POLICY_PROMPT
        "spawn_teammate",
        "list_agents",
        "send_message",
        "interrupt_agent",
        "team_task_list",
        "team_task_update",
        "Run",
    ]
}

/// 提示词里**看起来像工具名、但不是工具**的 token 白名单（PT4 反扫的豁免表）。
///
/// PT4 对提示词做工具名反扫，用的是形状启发式（长度 ≥5 且含下划线或内部大写）。
/// 该启发式必然命中少量非工具 token；把它们显式登记在这里，比放宽启发式更安全：
/// 新增一个"像工具名却解析不到"的 token 会让门禁变红，逼维护者确认它到底是
/// 拼写错误、被删工具，还是普通散文——fail-closed，而不是静默放过。
pub fn prompt_non_tool_tokens() -> &'static [&'static str] {
    &[
        // 团队策略段里的文件版本冲突错误码（tools::workspace 的返回值），不是工具。
        "cas_conflict",
        // 团队策略段里的共享任务表字段名（team_task_update 的入参），不是工具。
        "expected_revision",
        // 团队策略段用通配写法指代 team_task_* 一族，tokenizer 会把星号切掉只剩前缀。
        "team_task_",
        // BASE 段里作为示例出现的类名（`搜索 TokenManager 类`），不是工具。
        "TokenManager",
    ]
}

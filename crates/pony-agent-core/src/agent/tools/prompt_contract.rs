//! 提示词 ↔ 工具注册表契约面：工具名真源、工具自注册的用法条款、稳定层渲染器。
//!
//! 稳定层提示词（`agent::system_prompt_template`）的模板里**不出现任何工具专名**，只有占位符：
//! - `{{tool:<primitive>}}` —— 渲染时从 `tools::names` 取模型可见名（名称/曝光的唯一真源）；
//! - `{{guideline:<key>}}` —— 渲染时取本模块 [`GUIDELINES`] 里工具自注册的用法条款。
//!
//! 这样改工具名、降曝光面、删工具都只改工具注册表一处；提示词侧要么自动跟随，要么整条消失。
//! `tests/prompt_tool_consistency.rs`（PT1–PT6）双向校验本模块、提示词文本与冻结契约文档。
//!
//! 为什么清单放在本模块而非 `agent/context.rs`：后者已顶到 `size:check` 的存量基线（3003 行），
//! 净增任何行都是新违规；按 `implemented/process/2026-11-12-file-size-gate-module-split-not-baseline-bump.md`
//! 的先例拆模块，不抬基线。

use std::sync::OnceLock;

use super::names::{model_visible_tool_name_opt, tool_exposure_for_name};
use super::{ToolExposure, canonical_tool_name};

/// 一条「由工具 owns 的提示词用法条款」。
///
/// `primitive` 是稳定 primitive 名（工具注册表的键），`text` 里的 `{{tool}}` 在渲染时替换为
/// 该 primitive 的**模型可见名**。该工具不在模型可见面时，含此条款的整行提示词被删除。
pub struct ToolGuideline {
    /// 模板槽位键（`{{guideline:<key>}}`）。
    pub key: &'static str,
    /// owns 这条条款的工具 primitive 名。
    pub primitive: &'static str,
    /// 条款正文（不含行首项目符号前缀），允许 `{{tool}}` 占位。
    pub text: &'static str,
}

/// 工具自注册的提示词条款表：模板只放槽位，正文归工具侧。
pub const GUIDELINES: &[ToolGuideline] = &[
    ToolGuideline {
        key: "todo_task_planning",
        primitive: "todo_write",
        text: "For multi-step work, track steps with `{{tool}}` and keep statuses current.",
    },
    ToolGuideline {
        key: "goal_anchoring",
        primitive: "create_goal",
        text: "For multi-step, iterative, or long-running tasks, proactively anchor the session goal with `{{tool}}` even if the user did not say \"goal\". Skip for trivial single-turn questions.",
    },
    ToolGuideline {
        key: "goal_lifecycle",
        primitive: "update_goal",
        text: "Only mark complete via `{{tool}}` when the objective is actually achieved. Pause and resume require direct human intent. Mark blocked only when the same concrete blocker persists across at least 3 consecutive rounds.",
    },
    ToolGuideline {
        key: "todo_implementation_planning",
        primitive: "todo_write",
        text: "Before executing complex multi-step work, plan concrete steps using `{{tool}}`. Keep tasks updated as progress occurs.",
    },
];

/// 提示词里**看起来像工具名、但不是工具**的 token 白名单（PT4 反扫的豁免表）。
///
/// PT4 对提示词做工具名反扫，用的是形状启发式（长度 ≥5 且含下划线或内部大写）。
/// 该启发式必然命中少量非工具 token；把它们显式登记在这里，比放宽启发式更安全：
/// 新增一个「像工具名却解析不到」的 token 会让门禁变红，逼维护者确认它到底是
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

/// 提示词里**按名字引用**的模型可见工具（由模板占位符 + 条款表派生，无人工清单）。
///
/// `tests/prompt_tool_consistency.rs` 做双向机械校验：
/// 登记项必须存在于工具注册表（PT1）且曝光面为 `ModelVisible`（PT2），
/// 且必须真的出现在提示词文本里（PT3）；反向地，提示词中出现的长工具名必须已登记（PT4）。
pub fn prompt_tool_references() -> &'static [&'static str] {
    static REFERENCES: OnceLock<Vec<&'static str>> = OnceLock::new();
    REFERENCES
        .get_or_init(|| {
            let mut names: Vec<&'static str> = Vec::new();
            for guideline in GUIDELINES {
                names.push(resolve_model_name(guideline.primitive));
            }
            for template in [
                crate::agent::system_prompt_template::base_prompt_template(),
                crate::agent::system_prompt_template::team_policy_template(),
            ] {
                for primitive in tool_placeholders(template) {
                    names.push(resolve_model_name(primitive));
                }
            }
            names.sort_unstable();
            names.dedup();
            names
        })
        .as_slice()
}

/// 渲染一份稳定层模板：按行处理占位符，条款所属工具不可见时整行删除。
///
/// 逐行删除而不是整段替换，是为了保留模板的项目符号与行结构：
/// `  * Task planning: {{guideline:todo_task_planning}}` 在工具缺席时整行消失，
/// 而不是留下一个空项目符号。
pub fn render_system_prompt(template: &str) -> String {
    let mut rendered = String::with_capacity(template.len());
    for line in template.split_inclusive('\n') {
        if let Some(text) = render_line(line) {
            rendered.push_str(&text);
        }
    }
    rendered
}

/// 渲染单行；`None` 表示该行依赖的条款所属工具不在模型可见面，整行删除。
fn render_line(line: &str) -> Option<String> {
    let mut rendered = line.to_string();
    // 1. 条款槽位：整行绑定唯一工具，工具不可见即整行删除。
    if let Some(key) = placeholder_value(&rendered, "guideline:") {
        let guideline = GUIDELINES
            .iter()
            .find(|entry| entry.key == key)
            .unwrap_or_else(|| {
                panic!(
                    "提示词模板引用了未注册的工具条款 `{key}`：槽位必须在 tools::prompt_contract::GUIDELINES 里登记"
                )
            });
        if tool_exposure_for_name(guideline.primitive) != ToolExposure::ModelVisible {
            return None;
        }
        let body = guideline
            .text
            .replace("{{tool}}", resolve_model_name(guideline.primitive));
        rendered = rendered.replace(&format!("{{{{guideline:{key}}}}}"), &body);
    }
    // 2. 工具名占位符：只能是已注册工具，否则宁可炸也不要让模型看见半截模板。
    while let Some(primitive) = placeholder_value(&rendered, "tool:") {
        let name = resolve_model_name(primitive);
        rendered = rendered.replace(&format!("{{{{tool:{primitive}}}}}"), name);
    }
    assert!(
        !rendered.contains("{{"),
        "提示词模板渲染后仍残留占位符，说明模板语法写错：{rendered}"
    );
    Some(rendered)
}

/// 取出第一个 `<prefix><value>}}` 占位符的 value。
fn placeholder_value<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let start = text.find(&format!("{{{{{prefix}"))? + prefix.len() + 2;
    let end = text[start..].find("}}")? + start;
    Some(&text[start..end])
}

/// 扫描模板里全部 `{{tool:<primitive>}}` 占位符的 primitive 名。
fn tool_placeholders(template: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(index) = rest.find("{{tool:") {
        let after = &rest[index + "{{tool:".len()..];
        let Some(end) = after.find("}}") else { break };
        found.push(&after[..end]);
        rest = &after[end + 2..];
    }
    found
}

/// primitive 名 → 模型可见名；未注册直接炸（提示词模板与工具表失配属开发期错误）。
fn resolve_model_name(primitive: &str) -> &'static str {
    model_visible_tool_name_opt(primitive).unwrap_or_else(|| {
        panic!(
            "提示词引用了工具注册表中不存在的 primitive `{primitive}`：\
             提示词只能点名已注册工具（改名/删除后必须同步模板或条款表，canonical 名 = {:?}）",
            canonical_tool_name(primitive)
        )
    })
}

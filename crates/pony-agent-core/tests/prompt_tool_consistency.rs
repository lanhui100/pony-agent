//! 黑盒契约测试：提示词 ↔ 工具注册表一致性（Prompt/Tool Consistency）
//!
//! 背景：稳定层提示词按**产品名**点名工具（`TodoWrite` / `create_goal` / 团队工具 / `Run`），
//! 而工具注册表在 `agent::tools::names` 里独立维护模型名、别名与曝光面（`ToolExposure`）。
//! 两份事实源没有任何机械约束，改名、降曝光或删工具都会让提示词继续指挥模型调用
//! 一个不存在/不可见的工具——这是"提示词对模型撒谎"，且不会有任何现有测试变红。
//!
//! 断言 ID（PT1–PT6）：
//! - PT1：`prompt_tool_references()` 每项都必须能解析到注册表中的工具；
//! - PT2：登记项必须是模型可见面（`ModelVisible`），不得指向 Deferred/Internal；
//! - PT3：登记项必须真的出现在提示词文本里（防止登记表僵化成过期清单）；
//! - PT4：提示词文本里出现的工具名必须全部已登记（防止新增点名却漏登记）；
//! - PT5：冻结契约文档（`.dev-team/contracts/system-prompt-v2.md`）§2 全文与字节数与代码一致；
//! - PT6：稳定层**模板本体**不得出现任何工具专名（解耦只靠占位符，工具名由注册表渲染）。
//!
//! 读取入口固定为 `agent::context::{base_system_prompt, prompt_tool_references}`，
//! 禁止测试直接引用私有常量 `BASE_SYSTEM_PROMPT`。

use pony_agent_core::agent::context::{agent_team_policy_prompt, base_system_prompt};
use pony_agent_core::agent::tools::{
    canonical_tool_name, prompt_non_tool_tokens, prompt_tool_references, tool_exposure_for_name,
    ToolExposure,
};

/// 提示词全文（稳定层 BASE + 团队策略段）与团队段是否注入无关，
/// 这里校验的是"文本里写了什么"，不是"本轮注入了什么"。
fn prompt_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        ("BASE_SYSTEM_PROMPT", base_system_prompt()),
        ("AGENT_TEAM_POLICY_PROMPT", agent_team_policy_prompt()),
    ]
}

/// 把文本切成候选工具名 token：只保留 `[A-Za-z0-9_]`，其余字符（空白、标点、反引号）作分隔。
fn tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
        .collect()
}

/// "像工具名"的启发式：长度 ≥ 5 且含下划线或内部大写（`TodoWrite` / `create_goal`）。
///
/// 排除 `Run` / `Read` / `Write` 这类普通英文单词，避免把 `Read-before-write`
/// 里的 `Read` 误判成工具点名（那是散文，不是工具引用）。
fn looks_like_tool_name(token: &str) -> bool {
    token.len() >= 5
        && (token.contains('_')
            || token.chars().skip(1).any(|c| c.is_ascii_uppercase()))
}

/// PT1：登记表里的每个名字都必须能在工具注册表中解析。
#[test]
fn pt1_every_prompt_tool_reference_resolves_in_registry() {
    // Arrange
    let references = prompt_tool_references();

    // Act & Assert
    assert!(
        !references.is_empty(),
        "PT1 违约：prompt_tool_references() 为空。提示词确实点名了工具（TodoWrite/create_goal/update_goal/团队工具），空表会让 PT3/PT4 双双空转通过"
    );
    for name in references {
        assert!(
            canonical_tool_name(name).is_some(),
            "PT1 违约：提示词点名的工具 `{name}` 在工具注册表中不存在（已改名/已删除？）。\
             提示词对模型点名一个不存在的工具 = 指挥模型调用空气。请同步 tools::names 或修改提示词"
        );
    }
}

/// PT2：登记项必须是模型可见面，不得指向 Deferred / Internal。
#[test]
fn pt2_every_prompt_tool_reference_is_model_visible() {
    // Arrange
    let references = prompt_tool_references();

    // Act & Assert
    for name in references {
        let exposure = tool_exposure_for_name(name);
        assert_eq!(
            exposure,
            ToolExposure::ModelVisible,
            "PT2 违约：提示词点名的工具 `{name}` 曝光面为 {:?}。模型看不见的工具不得在稳定层被要求使用\
             （需要 tool_search 提升的工具只能由 ToolSearch 路径披露）",
            exposure
        );
    }
}

/// PT3：登记项必须真的出现在提示词文本里；否则登记表就是过期清单。
#[test]
fn pt3_every_registered_reference_actually_appears_in_prompt() {
    // Arrange
    let corpus = prompt_corpus();

    // Act & Assert
    for name in prompt_tool_references() {
        let found = corpus
            .iter()
            .any(|(_, text)| tokens(text).iter().any(|token| token == name));
        assert!(
            found,
            "PT3 违约：登记表声称提示词点名了 `{name}`，但 BASE 与团队策略段都未出现该 token。\
             工具已从提示词移除时应同步删除登记项，否则门禁保护的是一个不再存在的耦合"
        );
    }
}

/// PT4：提示词里出现的工具名必须全部对得上注册表（反向覆盖，fail-closed）。
///
/// 三种失效形态各有一条判定，**不接受"解析不到就放过"**：
/// 1. 能解析但未登记 → `unregistered`；
/// 2. 已登记却解析不到（登记名已被注册表删掉/改名，典型漂移）→ `dead_reference`；
/// 3. 解析不到且不在 `prompt_non_tool_tokens()` 白名单里 → `unresolvable`（点名了不存在的工具）。
///
/// 只做第 1 支是假安全：`TodoWrite` → `TodoWriteRenamed` 且提示词、登记表、契约三方
/// "自洽地"指向不存在的工具时，旧实现 5/5 全绿（独立验证者的 A′ 变异实证）。
/// 独立性：第 2/3 支让本断言单独也咬得住，不依赖 PT1/PT2 兜底——
/// 否则读者会误判防线位置（A′ 复验曾实测：只靠 PT1/PT2 拦截，PT4 因 `registered.contains`
/// 提前跳过而全绿）。
#[test]
fn pt4_prompt_mentions_no_unregistered_tool_names() {
    // Arrange
    let registered = prompt_tool_references();
    let non_tool = prompt_non_tool_tokens();
    let corpus = prompt_corpus();
    let mut unregistered: Vec<(String, &str)> = Vec::new();
    let mut dead_reference: Vec<(String, &str)> = Vec::new();
    let mut unresolvable: Vec<(String, &str)> = Vec::new();

    // Act
    for (source, text) in corpus {
        for token in tokens(text) {
            if !looks_like_tool_name(token) {
                continue;
            }
            let is_registered = registered.contains(&token);
            let resolves = canonical_tool_name(token).is_some();
            match (is_registered, resolves) {
                // 1. 注册表真有这个工具，但提示词点名却没登记。
                (false, true) => unregistered.push((source.to_string(), token)),
                // 2. 登记表声称有、注册表却解析不到：登记名已被删/改名。
                (true, false) => dead_reference.push((source.to_string(), token)),
                // 3. 谁都不认识的「像工具名」token：白名单外一律红。
                (false, false) if !non_tool.contains(&token) => {
                    unresolvable.push((source.to_string(), token))
                }
                _ => {}
            }
        }
    }

    // Assert
    assert!(
        unregistered.is_empty(),
        "PT4 违约：提示词点名了注册表中存在、但未登记的模型可见工具：{unregistered:?}。\
         请在 tools::prompt_contract::prompt_tool_references() 中登记，让 PT1/PT2 一并校验其存在性与曝光面"
    );
    assert!(
        dead_reference.is_empty(),
        "PT4 违约：提示词/登记表声称的工具在注册表中解析不到：{dead_reference:?}。\
         典型成因是工具改名后只改了登记表与提示词（三方「自洽」却指向不存在的工具）。\
         请修提示词与 tools::prompt_contract::prompt_tool_references() 指向真实模型名"
    );
    assert!(
        unresolvable.is_empty(),
        "PT4 违约：提示词里出现「像工具名却解析不到任何注册项」的 token：{unresolvable:?}。\
         三种可能：①工具已改名而提示词沿用旧名（典型漂移，必须修提示词）；\
         ②新增点名但漏登记（登记后由 PT1/PT2 校验）；③该 token 只是散文里的普通标识符\
         （请登记到 tools::prompt_contract::prompt_non_tool_tokens() 并注明理由）。\
         本断言 fail-closed：宁可要求一次显式确认，也不静默放过改名漂移"
    );
}

/// PT5：冻结契约文档必须与代码逐字一致（§2 全文 + 字节数）。
///
/// 契约文档是"承诺"的文本形态；代码悄悄改文案而不改契约，等于承诺与实现分家。
/// 这里读取契约文档，机械比对两件事：§2 ```text 代码块全文 == base_system_prompt()，
/// 以及表格里登记的字节数 == 实际字节数。
#[test]
fn pt5_frozen_contract_document_matches_the_prompt() {
    // Arrange
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.dev-team/contracts/system-prompt-v2.md"
    );
    let document = std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("PT5 读取契约文档失败：{path}（{error}）。契约文档是提示词的冻结真源，缺失即门禁失效")
    });
    let prompt = base_system_prompt();

    // Act 1：§2 的 ```text 代码块全文。
    let fenced = document
        .split_once("## 2. v2 提示词全文（逐字冻结）")
        .unwrap_or_else(|| {
            panic!("PT5 违约：契约文档缺少『## 2. v2 提示词全文（逐字冻结）』节，无法比对全文")
        })
        .1
        .split_once("```text")
        .unwrap_or_else(|| panic!("PT5 违约：契约文档 §2 缺少 ```text 冻结代码块"))
        .1
        .split_once("```")
        .unwrap_or_else(|| panic!("PT5 违约：契约文档 §2 的 ```text 代码块未闭合"))
        .0;
    // Act 2：表格里"全文 UTF-8 字节数"一行登记的字节数（必须是表格行，避免命中散文里的同名短语）。
    let bytes_row = document
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
        .find(|line| line.contains("UTF-8 字节数"))
        .unwrap_or_else(|| panic!("PT5 违约：契约文档 §6 缺少『UTF-8 字节数』登记行"));
    let recorded_bytes: usize = bytes_row
        .split("**")
        .filter_map(|segment| segment.trim().parse::<usize>().ok())
        .next()
        .unwrap_or_else(|| {
            panic!("PT5 违约：契约文档的字节数登记行不可解析：{bytes_row}（应形如 `| ... | **3790**（...） |`）")
        });

    // Assert
    // 代码块首尾各有一个换行是 Markdown 围栏的定界符，不参与逐字比对（字节数由下方独立断言）。
    assert_eq!(
        fenced.trim(),
        prompt.trim(),
        "PT5 违约：契约文档 §2 的冻结全文与 base_system_prompt() 不一致。\
         提示词文案属于契约面：改文案必须同提交修订 .dev-team/contracts/system-prompt-v2.md"
    );
    assert_eq!(
        recorded_bytes,
        prompt.len(),
        "PT5 违约：契约文档登记的字节数（{recorded_bytes}）与 base_system_prompt().len()（{}）不一致。\
         字节数是前缀预算与体量护栏的输入，登记值过期会让后续容量判断建立在错误事实上",
        prompt.len()
    );
}


/// 模板里去掉所有 `{{...}}` 占位符后的**字面文本**（占位符内的 primitive 名不算工具专名）。
fn template_literal_text(template: &str) -> String {
    let mut literal = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        literal.push_str(&rest[..open]);
        match rest[open + 2..].find("}}") {
            Some(close) => rest = &rest[open + 2 + close + 2..],
            None => return literal,
        }
    }
    literal.push_str(rest);
    literal
}

/// PT6：稳定层模板本体不得出现任何工具专名。
///
/// B138 的解耦形态是「模板放槽位、工具名由注册表渲染」。若有人图省事把工具名又写回模板，
/// 渲染照旧能跑、PT1–PT5 也照旧全绿（因为渲染结果没变），但解耦已经失效——这个名字又变成
/// 第二处需要人工同步的事实。所以必须直接检查模板原文，而不是渲染结果。
#[test]
fn pt6_prompt_templates_contain_no_tool_names() {
    // Arrange
    let templates = [
        (
            "base_prompt_template",
            pony_agent_core::agent::system_prompt_template::base_prompt_template(),
        ),
        (
            "team_policy_template",
            pony_agent_core::agent::system_prompt_template::team_policy_template(),
        ),
    ];
    let non_tool = prompt_non_tool_tokens();
    let mut leaked: Vec<(&'static str, String)> = Vec::new();

    // Act
    for (source, template) in templates {
        let literal = template_literal_text(template);
        for token in tokens(&literal) {
            if !looks_like_tool_name(token) || non_tool.contains(&token) {
                continue;
            }
            if canonical_tool_name(token).is_some() {
                leaked.push((source, token.to_string()));
            }
        }
    }

    // Assert
    assert!(
        leaked.is_empty(),
        "PT6 违约：提示词模板里出现了工具专名 {leaked:?}。\
         模板只允许 `{{{{tool:<primitive>}}}}` / `{{{{guideline:<key>}}}}` 占位符：\
         工具名必须由 tools::names 渲染（PT1/PT2 才能守住改名与曝光面）"
    );
    for (source, template) in templates {
        let rendered = if source == "base_prompt_template" {
            base_system_prompt()
        } else {
            agent_team_policy_prompt()
        };
        assert!(
            !rendered.contains("{{"),
            "PT6 违约：{source} 渲染后仍残留占位符，说明渲染器漏替换：{rendered}"
        );
        assert!(
            !template.contains("{{tool}}"),
            "PT6 违约：{source} 直接在模板里写了 `{{{{tool}}}}`（无 primitive 名），渲染器无法解析它"
        );
    }
}

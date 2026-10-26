//! W25「假能力收口」验收测试（红相锚定）。
//!
//! 背景（审计 G-02）：`subagent` / `workflow` / `spawn_teammate` / `list_agents` /
//! `send_message` / `interrupt_agent` / `drain_inbox` / `team_task_create` /
//! `team_task_update` / `team_task_list` 这 10 个工具的实现是合成字符串桩
//! （`agent/orchestration.rs` 的 `subagent` / `workflow` / `spawn_teammate`），
//! 经 `ToolRouter` 调用会返回 `status:"ok"` 的假回执，模型会误信已派生子代理并收到
//! 「完成」回执，实际什么都没发生。
//!
//! W25 契约（`docs/audit/agent-core-improvement-plan.md` §W25）：
//! - T1：模型可见工具面**双向**锁定——名字集合精确等于 21 个真实工具（既防假能力回流，
//!   也防真实工具被误摘）；
//! - T2：直接经 `ToolRouter` 调用必须返回 `status != "ok"` 且错误码
//!   `capability_unavailable`，message 非空且含可行动指引（指向 improvement-plan / W29）；
//! - T3：回归保护——真实工具 `TodoWrite` / `TodoList` 仍在模型可见面且可执行；
//! - T4：这 10 个 descriptor 的 `exposure` 不再是 `ModelVisible`（允许 Internal/Deferred）。
//!
//! 公开 API 说明（集成测试只能使用 crate 公开 API）：
//! `provider::anthropic_tools_payload` 是私有函数，无法直接调用。它的模型可见面来源是
//! `builtin_provider_contract_views()` → `builtin_tool_surface().provider_contract_views()`
//! → `TurnToolView::provider_contract_views`，而 `builtin_turn_tool_contract_views()` 是同一
//! 投影的公开入口。因此 T1 用这两个公开入口断言，与 Anthropic/OpenAI tools payload 同源。

use pony_agent_core::agent::tools::{
    builtin_tool_surface, builtin_tools, builtin_turn_tool_contract_views, ToolCall, ToolExposure,
    ToolRegistrySnapshot, ToolResult, ToolRouter,
};
use serde_json::{json, Value};

/// 10 个「假能力」工具名（审计 G-02 / W25 契约冻结名单）。
const FAKE_CAPABILITY_TOOLS: [&str; 10] = [
    "subagent",
    "workflow",
    "spawn_teammate",
    "list_agents",
    "send_message",
    "interrupt_agent",
    "drain_inbox",
    "team_task_create",
    "team_task_update",
    "team_task_list",
];

/// 生产工具面投影 `builtin_turn_tool_contract_views()` 的 29 个真实工具名
/// （== `builtin_tool_surface().provider_contract_views()` == Anthropic/OpenAI tools payload 数据源）。
///
/// 演进历史：
/// - W25 基线：21 项（彻底移除了 10 个假能力工具）。
/// - W37-C1 新增 7 项已真实实现的工具（Present, Find, GitDiff, JobStart, JobOutput, JobKill, JobList）；
/// - W42/PA-149 新增 1 项真实实现的工具（skill_load 按需加载技能正文）。
/// 总计 29 项真实生产工具，10 个假能力工具绝不回流。
const EXPECTED_MODEL_VISIBLE_TOOLS: [&str; 29] = [
    "Run",
    "Ask",
    "Read",
    "ReadDocument",
    "List",
    "Search",
    "Glob",
    "WebFetch",
    "WebSearch",
    "MCPResource",
    "ToolSearch",
    "Write",
    "Edit",
    "BatchExecute",
    "Plan",
    "ViewImage",
    "TodoWrite",
    "TodoList",
    "create_goal",
    "get_goal",
    "update_goal",
    "Present",
    "Find",
    "GitDiff",
    "JobStart",
    "JobOutput",
    "JobKill",
    "JobList",
    "skill_load",
];

fn make_call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments,
        plan: None,
    }
}

/// 模型可见工具面（公开等价入口：provider tools payload 的同一投影）。
fn model_visible_tool_names() -> Vec<String> {
    builtin_turn_tool_contract_views()
        .into_iter()
        .map(|view| view.name)
        .collect()
}

fn parse_output(result: &ToolResult) -> Option<Value> {
    serde_json::from_str::<Value>(&result.output).ok()
}

fn error_field<'a>(parsed: &'a Option<Value>, field: &str) -> Option<&'a str> {
    parsed
        .as_ref()
        .and_then(|value| value.pointer(&format!("/error/{field}")))
        .and_then(Value::as_str)
}

/// 「可行动指引」判定：必须指向本计划（improvement-plan）或 W29 真实实现波次。
fn has_actionable_guidance(message: &str, hint: &str) -> bool {
    let haystack = format!("{message}\n{hint}").to_lowercase();
    ["w29", "improvement-plan", "improvement_plan", "改进计划"]
        .iter()
        .any(|needle| haystack.contains(needle))
}

// ─────────────────────────────────────────────────────────────────────────────
// T1：模型可见工具面不得出现这 10 个假能力工具名
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t1_fake_capability_tools_are_absent_from_model_visible_surface() {
    // Arrange：原始内置目录（内部注册，仍保留给 W29 接管）+ 模型可见投影
    let raw_catalog: Vec<&str> = builtin_tools().iter().map(|tool| tool.name).collect();
    let visible = model_visible_tool_names();

    println!("[W25][T1] model_visible_tools = {visible:?}");

    // Arrange 断言：这 10 个工具必须仍作为内部注册存在（否则「摘除可见面」是假象，
    // 且 W29 无法接管真实实现；见 improvement-plan §W25「保留 handler 供 W29 接管」）。
    let unregistered: Vec<&str> = FAKE_CAPABILITY_TOOLS
        .iter()
        .copied()
        .filter(|name| !raw_catalog.contains(name))
        .collect();
    assert!(
        unregistered.is_empty(),
        "T1 Arrange 失败：以下假能力工具已从 builtin_tools() 内部目录消失，W29 将无 handler 可接管。\
         期望：10 个名字全部保留在内部目录；实际缺失：{unregistered:?}；当前内部目录共 {} 项",
        raw_catalog.len()
    );

    // Act：读取模型可见面（与 provider tools payload 同源）
    let actual: std::collections::BTreeSet<&str> =
        visible.iter().map(String::as_str).collect();
    let expected: std::collections::BTreeSet<&str> =
        EXPECTED_MODEL_VISIBLE_TOOLS.iter().copied().collect();

    // Assert 1（双向）：不得多（假能力回流），也不得少（真实工具被误摘）
    let leaked: Vec<&&str> = actual.difference(&expected).collect();
    let missing: Vec<&&str> = expected.difference(&actual).collect();
    assert!(
        leaked.is_empty() && missing.is_empty(),
        "T1 失败：模型可见工具面与冻结的 21 名清单不一致。\n\
         期望（{} 项）：{expected:?}\n\
         实际（{} 项）：{actual:?}\n\
         多出的（假能力回流 / 未登记工具）：{leaked:?}\n\
         缺失的（真实工具被误摘）：{missing:?}",
        expected.len(),
        actual.len()
    );

    // Assert 2（显式二次确认）：10 个假能力名字逐个不得出现，失败信息直指审计 G-02 名单
    let fake_leaks: Vec<&str> = FAKE_CAPABILITY_TOOLS
        .iter()
        .copied()
        .filter(|name| actual.contains(name))
        .collect();
    assert!(
        fake_leaks.is_empty(),
        "T1 失败：模型可见工具面仍包含假能力工具。\
         期望：{FAKE_CAPABILITY_TOOLS:?} 全部不出现在模型可见面；\
         实际泄漏：{fake_leaks:?}；实际模型可见面（{} 项）：{actual:?}",
        actual.len()
    );

    // Assert 3（交叉校验）：ToolSurface 公开投影必须与上面的可见面完全一致，
    // 防止未来某处绕过 TurnToolView 直接把 raw builtin_tools() 喂给 provider。
    let surface_names: std::collections::BTreeSet<String> = builtin_tool_surface()
        .provider_contract_views()
        .expect("builtin tool surface provider_contract_views 必须可投影")
        .into_iter()
        .map(|view| view.name)
        .collect();
    assert_eq!(
        surface_names,
        visible.iter().cloned().collect::<std::collections::BTreeSet<_>>(),
        "T1 交叉校验失败：builtin_tool_surface().provider_contract_views() 与 \
         builtin_turn_tool_contract_views() 投影不一致（应为同一 TurnToolView 投影）。\
         期望：{visible:?}；实际：{surface_names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T2：直接经 ToolRouter 调用必须返回 capability_unavailable，禁止 ok 假回执
//
// 因果（本次红相的真正缺口）：
// T1 已证明「模型看不到这 10 个工具」，但**看不到 ≠ 不会被调用**。模型仍可能通过
// ①幻觉出工具名、②历史 transcript/被注入的提示词里出现旧工具名、③非模型入口
// （host/子代理/测试）按名字硬调，把 `subagent` / `team_task_*` 等名字直接交给
// `ToolRouter::execute`。当前 ToolRouter 对这些名字走的是真实 handler 分支，返回
// `status:"ok"` + 合成回执（orchestration.rs 的 `subagent` / `workflow` / `spawn_teammate`），
// 模型会真诚相信「已派生子代理并收到完成回执」，实际什么都没发生 —— 这比能力缺失更危险。
// 因此 W25 要求：无论调用方是谁、参数是否合法，这 10 个 handler 都必须返回结构化
// `capability_unavailable`（并指向 improvement-plan / W29），绝不再返回 ok 假回执。
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t2_fake_capability_tools_return_capability_unavailable_error() {
    // Arrange：构造可复现的调用参数；先建立 team/task 状态，确保每个工具都能走到
    // 其「原本的 happy path」，从而证明当前返回的是假成功而不是参数错误。
    let router = ToolRouter::new();
    let unique = format!("w25-probe-{}", std::process::id());

    let _ = router.execute(&make_call(
        "spawn_teammate",
        json!({
            "name": unique,
            "role": "probe",
            "description": "W25 红相探针队友",
            "prompt": "probe"
        }),
    ));

    let created = router.execute(&make_call(
        "team_task_create",
        json!({
            "subject": "W25 probe task",
            "description": "W25 红相探针任务"
        }),
    ));
    let probe_task_id = parse_output(&created)
        .as_ref()
        .and_then(|value| value.pointer("/result/task_id"))
        .and_then(Value::as_str)
        .unwrap_or("task-w25-probe-missing")
        .to_string();

    let cases: Vec<(&str, Value)> = vec![
        (
            "subagent",
            json!({
                "prompt": "Inspect auth tokens in workspace",
                "description": "W25 probe subagent",
                "run_in_background": false
            }),
        ),
        (
            "workflow",
            json!({
                "name": "w25-probe-workflow",
                "steps": [{ "id": "step-1", "action": "noop", "depends_on": [] }]
            }),
        ),
        (
            "spawn_teammate",
            json!({
                "name": unique,
                "role": "probe",
                "description": "W25 红相探针队友",
                "prompt": "probe"
            }),
        ),
        ("list_agents", json!({})),
        (
            "send_message",
            json!({
                "target": unique,
                "message": "Please review this pull request",
                "mode": "queue"
            }),
        ),
        (
            "interrupt_agent",
            json!({ "target": unique, "reason": "W25 probe interrupt" }),
        ),
        ("drain_inbox", json!({ "target": unique })),
        (
            "team_task_create",
            json!({
                "subject": "W25 probe task",
                "description": "W25 红相探针任务"
            }),
        ),
        (
            "team_task_update",
            json!({
                "task_id": probe_task_id,
                "expected_revision": 1,
                "action": "claim",
                "owner": "lead"
            }),
        ),
        ("team_task_list", json!({})),
    ];

    // Act + Assert：逐个工具断言结构化错误契约，聚合全部违规再一次性失败，
    // 便于实施者一次看到 10 条真实差异（而不是第一条就中断）。
    let mut violations: Vec<String> = Vec::new();

    for (name, arguments) in cases {
        let result = router.execute(&make_call(name, arguments));
        let parsed = parse_output(&result);
        let code = error_field(&parsed, "code");
        let message = error_field(&parsed, "message").unwrap_or("");
        let hint = error_field(&parsed, "hint").unwrap_or("");

        if result.status == "ok" {
            violations.push(format!(
                "`{name}`: status=\"ok\"（期望 status!=\"ok\" 且 code=capability_unavailable）—— \
                 实际返回了假成功回执，模型会误信能力已执行。output={}",
                preview(&result.output)
            ));
            continue;
        }

        if code != Some("capability_unavailable") {
            violations.push(format!(
                "`{name}`: status=\"{}\" 但 error.code={code:?}（期望 \"capability_unavailable\"）。output={}",
                result.status,
                preview(&result.output)
            ));
            continue;
        }

        if message.trim().is_empty() {
            violations.push(format!(
                "`{name}`: error.message 为空（期望非空且含指向 improvement-plan / W29 的可行动指引）。\
                 output={}",
                preview(&result.output)
            ));
            continue;
        }

        if !has_actionable_guidance(message, hint) {
            violations.push(format!(
                "`{name}`: error.message/hint 缺少可行动指引（期望包含 \"W29\" 或 \
                 \"improvement-plan\" 等指向 docs/audit/agent-core-improvement-plan.md 的线索）。\
                 实际 message=\"{message}\"，hint=\"{hint}\""
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "T2 失败：{} 个假能力工具未按 W25 契约返回结构化 `capability_unavailable` 错误。\n\
         期望：status!=\"ok\" + error.code=\"capability_unavailable\" + 非空可行动 message。\n\
         实际违规：\n- {}",
        violations.len(),
        violations.join("\n- ")
    );
}

fn preview(output: &str) -> String {
    const LIMIT: usize = 240;
    if output.chars().count() <= LIMIT {
        output.to_string()
    } else {
        let head: String = output.chars().take(LIMIT).collect();
        format!("{head}…(truncated, total {} chars)", output.chars().count())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T3：回归保护——真实工具 TodoWrite / TodoList 必须仍在模型可见面且可执行
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t3_real_todo_tools_remain_model_visible_and_executable() {
    // Arrange
    let visible = model_visible_tool_names();
    let router = ToolRouter::new();

    // Assert 1：可见面必须包含两个真实 todo 工具（防止 W25 误摘真实工具）
    for expected in ["TodoWrite", "TodoList"] {
        assert!(
            visible.iter().any(|name| name == expected),
            "T3 失败：真实工具 `{expected}` 从模型可见面消失。\
             期望：`{expected}` 出现在模型可见面；实际可见面（{} 项）：{visible:?}",
            visible.len()
        );
    }

    // Act：真实工具必须仍可执行（不是只剩名字的空壳）
    let result = router.execute(&make_call("todo_list", json!({})));

    // Assert 2：真实工具执行返回 ok
    assert_eq!(
        result.status, "ok",
        "T3 失败：真实工具 `todo_list` 执行失败，期望 status=\"ok\"。实际 status=\"{}\"，output={}",
        result.status,
        preview(&result.output)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T4：这 10 个工具的 descriptor exposure 不得再是 ModelVisible
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t4_fake_capability_descriptors_are_not_model_visible() {
    // Arrange
    let registry =
        ToolRegistrySnapshot::builtin().expect("builtin registry snapshot 必须可构建");

    // Act + Assert：descriptor 必须存在（保留给 W29），且 exposure != ModelVisible
    let mut violations: Vec<String> = Vec::new();

    for name in FAKE_CAPABILITY_TOOLS {
        match registry.resolve(name) {
            None => violations.push(format!(
                "`{name}`: descriptor 缺失（期望存在且 exposure 为 Internal/Deferred，\
                 以便 W29 接管真实实现）"
            )),
            Some(descriptor) => {
                if descriptor.exposure == ToolExposure::ModelVisible {
                    violations.push(format!(
                        "`{name}`: descriptor exposure=ModelVisible（期望 Internal 或 Deferred），\
                         descriptor_id=`{}`",
                        descriptor.identity.descriptor_id
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "T4 失败：{} 个假能力工具 descriptor 仍被标记为模型可见。\
         期望：exposure ∈ {{Internal, Deferred}}。\n实际违规：\n- {}",
        violations.len(),
        violations.join("\n- ")
    );
}

//! Wave-61 团队激活红相验收测试（冻结测试；Executor 只读）。
//!
//! 契约真值源：`.dev-team/contracts/wave-61-team-activation.md`（**v1.2 演进**，Lead 裁决）
//! - C1（C1-A1/A2/A3）：模型可见工具面 29 → **38**，其中 9 项团队工具 `ModelVisible`；
//!   `workflow` 因 handler 是桩（`orchestration.rs:286-297` 只回显 step.id 并返回 `status:"success"`）
//!   退回 `Internal`，本文件对其保留**反向守卫**（`c1_workflow_stub_is_not_model_visible`）。
//!   v1.2 前的 39 项清单已作废：旧断言 → 新断言 = `len()==39` 且含 `workflow` → `len()==38` 且不含 `workflow`。
//! - C2（C2-A3/A4）：`AgentRuntimeBuilder::build()`（`runtime/builder.rs:79`）注册 `HostTurnEngine`，
//!   构建过 runtime 的进程内 `subagent_runtime_wired() == true`
//! - C3（C3-A1/A2 + v1.2 文本口径）：已接线 → System 消息数 == 2 且第 2 条以
//!   `Agent Teams is available in this session` 开头，且策略段文本必须含本仓真实口径
//!   `Run` / `cas_conflict`、不得含参考实现口径 `Bash` / `FS_STALE_VERSION`；
//!   未接线 → System 数 == 1 且全文无该锚串
//! - C5（C5-A1/A3）：工具可见性与策略段同源于 `subagent_runtime_wired()`
//!
//! 分级：L3（跨模块集成 + 进程级 OnceLock 单例 + 源码级同源证据）。
//!
//! ## 进程隔离理由（C3-A1/A2 的硬约束）
//! `subagent_runtime::SUBAGENT_ENGINE` 是进程级 `OnceLock`（`subagent_runtime.rs:37-45`），
//! **首写者胜、不可复位**。同一测试进程内既有"已接线"用例又有"未接线"用例时，后者的前置条件
//! 会随用例执行顺序被前者污染（`cargo test` 默认多线程并发执行）。因此 C3 的已接线/未接线两条
//! 断言必须落在**独立进程**：由 [`c3_team_policy_system_message_present_when_wired`] /
//! [`c3_team_policy_absent_when_unwired`] 用 `std::process::Command` 递归调用本测试二进制的
//! [`probe_child_env`] 入口，经环境变量 `PONY_TEAM_POLICY_PROBE=wired|unwired` 分派，
//! 父进程只断言子进程 exit code 与 stdout 中的 `TEAM_POLICY=present|absent` 标记。
//! **没有任何断言被跳过或降级为 println**：真正的断言全部在子进程内执行，失败即非零退出。
//!
//! ## 公开入口说明（不自造 API）
//! C3 的消息组装入口使用既有公开入口 `DefaultTurnContextBuilder`（实现 `TurnContextBuilder`
//! trait，`context.rs:286-320`）的 `build_request(...)`，其返回值 `ProviderRequest.input` 即
//! `flatten_layered_input_messages`（`context.rs:634`）产出的消息序列；未使用任何
//! `pub(crate)` 私有函数。
//!
//! ## `model_visible_tool_name` 与产品名的口径（契约 C1.1 + C1.6）
//! - C1.1 禁止的是给这 10 项起 **CamelCase 别名**（`builtin_aliases_for_primitive()` 保持 `vec![同名]`），
//!   **不禁止** `model_visible_tool_name_opt()` 的**小写原名透传**映射。
//! - C1.6（契约 v1.1，第 146 行起）反过来**要求**为这 10 项登记「小写原名透传」：`model_visible_tool_name()`
//!   （`tools.rs:7257-7261`）对未登记名回落字面量 `"Run"`，而 `telemetry.rs` 的 tool activity 记录点
//!   （`:306`/`:332`/`:508`/`:576`）正用 `model_visible_tool_name(&tool.name)` 生成 trace 里的 tool `name`
//!   ——不登记会让团队工具在 trace 里被记成 `Run`，污染本波赖以定因的取证面。
//! - 因此本文件同时断言 C1-A6（`model_visible_tool_name(name) == name`）与 C1-A7
//!   （`product_visible_tool_name(name) == name`），并额外断言 39 项清单中 `Run` 只出现一次（防产品名塌缩）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once};
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    base_system_prompt, prompt_layers_are_ordered, DefaultTurnContextBuilder, LongTermMemory,
    RetrievedContextState, RunState, SessionContext, TranscriptContext, TurnContext,
    TurnContextBuilder,
};
use pony_agent_core::agent::orchestration::{
    self, MessageDeliveryMode, ReadInboxArgs, SendMessageArgs, SpawnTeammateArgs, TeammateStatus,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderMessage, ProviderProtocol, ProviderRole,
};
use pony_agent_core::agent::runtime::AgentRuntimeBuilder;
use pony_agent_core::agent::runtime_helper::TestRuntimeGuard;
use pony_agent_core::agent::session::{EnvironmentInfo, FileSessionBackend, SessionStore};
use pony_agent_core::agent::subagent_runtime::{
    set_subagent_engine, subagent_engine, HostTurnEngine, SubagentRunOutcome, SubagentTurnEngine,
};
use pony_agent_core::agent::tools::{
    builtin_tool_surface, builtin_turn_tool_contract_views, model_visible_tool_name,
    product_visible_tool_name, subagent_runtime_wired, tool_exposure_for_name, ToolExposure,
};

// ============================================================
// 冻结常量（逐字取自契约 C1.1 / C1.2 / C3.4）
// ============================================================

/// 契约 v1.2（Lead 裁决）：模型面团队工具 9 项 primitive 名（product 名 == primitive 名，小写原形）。
///
/// `workflow` **不在**此列：其 handler（`orchestration.rs:286-297`）只回显 step.id 并返回
/// `status:"success"` 的桩，暴露即伪造成功（L3-P BLOCKING），故退回 `Internal`；
/// 恢复条件＝真实 DAG 引擎落地（真实执行 + 真实错误码，不再回显合成 success）。
const TEAM_TOOLS: [&str; 9] = [
    "subagent",
    "spawn_teammate",
    "list_agents",
    "send_message",
    "interrupt_agent",
    "drain_inbox",
    "team_task_create",
    "team_task_update",
    "team_task_list",
];

/// 契约 C3.4：策略段标题行（机器判定锚串，与 DSH `:36` 同一锚串）。
const TEAM_POLICY_ANCHOR: &str = "Agent Teams is available in this session";

/// 契约 v1.2 C1.2：38 项模型可见面精确清单（含顺序；`workflow` 退回 Internal 后不在其中）。
///
/// 顺序真值源：registry 排序键 `(!visible, product_first_index)`（`tools.rs:555-566`）
/// ＋ `builtin_tools()` 定义序（10 项团队工具注册于 `tools.rs:5925-6138`，夹在 `view_image`
/// `:5894` 与 `todo_write` `:6147` 之间），故 10 项插入 `ViewImage` 之后、`TodoWrite` 之前。
const EXPECTED_38: [&str; 38] = [
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
    "subagent",
    "spawn_teammate",
    "list_agents",
    "send_message",
    "interrupt_agent",
    "drain_inbox",
    "team_task_create",
    "team_task_update",
    "team_task_list",
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

// ============================================================
// C1：工具面 39 项（C1-A1 / C1-A2 / C1-A3）
// ============================================================

/// C1-A1（C1.2 精确清单）+ C1-A2（10 项逐个可见）的集合侧断言。
#[test]
fn c1_team_tools_are_model_visible() {
    // Act：读取模型可见面（== provider tools payload 同一投影）。
    let views = builtin_turn_tool_contract_views();
    let names: Vec<&str> = views.iter().map(|view| view.name.as_str()).collect();

    // Assert 1：总数必须为 38（C1-A1，契约 v1.2：29 既有 + 9 团队，`workflow` 退回 Internal）。
    assert_eq!(
        names.len(),
        38,
        "C1-A1 失败：模型可见工具面必须为 38 项（29 既有 + 9 团队；`workflow` v1.2 退回 Internal）。\
         实际 {} 项：{names:?}",
        names.len()
    );

    // Assert 1b（契约 v1.2 反向守卫）：`workflow` 的 handler 是只回显 step.id 并返回
    // `status:"success"` 的桩（orchestration.rs:286-297），暴露即伪造成功，**不得**进模型面。
    assert!(
        !names.contains(&"workflow"),
        "契约 v1.2 失败：`workflow` 桩 handler 不得进模型面（暴露即伪造成功；恢复条件＝真实 DAG 引擎）"
    );

    // Assert 2：9 个团队工具逐个必须出现（C1-A2 的集合侧；失败信息逐名可见）。
    let missing: Vec<&&str> = TEAM_TOOLS
        .iter()
        .filter(|name| !names.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "C1-A2 失败：以下团队工具未进入模型可见面（当前落 tools.rs:7381 `_ => Internal`）：{missing:?}；\
         实际可见面（{} 项）：{names:?}",
        names.len()
    );

    // Assert 3：逐字、逐序等于 C1.2 的 38 行清单（含顺序）。
    let expected: Vec<&str> = EXPECTED_38.to_vec();
    assert_eq!(
        names, expected,
        "C1-A1 失败：模型可见面与契约 v1.2 C1.2 的 38 项精确清单（含顺序）不一致"
    );

    // Assert 4：产品名不得塌缩——`Run` 在 38 项中必须唯一（`time_now`/`run_command` 共用槽位，
    // 若团队工具误落 `unwrap_or("Run")` 兜底会产生重复 Run）。
    let run_slots = names.iter().filter(|name| **name == "Run").count();
    assert_eq!(
        run_slots, 1,
        "C1 失败：`Run` 槽位必须唯一（产品名塌缩会把团队工具误标成 Run）；实际出现 {run_slots} 次"
    );

    // Assert 5：视图的 exposure 字段对 9 项团队工具必须为 model_visible（C4.3/T4 同源口径）。
    let violations: Vec<String> = views
        .iter()
        .filter(|view| TEAM_TOOLS.contains(&view.name.as_str()))
        .filter(|view| view.exposure != "model_visible")
        .map(|view| format!("{} exposure={}", view.name, view.exposure))
        .collect();
    assert!(
        violations.is_empty(),
        "C1 失败：团队工具 contract view 的 exposure 必须为 model_visible，实际违规：{violations:?}"
    );
}

/// 契约 v1.2（Lead 裁决，L3-P BLOCKING）：`workflow` 必须退回 `Internal`，不得进模型面。
///
/// 真值源：`crates/pony-agent-core/src/agent/orchestration.rs:286-297` —— `workflow` handler 是桩：
/// 只回显 `step.id` 并返回 `status:"success"`，没有任何真实 DAG 执行与真实错误码，模型面暴露即
/// 构成"伪造成功"（比能力缺失更危险）。
/// 恢复条件：真实 DAG 引擎落地（真实调度、真实失败传播、不再回显合成 success）后，须走新契约
/// （本文件 successor）重新登记，方可移出本反向守卫。
#[test]
fn c1_workflow_stub_is_not_model_visible() {
    // Assert 1：descriptor 的 exposure 必须为 Internal。
    let exposure = tool_exposure_for_name("workflow");
    assert_eq!(
        exposure,
        ToolExposure::Internal,
        "契约 v1.2 失败：`workflow`（orchestration.rs:286-297 桩 handler）必须为 ToolExposure::Internal，\
         实际 {exposure:?}；暴露桩实现会让模型误信 DAG 已真实执行"
    );

    // Assert 2：contract views 里不得出现 `workflow`（即使 exposure 改了也可能被白名单漏出）。
    let names: Vec<String> = builtin_turn_tool_contract_views()
        .into_iter()
        .map(|view| view.name)
        .collect();
    assert!(
        !names.iter().any(|name| name == "workflow"),
        "契约 v1.2 失败：`workflow` 出现在模型可见面（实际 {} 项：{names:?}）",
        names.len()
    );
}

/// 契约 C1-A6（`model_visible_tool_name` 不得回落 `Run`）+ C1-A7（产品名原样透传）
/// + C1-A3（provider 投影与 turn view 交叉一致）。
#[test]
fn c1_product_names_do_not_fall_back_to_run() {
    // Assert 1（C1.6 / C1-A6）：9 项（不含已退回 Internal 的 `workflow`）必须登记小写原名透传，
    // 否则 telemetry 记录点会把团队工具记成 `Run`，污染 trace 取证面。
    let mut fallback_violations: Vec<String> = Vec::new();
    for name in TEAM_TOOLS {
        let telemetry_name = model_visible_tool_name(name);
        if telemetry_name != name {
            fallback_violations.push(format!(
                "`{name}` → telemetry/trace 名 `{telemetry_name}`（期望小写原名，禁止回落 `Run`）"
            ));
        }
    }
    assert!(
        fallback_violations.is_empty(),
        "C1-A6 失败：`model_visible_tool_name()`（tools.rs:7257-7261）对未登记的团队工具回落 `unwrap_or(\"Run\")`。\
         契约 C1.6 要求 `model_visible_tool_name_opt()`（:7216-7252）为这 10 项登记小写原名透传\
         （与 `TOOL_SKILL_LOAD => TOOL_SKILL_LOAD` 同构），否则 telemetry.rs:306/332/508/576 会把团队工具记成 `Run`。\
         实际违规：{fallback_violations:?}"
    );

    // Assert 2（C1.1 / C1-A7）：9 项 provider 面 product 名必须原样透传小写原名。
    let mut violations: Vec<String> = Vec::new();
    for name in TEAM_TOOLS {
        let product = product_visible_tool_name(name);
        if product != name {
            violations.push(format!("`{name}` → product 名 `{product}`（期望原样透传）"));
        }
    }
    assert!(
        violations.is_empty(),
        "C1-A7 失败：团队工具产品名不得被映射/回落（尤其不得回落 `Run`），实际违规：{violations:?}"
    );

    // Assert 2（C1-A3）：两条公开投影必须给出完全一致的名字集合。
    let turn_names: Vec<String> = builtin_turn_tool_contract_views()
        .into_iter()
        .map(|view| view.name)
        .collect();
    let surface_names: Vec<String> = builtin_tool_surface()
        .provider_contract_views()
        .expect("builtin tool surface provider_contract_views 必须可投影")
        .into_iter()
        .map(|view| view.name)
        .collect();
    assert_eq!(
        turn_names, surface_names,
        "C1-A3 失败：builtin_tool_surface().provider_contract_views() 与 \
         builtin_turn_tool_contract_views() 必须为同一 TurnToolView 投影（含顺序）"
    );
}

// ============================================================
// C2：宿主引擎接线（C2-A3 / C2-A4）
// ============================================================

/// C2-A3 + C2-A4：`AgentRuntimeBuilder::build()` 必须在 `builder.rs` 注册宿主引擎。
///
/// 断言：连续两次 `build()` 不 panic（`build()` 返回 `AgentRuntime` 而非 `Result`，注册失败
/// 不得改变返回值/不得阻断启动）；构建过 runtime 后 `subagent_runtime_wired() == true`。
#[test]
fn c2_builder_registers_subagent_engine() {
    sandbox_env();
    let _rt = TestRuntimeGuard::new();

    // Act：连续两次构建（第二次注册必然返回 Err 且必须被忽略——OnceLock 首写者胜）。
    let first = AgentRuntimeBuilder::new()
        .session_store(sandboxed_session_store("c2-build-1"))
        .build();
    let second = AgentRuntimeBuilder::new()
        .session_store(sandboxed_session_store("c2-build-2"))
        .build();
    drop(second);
    drop(first);

    // Assert（C2-A4）：构建过 runtime 的进程内必须已接线。
    assert!(
        subagent_runtime_wired(),
        "C2-A3/A4 失败：`AgentRuntimeBuilder::build()`（runtime/builder.rs:79）未注册 \
         HostTurnEngine，故构建后 `subagent_runtime_wired()` 仍为 false —— 生产路径上 10 项团队工具\
         会全部 fail-closed（tools.rs:1571-1573 等 10 处 capability_unavailable）"
    );
    assert!(
        subagent_engine().is_some(),
        "C2-A4 失败：`subagent_runtime_wired()==true` 但引擎槽为空（同源判定被绕过）"
    );
}

// ============================================================
// C3：团队策略提示的接线门控（C3-A1 / C3-A2）——进程隔离
// ============================================================

/// 子进程入口：父进程用例通过 `PONY_TEAM_POLICY_PROBE=wired|unwired` 分派到本入口执行真实断言。
///
/// 说明（如实登记，非"跳过"）：未设置该环境变量时本入口不做断言、打印一行审计说明后返回，
/// 它是**子进程入口**而非独立断言用例；三条 C3/C5 断言的执行责任在父进程用例上，
/// 父进程断言子进程 exit code == 0 且 stdout 含 `TEAM_POLICY=present|absent`。
#[test]
fn probe_child_env() {
    let Ok(mode) = std::env::var("PONY_TEAM_POLICY_PROBE") else {
        println!(
            "[wave61] probe_child_env: 未设置 PONY_TEAM_POLICY_PROBE（父进程用例 c3_* 会以子进程方式调用本入口并断言）"
        );
        return;
    };

    match mode.as_str() {
        "wired" => {
            // Arrange：显式把引擎槽置为已接线（公开 API；不经 builder，以隔离 C2 的验证责任）。
            wire_engine_public_api();
            assert!(
                subagent_runtime_wired(),
                "C3-A1 前置失败：置入引擎后 subagent_runtime_wired() 仍为 false"
            );

            // Act：走既有公开组装入口产出消息序列。
            let messages = assemble_messages("child-wired");

            // Assert 1：System 消息数 == 2。
            let systems = system_messages(&messages);
            assert_eq!(
                systems.len(),
                2,
                "C3-A1 失败：已接线路径的 System 消息数必须为 2（BASE + 团队策略段），实际 {} 条；\
                 全体角色序列：{:?}",
                systems.len(),
                messages.iter().map(role_tag).collect::<Vec<_>>()
            );

            // Assert 2：第 2 条 System 必须以契约锚串开头。
            assert!(
                systems[1].content.starts_with(TEAM_POLICY_ANCHOR),
                "C3-A1 失败：第 2 条 System 消息必须以 `{TEAM_POLICY_ANCHOR}` 开头，实际开头：{:?}",
                systems[1].content.chars().take(120).collect::<String>()
            );

            // Assert 3：层序不变量必须保持（System 全在 Developer 之前）。
            assert!(
                prompt_layers_are_ordered(&messages),
                "C3-A1 失败：prompt_layers_are_ordered 被破坏（动态层插到了稳定层之前，会打穿前缀缓存）"
            );

            // Assert 4：首条消息必须仍是 BASE_SYSTEM_PROMPT（逐字节）。
            assert_eq!(
                messages[0].content,
                base_system_prompt(),
                "C3-A1 失败：首条消息必须逐字节等于 base_system_prompt()（禁止改写 BASE 本体）"
            );

            // Assert 5（契约 v1.2 文本口径，机器判定）：策略段必须使用**本仓真实**工具名与错误码，
            // 不得残留参考实现（DSH）的 `Bash` / `FS_STALE_VERSION` 口径 —— 后者在本仓不存在，
            // 会诱导模型调用不存在的工具名或臆造错误码。
            let policy_text = &systems[1].content;
            assert!(
                policy_text.contains("Run"),
                "契约 v1.2 失败：策略段必须提及本仓真实执行工具 `Run`（不得用参考实现的 `Bash`）。\
                 实际策略段：{:?}",
                policy_text.chars().take(200).collect::<String>()
            );
            assert!(
                policy_text.contains("cas_conflict"),
                "契约 v1.2 失败：策略段必须提及本仓真实冲突错误码 `cas_conflict`（不得用 `FS_STALE_VERSION`）。\
                 实际策略段：{:?}",
                policy_text.chars().take(200).collect::<String>()
            );
            assert!(
                !policy_text.contains("Bash"),
                "契约 v1.2 失败：策略段不得包含参考实现的工具名 `Bash`（本仓无该工具，会诱导模型调用不存在的能力）"
            );
            assert!(
                !policy_text.contains("FS_STALE_VERSION"),
                "契约 v1.2 失败：策略段不得包含参考实现的错误码 `FS_STALE_VERSION`（本仓真实错误码是 `cas_conflict`）"
            );

            println!("TEAM_POLICY=present");
        }
        "unwired" => {
            // Arrange：本进程从未构建 runtime / 未注册引擎，槽必为空。
            assert!(
                subagent_engine().is_none(),
                "C3-A2 前置失败：隔离子进程内引擎槽已非空（进程隔离失效，断言不可信）"
            );
            assert!(
                !subagent_runtime_wired(),
                "C3-A2 前置失败：未接线进程内 wired 必须为 false"
            );

            // Act + Assert 1：System 消息数 == 1。
            let messages = assemble_messages("child-unwired");
            let systems = system_messages(&messages);
            assert_eq!(
                systems.len(),
                1,
                "C3-A2 失败：未接线路径的 System 消息数必须为 1，实际 {} 条；\
                 全体角色序列：{:?}",
                systems.len(),
                messages.iter().map(role_tag).collect::<Vec<_>>()
            );

            // Assert 2：全体消息文本中不得存在策略段锚串（连空段/占位段也不允许）。
            let offenders: Vec<String> = messages
                .iter()
                .filter(|message| message.content.contains(TEAM_POLICY_ANCHOR))
                .map(|message| role_tag(message).to_string())
                .collect();
            assert!(
                offenders.is_empty(),
                "C3-A2 失败：未接线路径出现了团队策略段锚串（角色：{offenders:?}）—— \
                 未接线宿主的提示面必须逐字节不变"
            );

            println!("TEAM_POLICY=absent");
        }
        other => panic!("未知 PONY_TEAM_POLICY_PROBE={other}（只允许 wired|unwired）"),
    }
}

/// v1.3 子进程入口：父用例 `c7_*`（后续 `c8_*`）通过 `PONY_WAVE61_V13_CHILD=c7|c8` 分派，
/// 在**全新进程**内先注入计数 Stub 引擎再执行真实断言（OnceLock 首写者胜，父进程已被 C2 占用）。
///
/// 与 [`probe_child_env`] 同构：未设置环境变量时本入口只打印审计说明并返回（它是子进程入口，
/// 三条断言的执行责任在父用例；父用例断言子进程 exit code == 0 且 stdout 含 `C7_OK`/`C8_OK`）。
#[test]
fn v13_child_env() {
    let Ok(mode) = std::env::var(V13_CHILD_ENV) else {
        println!("[wave61] v13_child_env: 未设置 {V13_CHILD_ENV}（父用例 c7_*/c8_* 会以子进程方式调用本入口并断言）");
        return;
    };
    match mode.as_str() {
        "c7" => run_c7_body(),
        "c7b" => run_c7b_body(),
        "c7c" => run_c7c_body(),
        "c8" => run_c8_body(),
        other => panic!("未知 {V13_CHILD_ENV}={other}（只允许 c7|c7b|c7c|c8）"),
    }
}

/// C3-A1：已接线路径必须注入策略段（红相：当前 System 数恒为 1，锚串缺席）。
#[test]
fn c3_team_policy_system_message_present_when_wired() {
    let outcome = run_child_probe("wired");
    assert!(
        outcome.status.success(),
        "C3-A1 失败（红相）：已接线路径未注入团队策略段。子进程 Exit={:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        outcome.status.code(),
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("TEAM_POLICY=present"),
        "C3-A1 失败：子进程成功退出但未输出 `TEAM_POLICY=present` 标记\nstdout:\n{}",
        outcome.stdout
    );
}

/// C3-A2：未接线路径必须完全不注入（该断言在当前实现下即应成立，防"无条件注入"回归）。
#[test]
fn c3_team_policy_absent_when_unwired() {
    let outcome = run_child_probe("unwired");
    assert!(
        outcome.status.success(),
        "C3-A2 失败：未接线路径的提示面被改动（必须逐字节不变）。子进程 Exit={:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        outcome.status.code(),
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("TEAM_POLICY=absent"),
        "C3-A2 失败：子进程成功退出但未输出 `TEAM_POLICY=absent` 标记\nstdout:\n{}",
        outcome.stdout
    );
}

// ============================================================
// C5：同源门控（C5-A1 / C5-A3）
// ============================================================

/// C5-A3：策略段门控与 10 处 fail-closed 门必须复用**同一**判定 `subagent_runtime_wired()`。
///
/// 源码级判定（等价于契约的 `grep -n "subagent_runtime_wired()"` 覆盖 11 处）：
/// 1. `agent/tools.rs` 内 `subagent_runtime_wired()` 出现次数 == 11（1 处定义 + 10 处 fail-closed 门）；
/// 2. 承载策略段锚串的源文件必须**同时**引用同一判定函数（同源注册，禁止第二套接线判定）；
/// 3. 策略段组装点 `agent/context.rs` 必须引用同一判定函数（C3.3 落层位置）。
#[test]
fn c5_same_gate_source() {
    let src_root = crate_src_root();
    let tools_src = read_src(&src_root, "agent/tools.rs");
    let context_src = read_src(&src_root, "agent/context.rs");

    // Assert 1：10 处 fail-closed 门 + 1 处定义 == 11。
    let tools_hits = tools_src.matches("subagent_runtime_wired()").count();
    assert_eq!(
        tools_hits, 11,
        "C5-A3 失败：agent/tools.rs 内 `subagent_runtime_wired()` 必须出现 11 次\
         （1 处定义 + 10 处 fail-closed 门），实际 {tools_hits} 次"
    );

    // Assert 2：锚串所在文件的同源门控。
    let mut anchor_files: Vec<PathBuf> = Vec::new();
    for file in rust_files_under(&src_root) {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("读取源码失败 {}: {error}", file.display()));
        if text.contains(TEAM_POLICY_ANCHOR) {
            anchor_files.push(file);
        }
    }
    assert!(
        !anchor_files.is_empty(),
        "C5-A1/A3 失败：src 下不存在团队策略段锚串 `{TEAM_POLICY_ANCHOR}`（C3 未落地）"
    );
    let unsourced: Vec<String> = anchor_files
        .iter()
        .filter(|file| {
            std::fs::read_to_string(file)
                .map(|text| !text.contains("subagent_runtime_wired("))
                .unwrap_or(false)
        })
        .map(|file| relative_to_crate(file))
        .collect();
    assert!(
        unsourced.is_empty(),
        "C5-A3 失败：承载策略段锚串的文件未复用同一判定 `subagent_runtime_wired()`（存在第二套接线判定）：{unsourced:?}"
    );

    // Assert 3：组装点 context.rs 必须引用同一判定（C3.3 落层位置）。
    assert!(
        context_src.contains("subagent_runtime_wired("),
        "C5-A3 失败：策略段组装点 `agent/context.rs` 未引用同源判定 `subagent_runtime_wired()`，\
         无法保证「工具可执行 ⇔ 策略存在」由同一门派生"
    );
}

// ============================================================
// C7：send_message 必须真实驱动队友 turn（红相：当前只入队 + 改状态，不跑引擎、不回执）
// ============================================================

/// C7：`spawn_teammate` → `send_message` → 轮询 `list_agents` 直到成员回到 `Inactive`（上限 5s），
/// 断言 (a) 期间观察到 Running/Active、(b) lead 收件箱能取到 Stub 引擎的固定回执、
/// (c) 引擎真实被调用（`AtomicUsize` 计数 ≥1 且 send 之后确有增量）。
///
/// 进程隔离理由（同 C3）：`SUBAGENT_ENGINE` 是进程级 `OnceLock`，父进程已在 C2 用生产路径
/// `HostTurnEngine` 注册过引擎，**首写者胜**导致本用例的计数 Stub 无法注入。故真实断言在
/// 隔离子进程内执行（父进程只断言 exit code + `C7_OK` 标记），复用 `probe_child_env` 模式。
#[test]
fn c7_send_message_drives_teammate_turn() {
    let outcome = run_v13_child("c7");
    assert!(
        outcome.status.success(),
        "C7 失败（红相）：send_message 未真实驱动队友 turn。子进程 Exit={:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        outcome.status.code(),
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("C7_OK"),
        "C7 失败：子进程成功退出但未输出 `C7_OK` 标记\nstdout:\n{}",
        outcome.stdout
    );
}

/// C7b（C10 单飞）：同一 target 连续两次 `send_message`（间隔 < 10ms）必须只有一个在途驱动，
/// 两条消息都被消费且各只消费一次。真值断言在隔离子进程（`v13_child_env=c7b`）内执行。
#[test]
fn c7b_single_flight_per_target() {
    let outcome = run_v13_child("c7b");
    assert!(
        outcome.status.success(),
        "C7b 失败：单飞语义不成立（连续两次 send 产生了并发驱动或丢消息）。子进程 Exit={:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        outcome.status.code(),
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("C7B_OK"),
        "C7b 失败：子进程成功退出但未输出 `C7B_OK` 标记\nstdout:\n{}",
        outcome.stdout
    );
}

/// C7c（C10 失败补偿）：引擎返回 `subagent_busy` 时，消息必须原序回滚入队（不丢），
/// 成员回到 `Inactive`，lead 收件箱收到含真实错误码的回执且绝无成功回执。
#[test]
fn c7c_requeue_on_engine_error() {
    let outcome = run_v13_child("c7c");
    assert!(
        outcome.status.success(),
        "C7c 失败：引擎失败时未做入队回滚/错误回执。子进程 Exit={:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        outcome.status.code(),
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("C7C_OK"),
        "C7c 失败：子进程成功退出但未输出 `C7C_OK` 标记\nstdout:\n{}",
        outcome.stdout
    );
}

// ============================================================
// C8：子代理 turn 必须携带父会话工作区根（Wave-61 C8.3）
// ============================================================

/// C8：`orchestration::subagent_in_workspace(args, Some(root))` 必须把该 root **透传**给
/// 引擎的 `run_turn_in(..., Some(root))`；`None` 必须原样透传 `None`（由引擎解析默认根）。
///
/// 进程隔离理由：记录 root 需要注入覆写 `run_turn_in` 的 Stub 引擎，而父进程的引擎槽已被
/// C2/C3 用例占用（`OnceLock` 首写者胜），故真实断言在隔离子进程内执行（`v13_child_env`）。
#[test]
fn c8_subagent_turn_uses_parent_workspace_root() {
    let outcome = run_v13_child("c8");
    assert!(
        outcome.status.success(),
        "C8 失败：subagent_in_workspace 未把父工作区根透传给引擎。子进程 Exit={:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        outcome.status.code(),
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("C8_OK"),
        "C8 失败：子进程成功退出但未输出 `C8_OK` 标记\nstdout:\n{}",
        outcome.stdout
    );
}

// ============================================================
// C9：策略段门控在组装函数内只求值一次（红相：当前 3 处直接调用）
// ============================================================

/// C9：`build_layered_turn_context` 函数体内 `agent_team_policy_enabled()` 的直接调用次数 ≤ 1，
/// 且必须先绑定到局部变量再复用（防止同一函数内多次读 OnceLock，出现"中间态撕裂"）。
#[test]
fn c9_policy_gate_evaluated_once() {
    let src_root = crate_src_root();
    let context_src = read_src(&src_root, "agent/context.rs");
    let body = function_body(&context_src, "fn build_layered_turn_context(").unwrap_or_else(|| {
        panic!(
            "C9 失败：agent/context.rs 中找不到 `fn build_layered_turn_context(` —— \
             组装入口被改名/移动，本断言必须随之迁移（不得跳过）"
        )
    });

    // Assert 1：直接调用次数 ≤ 1。
    let direct_calls = body.matches("agent_team_policy_enabled()").count();
    assert!(
        direct_calls <= 1,
        "C9 失败：`build_layered_turn_context` 函数体内 `agent_team_policy_enabled()` 出现 {direct_calls} 次，\
         期望 ≤ 1（必须先 `let xxx = agent_team_policy_enabled();` 绑定一次，再在稳定层/原生层/观察层复用同一值）"
    );

    // Assert 2：存在局部绑定（`let <ident> = agent_team_policy_enabled();`）。
    let has_binding = body.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("let ")
            && trimmed.contains('=')
            && trimmed.contains("agent_team_policy_enabled()")
            && trimmed.ends_with(';')
    });
    assert!(
        has_binding,
        "C9 失败：`build_layered_turn_context` 内不存在 `let <name> = agent_team_policy_enabled();` 形态的局部绑定，\
         门控未被固化为单一求值点"
    );
}

// ============================================================
// 脚手架：进程隔离 / 组装入口 / 沙箱 / 源码扫描
// ============================================================

const CHILD_ENV: &str = "PONY_TEAM_POLICY_PROBE";

struct ChildOutcome {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// 递归调用本测试二进制，仅运行 [`probe_child_env`] 入口，并以环境变量分派接线状态。
fn run_child_probe(mode: &str) -> ChildOutcome {
    let exe = std::env::current_exe().expect("current_exe 必须可读（进程隔离依赖它）");
    let output = std::process::Command::new(&exe)
        .arg("probe_child_env")
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_ENV, mode)
        .output()
        .unwrap_or_else(|error| panic!("无法派生隔离子进程（mode={mode}）: {error}"));
    ChildOutcome {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

// ---- v1.3/v1.4 子进程隔离 + C7 可观测 Stub 引擎 ----

const V13_CHILD_ENV: &str = "PONY_WAVE61_V13_CHILD";
/// C7 的 Stub 引擎固定回执文本（用于断言 lead 收件箱里的队友回执确实来自引擎，而非合成串）。
const C7_STUB_REPLY: &str = "c7-stub-teammate-reply";

/// 状态序列观测器：轮询线程与引擎回调共用同一个序列，保证时序因果可复现。
#[derive(Default)]
struct StatusWatch {
    /// 去重后的状态序列（仅在"与上一个不同"时追加，允许重复采样被折叠）。
    sequence: Mutex<Vec<String>>,
    /// 被观测的成员 target（spawn 后填充）。
    target: Mutex<String>,
}

impl StatusWatch {
    fn set_target(&self, target: &str) {
        *self.target.lock().unwrap() = target.to_string();
    }

    /// 采样一次当前名单状态；仅当目标成员存在且状态与序列末位不同才追加。
    fn sample(&self) {
        let target = self.target.lock().unwrap().clone();
        if target.is_empty() {
            return;
        }
        let Ok(list) = orchestration::list_agents() else {
            return;
        };
        let Some(member) = list.agents.iter().find(|item| item.target == target) else {
            return;
        };
        let status = format!("{:?}", member.status);
        let mut sequence = self.sequence.lock().unwrap();
        if sequence.last() != Some(&status) {
            sequence.push(status);
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.sequence.lock().unwrap().clone()
    }
}

/// 可观测 Stub 引擎（C7 强化 / C7b 单飞 / C7c 失败补偿共用）。
///
/// 能力：调用计数、**同时在途峰值**（`fetch_max`）、收到的 prompt 全文、
/// 调用时刻的目标状态采样（写入 [`StatusWatch`]）、可注入失败码、可选"闸门"用于制造确定性重叠窗口。
struct ScenarioEngine {
    calls: AtomicUsize,
    in_flight: AtomicUsize,
    peak_in_flight: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    reply: String,
    fail_with: Option<String>,
    watch: Option<Arc<StatusWatch>>,
    gate: Option<Arc<(Mutex<bool>, Condvar)>>,
}

impl ScenarioEngine {
    fn new(reply: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            peak_in_flight: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            reply: reply.to_string(),
            fail_with: None,
            watch: None,
            gate: None,
        }
    }

    fn with_watch(mut self, watch: Arc<StatusWatch>) -> Self {
        self.watch = Some(watch);
        self
    }

    fn with_failure(mut self, code: &str) -> Self {
        self.fail_with = Some(code.to_string());
        self
    }

    fn with_gate(mut self, gate: Arc<(Mutex<bool>, Condvar)>) -> Self {
        self.gate = Some(gate);
        self
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl SubagentTurnEngine for ScenarioEngine {
    fn run_turn(&self, prompt: &str, _timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_in_flight.fetch_max(now, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(prompt.to_string());
        // 调用时刻采样：同步实现下外部 20ms 轮询可能错过极短的 Running 窗口。
        if let Some(watch) = &self.watch {
            watch.sample();
        }
        // 确定性重叠窗口：调用方放行前本调用一直"在途"（不依赖 sleep 制造时序）。
        if let Some(gate) = &self.gate {
            let (lock, condvar) = &**gate;
            let mut released = lock.lock().unwrap_or_else(|err| err.into_inner());
            let mut waited = Duration::ZERO;
            while !*released && waited < Duration::from_secs(3) {
                let (next, timeout) = condvar
                    .wait_timeout(released, Duration::from_millis(20))
                    .unwrap_or_else(|err| err.into_inner());
                released = next;
                waited += Duration::from_millis(20);
                let _ = timeout;
            }
        }
        let result = match &self.fail_with {
            Some(code) => Err(code.clone()),
            None => Ok(SubagentRunOutcome {
                final_text: self.reply.clone(),
                session_id: "scenario-stub-session".to_string(),
            }),
        };
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

/// 递归调用本测试二进制，仅运行 [`v13_child_env`]，以 `PONY_WAVE61_V13_CHILD` 分派场景。
fn run_v13_child(mode: &str) -> ChildOutcome {
    let exe = std::env::current_exe().expect("current_exe 必须可读（进程隔离依赖它）");
    let output = std::process::Command::new(&exe)
        .arg("v13_child_env")
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(V13_CHILD_ENV, mode)
        .output()
        .unwrap_or_else(|error| panic!("无法派生隔离子进程（mode={mode}）: {error}"));
    ChildOutcome {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// 注入 Stub 引擎并派生一个**不自动驱动**的队友（空初始 prompt → spawn 不触发首个 turn）。
///
/// 返回 `(engine, target)`。若 spawn 会因非空 prompt 自动驱动，则 C7 的"send 驱动"语义无法与
/// spawn 自身驱动区分，故这里用空 prompt 精确隔离被测因果。
fn spawn_inert_teammate(engine: Arc<ScenarioEngine>, tag: &str) -> (Arc<ScenarioEngine>, String) {
    let _ = set_subagent_engine(engine.clone() as Arc<dyn SubagentTurnEngine>);
    assert!(
        subagent_runtime_wired(),
        "C7 前置失败：Stub 引擎注入后必须已接线"
    );
    let name = format!("{tag}-{}-{}", std::process::id(), unique_stamp());
    let spawned = orchestration::spawn_teammate(SpawnTeammateArgs {
        name,
        role: Some("probe".to_string()),
        description: format!("{tag} probe teammate"),
        prompt: String::new(),
        context: None,
    })
    .expect("C7 前置失败：spawn_teammate 必须成功");
    assert_eq!(
        spawned.member.status,
        TeammateStatus::Active,
        "C7 前置失败：空 prompt 派生后成员必须处于 Active（未被自动驱动）"
    );
    (engine, spawned.member.target)
}

fn teammate_status_of(target: &str) -> Option<TeammateStatus> {
    orchestration::list_agents().ok().and_then(|list| {
        list.agents
            .into_iter()
            .find(|member| member.target == target)
            .map(|member| member.status)
    })
}

/// 轮询等待条件成立（20ms 步进、上限 `limit`；不忙等、不靠固定 sleep 猜时序）。
fn wait_until(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

/// C7 真实断言体（仅在隔离子进程内执行）：状态迁移序列 + 回执 + 引擎真实调用。
fn run_c7_body() {
    sandbox_env();
    let watch = Arc::new(StatusWatch::default());
    let engine = Arc::new(ScenarioEngine::new(C7_STUB_REPLY).with_watch(Arc::clone(&watch)));
    let (engine, target) = spawn_inert_teammate(engine, "c7-probe");
    watch.set_target(&target);
    // spawn 后（send 前）确定性采样：必须是 Active。
    watch.sample();

    // 20ms 轮询线程：记录去重状态序列。
    let stop = Arc::new(AtomicBool::new(false));
    let poller = {
        let stop = Arc::clone(&stop);
        let watch = Arc::clone(&watch);
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                watch.sample();
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };

    let calls_before = engine.calls.load(Ordering::SeqCst);
    orchestration::send_message(SendMessageArgs {
        target: target.clone(),
        message: "please review the workspace diff".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("C7 前置失败：send_message 必须成功");

    let reached_inactive = wait_until(Duration::from_secs(5), || {
        teammate_status_of(&target) == Some(TeammateStatus::Inactive)
    });
    std::thread::sleep(Duration::from_millis(60));
    stop.store(true, Ordering::SeqCst);
    let _ = poller.join();
    watch.sample();

    let sequence = watch.snapshot();
    let calls_after = engine.calls.load(Ordering::SeqCst);

    // Assert (a)（强化）：状态迁移序列必须观测到 Active → Running → Inactive，且索引单调递增。
    let position = |want: &str| sequence.iter().position(|status| status == want);
    let (active, running, inactive) = (
        position("Active"),
        position("Running"),
        position("Inactive"),
    );
    assert!(
        reached_inactive,
        "C7-A 失败：5s 轮询窗口内成员未回到 Inactive（观测序列 {sequence:?}）"
    );
    assert!(
        active.is_some(),
        "C7-A 失败：未观测到 Active（spawn 后、send 前）——观测序列 {sequence:?}"
    );
    assert!(
        running.is_some(),
        "C7-A 失败：未观测到 Running（send 驱动期间）——观测序列 {sequence:?}"
    );
    assert!(
        inactive.is_some(),
        "C7-A 失败：未观测到 Inactive（收口）——观测序列 {sequence:?}"
    );
    assert!(
        active < running && running < inactive,
        "C7-A 失败：状态迁移必须单调 Active → Running → Inactive，实际序列 {sequence:?}\
         （位置 Active={active:?} / Running={running:?} / Inactive={inactive:?}）"
    );

    // Assert (b)：lead 收件箱能取到队友回执（Stub 引擎固定文本）。
    let inbox = orchestration::drain_inbox(ReadInboxArgs {
        target: "lead".to_string(),
    })
    .unwrap_or_else(|error| {
        panic!("C7-B 失败：lead 收件箱取不到队友回执（drain_inbox 报错：{error}）")
    });
    assert!(
        inbox
            .messages
            .iter()
            .any(|message| message.content.contains(C7_STUB_REPLY)),
        "C7-B 失败：lead 收件箱中没有 Stub 引擎回执文本 `{C7_STUB_REPLY}`；实际消息：{:?}",
        inbox
            .messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
    );

    // Assert (c)：引擎真实被调用（≥1 且 send 之后确有增量）。
    assert!(
        calls_after >= 1,
        "C7-C 失败：Stub 引擎调用次数为 0——send_message 未触达 SubagentTurnEngine（伪造成功）"
    );
    assert!(
        calls_after > calls_before,
        "C7-C' 失败：send_message 前后引擎调用次数无增量（before={calls_before}, after={calls_after}）"
    );

    // Assert (d)：驱动确实消费了**本次 send 的文本**（排除"仅 spawn 的初始 prompt 触发"）。
    let prompts = engine.prompts();
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains("please review the workspace diff")),
        "C7-D 失败：引擎收到的 prompt 中不含本次 send 的消息文本；实际 prompts={prompts:?}"
    );

    println!(
        "C7_OK calls={calls_after} sequence={sequence:?} final={:?}",
        teammate_status_of(&target)
    );
}

/// C7b 真实断言体：同一 target 连续两次 send（间隔 < 10ms）必须单飞——
/// 同时在途调用峰值 == 1，且两条消息文本都被处理且各只处理一次。
fn run_c7b_body() {
    sandbox_env();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let engine = Arc::new(ScenarioEngine::new("c7b-stub-reply").with_gate(Arc::clone(&gate)));
    let (engine, target) = spawn_inert_teammate(engine, "c7b-probe");

    // 两次 send 背靠背（间隔远小于 10ms）。
    orchestration::send_message(SendMessageArgs {
        target: target.clone(),
        message: "c7b-first-alpha".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("C7b 前置失败：第一次 send_message 必须成功");
    orchestration::send_message(SendMessageArgs {
        target: target.clone(),
        message: "c7b-second-beta".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("C7b 前置失败：第二次 send_message 必须成功");

    // 确定性确保已有一个在途调用（闸门使其保持"在途"，不靠 sleep 猜时序）。
    let entered = wait_until(Duration::from_secs(2), || {
        engine.in_flight.load(Ordering::SeqCst) >= 1
    });
    // 放行闸门，让所有在途/排队调用跑完。
    {
        let (lock, condvar) = &*gate;
        *lock.lock().unwrap_or_else(|err| err.into_inner()) = true;
        condvar.notify_all();
    }
    assert!(
        entered,
        "C7b 前置失败：2s 内没有任何引擎调用进入执行（send 未驱动）"
    );

    // 等待静默：状态回到 Inactive、无在途调用、调用数连续两次采样不变。
    let mut stable = 0usize;
    let mut last_calls = usize::MAX;
    let _ = wait_until(Duration::from_secs(5), || {
        let calls = engine.calls.load(Ordering::SeqCst);
        if calls == last_calls && engine.in_flight.load(Ordering::SeqCst) == 0 {
            stable += 1;
        } else {
            stable = 0;
            last_calls = calls;
        }
        stable >= 3 && teammate_status_of(&target) == Some(TeammateStatus::Inactive)
    });

    let peak = engine.peak_in_flight.load(Ordering::SeqCst);
    let calls = engine.calls.load(Ordering::SeqCst);
    let prompts = engine.prompts();
    let joined = prompts.join("\n");

    // Assert 1（单飞）：同时在途调用峰值 == 1（不允许两个并发驱动线程）。
    assert_eq!(
        peak, 1,
        "C7b 失败：同一 target 的同时在途引擎调用峰值为 {peak}（期望 1）——\
         两次 send_message 各自派生了并发驱动线程，违反 C10 单飞语义。prompts={prompts:?}"
    );

    // Assert 2（不丢消息）：两条文本都必须被处理。
    assert!(
        joined.contains("c7b-first-alpha"),
        "C7b 失败：第一条消息文本未被任何引擎调用消费；prompts={prompts:?}"
    );
    assert!(
        joined.contains("c7b-second-beta"),
        "C7b 失败：第二条消息文本未被任何引擎调用消费；prompts={prompts:?}"
    );

    // Assert 3（不重复消费）：每条文本只能出现在一次引擎调用里。
    for needle in ["c7b-first-alpha", "c7b-second-beta"] {
        let hits = prompts
            .iter()
            .filter(|prompt| prompt.contains(needle))
            .count();
        assert_eq!(
            hits, 1,
            "C7b 失败：`{needle}` 被 {hits} 次引擎调用重复消费（期望 1）；prompts={prompts:?}"
        );
    }
    assert!(calls >= 1, "C7b 失败：引擎从未被调用（send 未驱动）");

    println!("C7B_OK calls={calls} peak={peak} prompts={}", prompts.len());
}

/// C7c 真实断言体：引擎失败（`subagent_busy`）时必须**回滚入队**（消息不丢、顺序不变）、
/// 成员回到 Inactive，并把错误回执投递到 lead 收件箱。
fn run_c7c_body() {
    sandbox_env();
    let engine = Arc::new(ScenarioEngine::new("unused").with_failure("subagent_busy"));
    let (engine, target) = spawn_inert_teammate(engine, "c7c-probe");

    // 先用纯入队 seam 造两条确定顺序的积压消息（不驱动），再由 send_message 触发一次驱动作业。
    for text in ["c7c-alpha", "c7c-beta"] {
        orchestration::enqueue_message(SendMessageArgs {
            target: target.clone(),
            message: text.to_string(),
            mode: Some(MessageDeliveryMode::Queue),
        })
        .expect("C7c 前置失败：enqueue_message 必须成功");
    }
    orchestration::send_message(SendMessageArgs {
        target: target.clone(),
        message: "c7c-trigger".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("C7c 前置失败：send_message 必须成功");

    // 等待收口：驱动失败后成员必须回到 Inactive。
    assert!(
        wait_until(Duration::from_secs(5), || {
            teammate_status_of(&target) == Some(TeammateStatus::Inactive)
        }),
        "C7c 失败：引擎失败后 5s 内成员未回到 Inactive（实际 {:?}）",
        teammate_status_of(&target)
    );
    assert_eq!(
        engine.calls.load(Ordering::SeqCst),
        1,
        "C7c 前置失败：期望一次驱动作业命中引擎一次，实际 {}",
        engine.calls.load(Ordering::SeqCst)
    );

    // Assert 1（不丢消息 + 顺序不变）：三条消息必须仍在目标收件箱，且相对顺序不变。
    let inbox = orchestration::drain_inbox(ReadInboxArgs {
        target: target.clone(),
    })
    .expect("C7c 失败：目标收件箱必须仍存在");
    let contents: Vec<String> = inbox
        .messages
        .iter()
        .map(|message| message.content.clone())
        .collect();
    let expected = vec![
        "c7c-alpha".to_string(),
        "c7c-beta".to_string(),
        "c7c-trigger".to_string(),
    ];
    assert_eq!(
        contents, expected,
        "C7c 失败：引擎失败后消息未按原顺序回滚入队（期望 {expected:?}，实际 {contents:?}）——\
         失败补偿缺失导致消息丢失"
    );

    // Assert 2（不伪造成功）：lead 收件箱必须收到错误回执，且含真实错误码。
    let lead_inbox = orchestration::drain_inbox(ReadInboxArgs {
        target: "lead".to_string(),
    })
    .expect("C7c 失败：lead 收件箱必须存在");
    let lead_contents: Vec<String> = lead_inbox
        .messages
        .iter()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        lead_contents
            .iter()
            .any(|message| message.contains("subagent_busy")),
        "C7c 失败：lead 收件箱未收到含 `subagent_busy` 的错误回执（实际 {lead_contents:?}）"
    );
    assert!(
        !lead_contents
            .iter()
            .any(|message| message.contains(C7_STUB_REPLY)),
        "C7c 失败：引擎失败路径出现了成功回执（伪造成功）"
    );

    println!("C7C_OK contents={contents:?}");
}

/// 记录 `run_turn_in` 收到的 workspace_root 的 Stub 引擎。
///
/// `run_turn` 直接 panic：Wave-61 C8.3 要求 `subagent_in_workspace` 走 `run_turn_in`，
/// 若退回 `run_turn`，父根信息即丢失（子代理会用默认根建栈，违反 wave-58 F1-5）。
struct RootRecordingEngine {
    records: Mutex<Vec<Option<PathBuf>>>,
}

impl RootRecordingEngine {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
}

impl SubagentTurnEngine for RootRecordingEngine {
    fn run_turn(&self, _prompt: &str, _timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        panic!(
            "C8 失败：`subagent_in_workspace` 调用了 run_turn 而非 run_turn_in —— \
             父工作区根在调用链上被丢弃（wave-58 F1-5 / Wave-61 C8.3）"
        );
    }

    fn run_turn_in(
        &self,
        _prompt: &str,
        _timeout_ms: u64,
        workspace_root: Option<&Path>,
    ) -> Result<SubagentRunOutcome, String> {
        self.records
            .lock()
            .unwrap()
            .push(workspace_root.map(Path::to_path_buf));
        Ok(SubagentRunOutcome {
            final_text: "c8-stub-subagent-reply".to_string(),
            session_id: "c8-stub-session".to_string(),
        })
    }
}

/// C8 真实断言体（仅在隔离子进程内执行）。
fn run_c8_body() {
    sandbox_env();
    let engine = Arc::new(RootRecordingEngine::new());
    let _ = set_subagent_engine(engine.clone() as Arc<dyn SubagentTurnEngine>);
    assert!(
        subagent_runtime_wired(),
        "C8 前置失败：Stub 引擎注入后必须已接线"
    );

    let root = std::env::temp_dir().join(format!(
        "wave61-c8-parent-root-{}-{}",
        std::process::id(),
        unique_stamp()
    ));
    std::fs::create_dir_all(&root).expect("C8 前置失败：创建父根临时目录失败");

    let make_args = || pony_agent_core::agent::orchestration::SubagentArgs {
        prompt: "c8 probe subagent".to_string(),
        description: "c8 probe".to_string(),
        run_in_background: Some(false),
        fork_context: None,
    };

    // (1) Some(root)：父根必须逐字透传。
    let with_root = orchestration::subagent_in_workspace(make_args(), Some(root.as_path()))
        .unwrap_or_else(|error| {
            panic!("C8 失败：subagent_in_workspace(Some(root)) 返回 Err: {error}")
        });
    assert_eq!(
        with_root.status, "completed",
        "C8 失败：同步子代理 turn 的 status 必须为 completed"
    );
    let recorded = engine.records.lock().unwrap().clone();
    let canonical_expected = root.canonicalize().unwrap_or_else(|_| root.clone());
    let first =
        recorded.first().cloned().flatten().unwrap_or_else(|| {
            panic!("C8 失败：引擎未收到 workspace_root（records={recorded:?}）")
        });
    let first_canonical = first.canonicalize().unwrap_or_else(|_| first.clone());
    assert_eq!(
        first_canonical, canonical_expected,
        "C8 失败：引擎收到的 workspace_root 与传入的父根不一致（recorded={recorded:?}，expected={canonical_expected:?}）"
    );

    // (2) None：必须原样透传 None（由引擎自行解析默认根，不得凭空造一个根）。
    orchestration::subagent_in_workspace(make_args(), None)
        .unwrap_or_else(|error| panic!("C8 失败：subagent_in_workspace(None) 返回 Err: {error}"));
    let recorded = engine.records.lock().unwrap().clone();
    assert_eq!(
        recorded.len(),
        2,
        "C8 失败：引擎应被调用 2 次（Some(root) + None），实际 {recorded:?}"
    );
    assert_eq!(
        recorded[1], None,
        "C8 失败：`subagent_in_workspace(args, None)` 必须把 None 原样透传给引擎（实际 {:?}）",
        recorded[1]
    );

    let _ = std::fs::remove_dir_all(&root);
    println!("C8_OK records={recorded:?}");
}

/// 从源码文本中取出以 `needle` 开头的函数体（按大括号配平；返回不含外层大括号的函数体）。
fn function_body<'a>(source: &'a str, needle: &str) -> Option<&'a str> {
    let start = source.find(needle)?;
    let body_start = source[start..].find('{')? + start;
    let mut depth = 0usize;
    let mut in_string = false;
    let bytes = source.as_bytes();
    let mut index = body_start;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        match ch {
            '"' if !is_escaped(source, index) => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[body_start + 1..index]);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn is_escaped(source: &str, index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if source.as_bytes()[cursor] == b'\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

/// 公开 API 置入宿主引擎（C3 只验证"接线后策略段是否注入"，不验证接线来源）。
fn wire_engine_public_api() {
    if subagent_engine().is_none() {
        let engine: Arc<dyn SubagentTurnEngine> = Arc::new(HostTurnEngine::new());
        let _ = set_subagent_engine(engine);
    }
    assert!(
        subagent_runtime_wired(),
        "C3 前置失败：公开 API `set_subagent_engine` 后 `subagent_runtime_wired()` 必须为 true"
    );
}

/// 走既有公开组装入口：`TurnContextBuilder::build_request`（`context.rs:286-320`）→ `input`。
fn assemble_messages(tag: &str) -> Vec<ProviderMessage> {
    sandbox_env();
    let workspace = temp_workspace(tag);
    let provider = provider_manager(Some(128_000));
    let retrieved = retrieved_state(&workspace, &unique_conversation_id(tag));
    let request =
        DefaultTurnContextBuilder.build_request("wave61-graph", &provider, &retrieved, &[]);
    let _ = std::fs::remove_dir_all(&workspace);
    request.input
}

fn system_messages(messages: &[ProviderMessage]) -> Vec<&ProviderMessage> {
    messages
        .iter()
        .filter(|message| matches!(message.role, ProviderRole::System))
        .collect()
}

fn role_tag(message: &ProviderMessage) -> &'static str {
    match message.role {
        ProviderRole::System => "System",
        ProviderRole::Developer => "Developer",
        ProviderRole::User => "User",
    }
}

fn provider_manager(context_window_tokens: Option<u32>) -> ProviderManager {
    ProviderManager::new(ResolvedProviderSelection {
        requested_name: "wave61-test-provider".to_string(),
        provider_name: "wave61-test-provider".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: "https://example.invalid/v1".to_string(),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "WAVE61_TEST_PROVIDER_API_KEY".to_string(),
        // 仅用于装配：`build_request` 不发网络请求，断言全部落在离线可观测投影上。
        api_key: Some("wave61-dummy-key".to_string()),
        model: "wave61-test-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    })
}

fn retrieved_state(cwd: &Path, conversation_id: &str) -> RetrievedContextState {
    RetrievedContextState {
        turn_context: TurnContext {
            user_message: "wave61 team activation probe".to_string(),
            images: Vec::new(),
            references_image: false,
            workspace_mode: None,
        },
        session_context: SessionContext {
            conversation_id: conversation_id.to_string(),
            title: "wave61".to_string(),
            summary: String::new(),
            recent_history: Vec::new(),
            recent_attachment_assets: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            env_info: Some(EnvironmentInfo {
                cwd: cwd.display().to_string(),
                platform: std::env::consts::OS.to_string(),
                shell: None,
                os_version: None,
                current_date: "2026-11-03".to_string(),
                timezone: Some("UTC".to_string()),
                is_git_repo: false,
                git_branch: None,
            }),
        },
        run_state: RunState::default(),
        long_term_memory: LongTermMemory::default(),
        transcript: TranscriptContext::default(),
    }
}

static SANDBOX: Once = Once::new();

/// 数据卫生：把 HOME / XDG_DATA_HOME / LOCALAPPDATA 重定向到本测试二进制独占的临时目录，
/// 杜绝读写用户真实数据目录（与既有 `slash_commands_acceptance.rs` 同一沙箱口径）。
fn sandbox_env() {
    SANDBOX.call_once(|| {
        let sandbox = std::env::temp_dir().join(format!(
            "pony-agent-it-wave61-{}-{}",
            std::process::id(),
            unique_stamp()
        ));
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
}

fn sandboxed_session_store(label: &str) -> SessionStore {
    sandbox_env();
    let storage = std::env::temp_dir()
        .join(format!("pony-agent-it-wave61-{}", std::process::id()))
        .join(format!("{label}-sessions"));
    std::fs::create_dir_all(&storage).expect("create isolated session storage");
    SessionStore::with_backend(Box::new(FileSessionBackend::new(storage)))
}

fn temp_workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "wave61-{tag}-{}-{}",
        std::process::id(),
        unique_stamp()
    ));
    std::fs::create_dir_all(&root).expect("create temp workspace");
    root
}

fn unique_conversation_id(tag: &str) -> String {
    format!("wave61-{tag}-{}-{}", std::process::id(), unique_stamp())
}

fn unique_stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn crate_src_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        root.is_dir(),
        "源码扫描根目录不存在：{}（断言不得跳过）",
        root.display()
    );
    root
}

fn read_src(src_root: &Path, relative: &str) -> String {
    let path = src_root.join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取源码失败 {}: {error}", path.display()))
}

fn rust_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read_dir = std::fs::read_dir(&current)
            .unwrap_or_else(|error| panic!("读取目录失败 {}: {error}", current.display()));
        let mut entries: Vec<PathBuf> = read_dir
            .map(|entry| entry.expect("directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files
}

fn relative_to_crate(path: &Path) -> String {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    path.strip_prefix(crate_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

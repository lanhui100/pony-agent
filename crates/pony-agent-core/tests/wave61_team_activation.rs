//! Wave-61 团队激活红相验收测试（冻结测试；Executor 只读）。
//!
//! 契约真值源：`.dev-team/contracts/wave-61-team-activation.md`
//! - C1（C1-A1/A2/A3）：模型可见工具面 29 → 39，10 项团队工具全部 `ModelVisible`
//! - C2（C2-A3/A4）：`AgentRuntimeBuilder::build()`（`runtime/builder.rs:79`）注册 `HostTurnEngine`，
//!   构建过 runtime 的进程内 `subagent_runtime_wired() == true`
//! - C3（C3-A1/A2）：已接线 → System 消息数 == 2 且第 2 条以
//!   `Agent Teams is available in this session` 开头；未接线 → System 数 == 1 且全文无该锚串
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
use std::sync::{Arc, Once};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    base_system_prompt, prompt_layers_are_ordered, DefaultTurnContextBuilder, LongTermMemory,
    RetrievedContextState, RunState, SessionContext, TranscriptContext, TurnContext,
    TurnContextBuilder,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderMessage, ProviderProtocol, ProviderRole,
};
use pony_agent_core::agent::runtime::AgentRuntimeBuilder;
use pony_agent_core::agent::runtime_helper::TestRuntimeGuard;
use pony_agent_core::agent::session::{EnvironmentInfo, FileSessionBackend, SessionStore};
use pony_agent_core::agent::subagent_runtime::{
    set_subagent_engine, subagent_engine, HostTurnEngine, SubagentTurnEngine,
};
use pony_agent_core::agent::tools::{
    builtin_tool_surface, builtin_turn_tool_contract_views, model_visible_tool_name,
    product_visible_tool_name, subagent_runtime_wired,
};

// ============================================================
// 冻结常量（逐字取自契约 C1.1 / C1.2 / C3.4）
// ============================================================

/// 契约 C1.1：10 个团队工具 primitive 名（product 名 == primitive 名，小写原形）。
const TEAM_TOOLS: [&str; 10] = [
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

/// 契约 C3.4：策略段标题行（机器判定锚串，与 DSH `:36` 同一锚串）。
const TEAM_POLICY_ANCHOR: &str = "Agent Teams is available in this session";

/// 契约 C1.2：39 项模型可见面精确清单（含顺序）。
///
/// 顺序真值源：registry 排序键 `(!visible, product_first_index)`（`tools.rs:555-566`）
/// ＋ `builtin_tools()` 定义序（10 项团队工具注册于 `tools.rs:5925-6138`，夹在 `view_image`
/// `:5894` 与 `todo_write` `:6147` 之间），故 10 项插入 `ViewImage` 之后、`TodoWrite` 之前。
const EXPECTED_39: [&str; 39] = [
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
    "workflow",
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

    // Assert 1：总数必须为 39（C1-A1）。
    assert_eq!(
        names.len(),
        39,
        "C1-A1 失败：模型可见工具面必须为 39 项（29 既有 + 10 团队）。实际 {} 项：{names:?}",
        names.len()
    );

    // Assert 2：10 个团队工具逐个必须出现（C1-A2 的集合侧；失败信息逐名可见）。
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

    // Assert 3：逐字、逐序等于 C1.2 的 39 行清单（含顺序）。
    let expected: Vec<&str> = EXPECTED_39.to_vec();
    assert_eq!(
        names, expected,
        "C1-A1 失败：模型可见面与契约 C1.2 的 39 项精确清单（含顺序）不一致"
    );

    // Assert 4：产品名不得塌缩——`Run` 在 39 项中必须唯一（`time_now`/`run_command` 共用槽位，
    // 若团队工具误落 `unwrap_or("Run")` 兜底会产生重复 Run）。
    let run_slots = names.iter().filter(|name| **name == "Run").count();
    assert_eq!(
        run_slots, 1,
        "C1 失败：`Run` 槽位必须唯一（产品名塌缩会把团队工具误标成 Run）；实际出现 {run_slots} 次"
    );

    // Assert 5：视图的 exposure 字段对 10 项团队工具必须为 model_visible（C4.3/T4 同源口径）。
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

/// 契约 C1-A6（`model_visible_tool_name` 不得回落 `Run`）+ C1-A7（产品名原样透传）
/// + C1-A3（provider 投影与 turn view 交叉一致）。
#[test]
fn c1_product_names_do_not_fall_back_to_run() {
    // Assert 1（C1.6 / C1-A6，当前红相）：10 项必须登记小写原名透传，
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

    // Assert 2（C1.1 / C1-A7）：10 项 provider 面 product 名必须原样透传小写原名。
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

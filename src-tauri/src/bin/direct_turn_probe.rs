#![allow(dead_code)]

#[path = "../agent/mod.rs"]
mod agent;

use agent::runtime::{AgentRuntime, TurnInput, TurnResult};
use agent::telemetry::{TurnToolActivity, TurnTraceStep};
use serde_json::Value;
use std::collections::BTreeSet;
use std::env;

struct DirectScenario {
    name: &'static str,
    description: &'static str,
    session_id: &'static str,
    expected_signal: &'static str,
    prompts: &'static [&'static str],
    expected_all_tools: &'static [&'static str],
    expected_any_tools: &'static [&'static str],
    expect_planned_children: bool,
    expect_nested_children: bool,
    expect_large_tool_payload: bool,
    expect_provider_signal: bool,
}

struct TurnSignalSummary {
    ready: bool,
    provider_mode: String,
    fallback_reason: Option<String>,
    tool_names: Vec<String>,
    planned_children: usize,
    nested_children: usize,
    max_tool_result_chars: usize,
    assistant_chars: usize,
    first_token_latency_ms: Option<u64>,
}

const DIRECT_SCENARIOS: &[DirectScenario] = &[
    DirectScenario {
        name: "workspace-tool",
        description: "验证 workspace 工具命中与文件读取链路。",
        session_id: "probe-workspace-tool",
        expected_signal: "第一轮应命中 workspace_list_files，第二轮应命中 workspace_read_file。",
        prompts: &[
            "当前文件夹中有哪些文件？",
            "这是什么文件？tauri.conf.json，都有哪些配置？",
        ],
        expected_all_tools: &["workspace_list_files", "workspace_read_file"],
        expected_any_tools: &[],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: false,
        expect_provider_signal: true,
    },
    DirectScenario {
        name: "history-carry",
        description: "验证 session history carry，第二轮不显式给文件名。",
        session_id: "probe-history-carry",
        expected_signal: "第二轮应基于上一轮历史继续命中 workspace_read_file 或 workspace_gather_context。",
        prompts: &[
            "请记住 tauri.conf.json 这个文件，后面我会继续问它。",
            "继续说这个文件里都配置了什么。",
        ],
        expected_all_tools: &[],
        expected_any_tools: &["workspace_read_file", "workspace_gather_context"],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: false,
        expect_provider_signal: true,
    },
    DirectScenario {
        name: "followup-fallback",
        description: "验证多轮 follow-up 与 provider/fallback 诊断信号。",
        session_id: "probe-followup-fallback",
        expected_signal:
            "真实 provider 可用时应保持 provider_mode=live；不可用时应在 fallback_reason 中直接体现。",
        prompts: &[
            "请用中文概括当前 Pony Agent 的 agent core 目标。",
            "继续上一轮，再补充当前是否发生了 provider fallback，以及原因是什么。",
        ],
        expected_all_tools: &[],
        expected_any_tools: &[],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: false,
        expect_provider_signal: true,
    },
    DirectScenario {
        name: "multipath-context",
        description: "验证多路径上下文收集时的组合工具与子调用可观测性。",
        session_id: "probe-multipath-context",
        expected_signal:
            "应命中 workspace_gather_context 或 workspace_batch，并在 tool_activities 中看到 planned/nested 子调用。",
        prompts: &[r"请同时查看 src-tauri/src/agent/tools.rs、src-tauri/src/agent/planner.rs、src-tauri/src/agent/telemetry.rs，概括它们在 ToolPlan 流转中的角色，并指出各自最相关的函数。"],
        expected_all_tools: &[],
        expected_any_tools: &["workspace_gather_context", "workspace_batch"],
        expect_planned_children: true,
        expect_nested_children: true,
        expect_large_tool_payload: false,
        expect_provider_signal: true,
    },
    DirectScenario {
        name: "large-result",
        description: "验证大文件/大结果场景下是否能清楚暴露工具与回合信号。",
        session_id: "probe-large-result",
        expected_signal:
            "应命中 workspace_read_file_segment / workspace_gather_context / workspace_search_text 之一，并给出较大的 tool result 体量信号。",
        prompts: &[r"请分析 src-tauri/src/agent/tools.rs 里 workspace_gather_context 的实现，给出关键流程与依据。文件较大时请自行选择分段读取、搜索或聚合上下文，不要省略判断依据。"],
        expected_all_tools: &[],
        expected_any_tools: &[
            "workspace_read_file_segment",
            "workspace_gather_context",
            "workspace_search_text",
        ],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: true,
        expect_provider_signal: true,
    },
];

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("--list")) {
        print_direct_scenarios();
        return;
    }

    let scenario_name = args.first().map(String::as_str).unwrap_or("all");
    let selected = select_direct_scenarios(scenario_name);
    if selected.is_empty() {
        eprintln!(
            "未知场景 `{}`。可先运行 `cargo run --bin direct_turn_probe -- --list` 查看可用场景。",
            scenario_name
        );
        std::process::exit(2);
    }

    eprintln!("direct_turn_probe");
    eprintln!("用法: cargo run --bin direct_turn_probe -- [all|workspace-tool|history-carry|followup-fallback|multipath-context|large-result|--list]");

    let mut runtime = AgentRuntime::new();
    for scenario in selected {
        run_direct_scenario(&mut runtime, scenario);
    }
}

fn print_direct_scenarios() {
    eprintln!("direct_turn_probe 场景列表：");
    for scenario in DIRECT_SCENARIOS {
        eprintln!(
            "- {}: {} | 期望信号: {}",
            scenario.name, scenario.description, scenario.expected_signal
        );
    }
}

fn select_direct_scenarios(name: &str) -> Vec<&'static DirectScenario> {
    if name == "all" {
        return DIRECT_SCENARIOS.iter().collect();
    }

    DIRECT_SCENARIOS
        .iter()
        .filter(|scenario| scenario.name == name)
        .collect()
}

fn run_direct_scenario(runtime: &mut AgentRuntime, scenario: &DirectScenario) {
    eprintln!("\n=== direct scenario: {} ===", scenario.name);
    eprintln!("description: {}", scenario.description);
    eprintln!("session_id: {}", scenario.session_id);
    eprintln!("expected_signal: {}", scenario.expected_signal);

    let mut turn_summaries = Vec::new();
    for (index, message) in scenario.prompts.iter().enumerate() {
        eprintln!("\n--- turn {} ---", index + 1);
        eprintln!("user: {}", message);

        let result = runtime.run_turn(TurnInput {
            message: (*message).to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some(scenario.session_id.to_string()),
            node_id: None,
            history: vec![],
            images: Vec::new(),
        });

        let summary = analyze_turn_result(&result);
        print_turn_result(&result, &summary);
        turn_summaries.push(summary);
    }

    print_scenario_summary(scenario, &turn_summaries);
}

fn analyze_turn_result(result: &TurnResult) -> TurnSignalSummary {
    let mut tool_names = BTreeSet::new();
    let mut planned_children = 0;
    let mut nested_children = 0;
    let mut max_tool_result_chars = 0;

    for activity in &result.tool_activities {
        tool_names.insert(activity.name.clone());
        if activity.status == "planned" {
            planned_children += 1;
        }
        if activity.id.contains("child-") {
            nested_children += 1;
        }
        let result_chars = activity
            .result_text
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or(0);
        max_tool_result_chars = max_tool_result_chars.max(result_chars);
    }

    if planned_children == 0 {
        for activity in &result.tool_activities {
            let inferred = activity
                .result_text
                .as_deref()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .and_then(|value| value.get("plannedCount").and_then(Value::as_u64))
                .unwrap_or(0) as usize;
            planned_children = planned_children.max(inferred);
        }
    }

    TurnSignalSummary {
        ready: result.phase == "ready",
        provider_mode: result.provider_mode.clone(),
        fallback_reason: result.fallback_reason.clone(),
        tool_names: tool_names.into_iter().collect(),
        planned_children,
        nested_children,
        max_tool_result_chars,
        assistant_chars: result.assistant_message.chars().count(),
        first_token_latency_ms: result.first_token_latency_ms,
    }
}

fn print_turn_result(result: &TurnResult, summary: &TurnSignalSummary) {
    eprintln!("phase: {}", result.phase);
    eprintln!(
        "provider: requested={} active={}/{} model={} mode={}",
        result.provider_requested_name,
        result.provider_name,
        result.provider_protocol,
        result.provider_model,
        result.provider_mode
    );
    eprintln!(
        "fallback_reason: {}",
        result.fallback_reason.as_deref().unwrap_or("none")
    );
    eprintln!(
        "token_usage: in={:?} out={:?} total={:?} first_token_latency_ms={:?}",
        result.input_tokens,
        result.output_tokens,
        result.total_tokens,
        result.first_token_latency_ms
    );
    eprintln!("session_summary: {}", result.session_summary);
    eprintln!("trace_steps: {}", format_trace_steps(&result.trace_steps));
    eprintln!(
        "tool_activities: {}",
        format_tool_activities(&result.tool_activities)
    );
    eprintln!(
        "signal_summary: ready={} provider_mode={} fallback={} tools={} planned_children={} nested_children={} max_tool_result_chars={} assistant_chars={} first_token_latency_ms={:?}",
        yes_no(summary.ready),
        summary.provider_mode,
        summary.fallback_reason.as_deref().unwrap_or("none"),
        format_tool_names(&summary.tool_names),
        summary.planned_children,
        summary.nested_children,
        summary.max_tool_result_chars,
        summary.assistant_chars,
        summary.first_token_latency_ms
    );
    eprintln!(
        "assistant_preview: {}",
        preview(&result.assistant_message, 280)
    );
}

fn print_scenario_summary(scenario: &DirectScenario, summaries: &[TurnSignalSummary]) {
    let seen_tools = collect_seen_tools(summaries);
    let total_planned_children: usize = summaries
        .iter()
        .map(|summary| summary.planned_children)
        .sum();
    let total_nested_children: usize = summaries
        .iter()
        .map(|summary| summary.nested_children)
        .sum();
    let max_tool_result_chars = summaries
        .iter()
        .map(|summary| summary.max_tool_result_chars)
        .max()
        .unwrap_or(0);
    let all_ready = summaries.iter().all(|summary| summary.ready);
    let provider_signal_ok = summaries.iter().all(|summary| {
        !summary.provider_mode.trim().is_empty()
            && (summary.provider_mode == "live" || summary.fallback_reason.is_some())
    });

    let mut failed_checks = Vec::new();
    let mut warn_checks = Vec::new();

    if !all_ready {
        failed_checks.push("存在未进入 ready 的 turn".to_string());
    }
    if scenario.expect_provider_signal && !provider_signal_ok {
        failed_checks.push("provider_mode / fallback_reason 诊断信号不足".to_string());
    }
    if !scenario.expected_all_tools.is_empty() {
        let missing = missing_tools(&seen_tools, scenario.expected_all_tools);
        if !missing.is_empty() {
            failed_checks.push(format!("缺少必需工具信号: {}", missing.join(", ")));
        }
    }
    if !scenario.expected_any_tools.is_empty()
        && !scenario
            .expected_any_tools
            .iter()
            .any(|name| seen_tools.iter().any(|tool| tool == name))
    {
        failed_checks.push(format!(
            "未命中任一期望工具: {}",
            scenario.expected_any_tools.join(", ")
        ));
    }
    if scenario.expect_planned_children && total_planned_children == 0 {
        warn_checks.push("未观察到 planned 子调用活动".to_string());
    }
    if scenario.expect_nested_children && total_nested_children == 0 {
        warn_checks.push("未观察到 nested 子调用结果".to_string());
    }
    if scenario.expect_large_tool_payload && max_tool_result_chars < 400 {
        warn_checks.push(format!(
            "最大 tool result 仅 {} chars，未明显进入大结果区间",
            max_tool_result_chars
        ));
    }

    let verdict = if failed_checks.is_empty() && warn_checks.is_empty() {
        "PASS"
    } else if failed_checks.is_empty() {
        "WARN"
    } else {
        "FAIL"
    };

    eprintln!("\nscenario_summary:");
    eprintln!("  verdict: {}", verdict);
    eprintln!("  seen_tools: {}", format_tool_names(&seen_tools));
    eprintln!("  total_planned_children: {}", total_planned_children);
    eprintln!("  total_nested_children: {}", total_nested_children);
    eprintln!("  max_tool_result_chars: {}", max_tool_result_chars);
    if !failed_checks.is_empty() {
        eprintln!("  failed_checks: {}", failed_checks.join(" | "));
    }
    if !warn_checks.is_empty() {
        eprintln!("  warn_checks: {}", warn_checks.join(" | "));
    }
}

fn collect_seen_tools(summaries: &[TurnSignalSummary]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for summary in summaries {
        for tool in &summary.tool_names {
            seen.insert(tool.clone());
        }
    }
    seen.into_iter().collect()
}

fn missing_tools(seen_tools: &[String], expected_tools: &[&str]) -> Vec<String> {
    expected_tools
        .iter()
        .filter(|tool| !seen_tools.iter().any(|seen| seen == **tool))
        .map(|tool| (*tool).to_string())
        .collect()
}

fn format_trace_steps(steps: &[TurnTraceStep]) -> String {
    if steps.is_empty() {
        return "none".to_string();
    }

    steps
        .iter()
        .map(|step| format!("{}={}", step.label, step.state))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn format_tool_activities(activities: &[TurnToolActivity]) -> String {
    if activities.is_empty() {
        return "none".to_string();
    }

    activities
        .iter()
        .map(|activity| {
            let arguments = activity
                .arguments_text
                .as_deref()
                .map(|text| format!(" args={}", preview(text, 80)))
                .unwrap_or_default();
            let result_text = activity
                .result_text
                .as_deref()
                .map(|text| format!(" result={}", preview(text, 100)))
                .unwrap_or_default();
            let duration = activity
                .duration_seconds
                .map(|seconds| format!(" {:.3}s", seconds))
                .unwrap_or_default();
            format!(
                "{} [{}] {}{}{}{}",
                activity.name, activity.status, activity.summary, arguments, result_text, duration
            )
        })
        .collect::<Vec<_>>()
        .join(" || ")
}

fn format_tool_names(tool_names: &[String]) -> String {
    if tool_names.is_empty() {
        "none".to_string()
    } else {
        tool_names.join(", ")
    }
}

fn preview(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized;
    }

    let compact = normalized.chars().take(max_chars).collect::<String>();
    format!("{}...(+{} chars)", compact, char_count - max_chars)
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

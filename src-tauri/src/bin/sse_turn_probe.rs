#![allow(dead_code)]

#[path = "../agent/mod.rs"]
mod agent;
#[path = "../sse_adapter.rs"]
mod sse_adapter;

use agent::runtime::{AgentRuntime, TurnInput, TurnStreamEvent};
use sse_adapter::BufferingSseTurnEventSink;
use std::collections::BTreeSet;
use std::env;

struct SseScenario {
    name: &'static str,
    description: &'static str,
    session_id: &'static str,
    expected_signal: &'static str,
    prompts: &'static [&'static str],
    expected_any_tools: &'static [&'static str],
    expect_planned_children: bool,
    expect_nested_children: bool,
    expect_large_tool_payload: bool,
}

struct ParsedFrame {
    event_name: String,
    frame_id: Option<String>,
    payload: Option<TurnStreamEvent>,
    raw: String,
}

struct SseTurnSummary {
    frame_count: usize,
    event_names: Vec<String>,
    delta_chunks: usize,
    reasoning_chunks: usize,
    turn_ids_consistent: bool,
    event_order_ok: bool,
    terminal_kind: Option<String>,
    provider_mode: Option<String>,
    fallback_reason: Option<String>,
    tool_names: Vec<String>,
    planned_children: usize,
    nested_children: usize,
    max_tool_result_chars: usize,
    assistant_chars: usize,
    first_token_latency_ms: Option<u64>,
}

const SSE_SCENARIOS: &[SseScenario] = &[
    SseScenario {
        name: "adapter-workspace",
        description: "验证第二 adapter 能稳定流出 started/delta/completed 事件，并覆盖 workspace 工具链路。",
        session_id: "sse-probe-workspace",
        expected_signal:
            "SSE 事件序列应完整，且应命中 workspace_list_files 或 workspace_read_file。",
        prompts: &[
            "当前文件夹中有哪些文件？",
            "继续，tauri.conf.json 是什么文件，都有哪些配置？",
        ],
        expected_any_tools: &["workspace_list_files", "workspace_read_file"],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: false,
    },
    SseScenario {
        name: "adapter-followup",
        description: "验证同一 session 上的 follow-up turn 能继续输出可诊断的 SSE 元信息。",
        session_id: "sse-probe-followup",
        expected_signal:
            "第二轮应继承上下文，并在 completed 事件中带出 provider_mode / fallback_reason。",
        prompts: &[
            "请用中文概括当前 Pony Agent 的 agent core 目标。",
            "继续上一轮，再补充当前是否发生了 provider fallback，以及原因是什么。",
        ],
        expected_any_tools: &[],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: false,
    },
    SseScenario {
        name: "adapter-multipath",
        description: "验证多路径上下文场景下，SSE 帧能暴露 planned/nested 子调用信号。",
        session_id: "sse-probe-multipath",
        expected_signal:
            "应命中 workspace_gather_context 或 workspace_batch，并在事件载荷里看见 planned/nested 工具活动。",
        prompts: &[r"请同时查看 src-tauri/src/agent/tools.rs、src-tauri/src/agent/planner.rs、src-tauri/src/agent/telemetry.rs，概括它们在 ToolPlan 流转中的角色，并指出各自最相关的函数。"],
        expected_any_tools: &["workspace_gather_context", "workspace_batch"],
        expect_planned_children: true,
        expect_nested_children: true,
        expect_large_tool_payload: false,
    },
    SseScenario {
        name: "adapter-large-result",
        description: "验证大结果场景下，SSE 汇总输出能显式暴露 tool result 体量与终态。",
        session_id: "sse-probe-large-result",
        expected_signal:
            "应命中 workspace_read_file_segment / workspace_gather_context / workspace_search_text 之一，并给出较大的结果体量信号。",
        prompts: &[r"请分析 src-tauri/src/agent/tools.rs 里 workspace_gather_context 的实现，给出关键流程与依据。文件较大时请自行选择分段读取、搜索或聚合上下文，不要省略判断依据。"],
        expected_any_tools: &[
            "workspace_read_file_segment",
            "workspace_gather_context",
            "workspace_search_text",
        ],
        expect_planned_children: false,
        expect_nested_children: false,
        expect_large_tool_payload: true,
    },
];

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--list") {
        print_sse_scenarios();
        return;
    }

    let raw = args.iter().any(|arg| arg == "--raw");
    let scenario_name = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .map(String::as_str)
        .unwrap_or("adapter-workspace");

    let selected = select_sse_scenarios(scenario_name);
    if selected.is_empty() {
        eprintln!(
            "未知场景 `{}`。可先运行 `cargo run --bin sse_turn_probe -- --list` 查看可用场景。",
            scenario_name
        );
        std::process::exit(2);
    }

    eprintln!("sse_turn_probe");
    eprintln!("用法: cargo run --bin sse_turn_probe -- [adapter-workspace|adapter-followup|adapter-multipath|adapter-large-result|all|--list] [--raw]");

    let mut runtime = AgentRuntime::new();
    for scenario in selected {
        run_sse_scenario(&mut runtime, scenario, raw);
    }
}

fn print_sse_scenarios() {
    eprintln!("sse_turn_probe 场景列表：");
    for scenario in SSE_SCENARIOS {
        eprintln!(
            "- {}: {} | 期望信号: {}",
            scenario.name, scenario.description, scenario.expected_signal
        );
    }
}

fn select_sse_scenarios(name: &str) -> Vec<&'static SseScenario> {
    if name == "all" {
        return SSE_SCENARIOS.iter().collect();
    }

    SSE_SCENARIOS
        .iter()
        .filter(|scenario| scenario.name == name)
        .collect()
}

fn run_sse_scenario(runtime: &mut AgentRuntime, scenario: &SseScenario, raw: bool) {
    eprintln!("\n=== sse scenario: {} ===", scenario.name);
    eprintln!("description: {}", scenario.description);
    eprintln!("session_id: {}", scenario.session_id);
    eprintln!("expected_signal: {}", scenario.expected_signal);

    let mut turn_summaries = Vec::new();
    for (index, prompt) in scenario.prompts.iter().enumerate() {
        let turn_id = format!("{}-turn-{}", scenario.session_id, index + 1);
        eprintln!("\n--- turn {} ---", index + 1);
        eprintln!("turn_id: {}", turn_id);
        eprintln!("user: {}", prompt);

        let frames = stream_turn(runtime, &turn_id, scenario.session_id, prompt);
        let summary = analyze_frames(&turn_id, &frames);
        print_turn_summary(&summary);
        if raw {
            print_raw_frames(&frames);
        }
        turn_summaries.push(summary);
    }

    print_scenario_summary(scenario, &turn_summaries);
}

fn stream_turn(
    runtime: &mut AgentRuntime,
    turn_id: &str,
    session_id: &str,
    message: &str,
) -> Vec<ParsedFrame> {
    let sink = BufferingSseTurnEventSink::new();
    runtime.start_turn_stream(
        &sink,
        turn_id.to_string(),
        TurnInput {
            message: message.to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some(session_id.to_string()),
            history: Vec::new(),
            images: Vec::new(),
        },
    );

    sink.frames()
        .into_iter()
        .map(|frame| parse_frame(frame))
        .collect()
}

fn parse_frame(raw: String) -> ParsedFrame {
    let mut event_name = String::new();
    let mut frame_id = None;
    let mut data_lines = Vec::new();

    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("event: ") {
            event_name = value.to_string();
        } else if let Some(value) = line.strip_prefix("id: ") {
            frame_id = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("data: ") {
            data_lines.push(value.to_string());
        }
    }

    let payload_text = data_lines.join("\n");
    let payload = serde_json::from_str::<TurnStreamEvent>(&payload_text).ok();

    ParsedFrame {
        event_name,
        frame_id,
        payload,
        raw,
    }
}

fn analyze_frames(expected_turn_id: &str, frames: &[ParsedFrame]) -> SseTurnSummary {
    let mut event_names = Vec::new();
    let mut delta_chunks = 0;
    let mut reasoning_chunks = 0;
    let mut turn_ids_consistent = true;
    let mut tool_names = BTreeSet::new();
    let mut planned_children = 0;
    let mut nested_children = 0;
    let mut max_tool_result_chars = 0;
    let mut terminal_kind = None;
    let mut provider_mode = None;
    let mut fallback_reason = None;
    let mut assistant_chars = 0;
    let mut first_token_latency_ms = None;

    for frame in frames {
        event_names.push(frame.event_name.clone());
        if frame.frame_id.as_deref() != Some(expected_turn_id) {
            turn_ids_consistent = false;
        }
        if frame.event_name == "turn:delta" {
            delta_chunks += 1;
        }
        if let Some(payload) = &frame.payload {
            if payload.reasoning_content.is_some() {
                reasoning_chunks += 1;
            }
            if let Some(mode) = &payload.provider_mode {
                provider_mode = Some(mode.clone());
            }
            if let Some(reason) = &payload.fallback_reason {
                fallback_reason = Some(reason.clone());
            }
            if let Some(text) = &payload.text {
                assistant_chars = assistant_chars.max(text.chars().count());
            }
            if let Some(latency) = payload.first_token_latency_ms {
                first_token_latency_ms = Some(latency);
            }
            if frame.event_name == "turn:completed" || frame.event_name == "turn:failed" {
                terminal_kind = Some(payload.kind.clone());
            }
            if let Some(activities) = &payload.tool_activities {
                for activity in activities {
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
            }
        } else {
            turn_ids_consistent = false;
        }
    }

    let event_order_ok = event_names
        .first()
        .map(|name| name == "turn:started")
        .unwrap_or(false)
        && event_names
            .last()
            .map(|name| name == "turn:completed" || name == "turn:failed")
            .unwrap_or(false);

    SseTurnSummary {
        frame_count: frames.len(),
        event_names,
        delta_chunks,
        reasoning_chunks,
        turn_ids_consistent,
        event_order_ok,
        terminal_kind,
        provider_mode,
        fallback_reason,
        tool_names: tool_names.into_iter().collect(),
        planned_children,
        nested_children,
        max_tool_result_chars,
        assistant_chars,
        first_token_latency_ms,
    }
}

fn print_turn_summary(summary: &SseTurnSummary) {
    eprintln!(
        "signal_summary: frames={} timeline={} delta_chunks={} reasoning_chunks={} turn_ids_consistent={} event_order_ok={} terminal_kind={} provider_mode={} fallback={} tools={} planned_children={} nested_children={} max_tool_result_chars={} assistant_chars={} first_token_latency_ms={:?}",
        summary.frame_count,
        compress_timeline(&summary.event_names),
        summary.delta_chunks,
        summary.reasoning_chunks,
        yes_no(summary.turn_ids_consistent),
        yes_no(summary.event_order_ok),
        summary.terminal_kind.as_deref().unwrap_or("none"),
        summary.provider_mode.as_deref().unwrap_or("none"),
        summary.fallback_reason.as_deref().unwrap_or("none"),
        format_tool_names(&summary.tool_names),
        summary.planned_children,
        summary.nested_children,
        summary.max_tool_result_chars,
        summary.assistant_chars,
        summary.first_token_latency_ms
    );
}

fn print_raw_frames(frames: &[ParsedFrame]) {
    println!("raw_frames_begin");
    for frame in frames {
        println!("{}", frame.raw);
    }
    println!("raw_frames_end");
}

fn print_scenario_summary(scenario: &SseScenario, summaries: &[SseTurnSummary]) {
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
    let all_terminal = summaries
        .iter()
        .all(|summary| summary.terminal_kind.is_some());
    let all_order_ok = summaries
        .iter()
        .all(|summary| summary.turn_ids_consistent && summary.event_order_ok);
    let provider_signal_ok = summaries.iter().all(|summary| {
        summary
            .provider_mode
            .as_deref()
            .map(|mode| mode == "live" || summary.fallback_reason.is_some())
            .unwrap_or(false)
    });

    let mut failed_checks = Vec::new();
    let mut warn_checks = Vec::new();

    if !all_terminal {
        failed_checks.push("存在未完成终态的 turn".to_string());
    }
    if !all_order_ok {
        failed_checks.push("SSE 事件顺序或 turn_id 一致性异常".to_string());
    }
    if !provider_signal_ok {
        failed_checks
            .push("completed/failed 事件未稳定暴露 provider_mode / fallback_reason".to_string());
    }
    if !scenario.expected_any_tools.is_empty()
        && !scenario
            .expected_any_tools
            .iter()
            .any(|tool| seen_tools.iter().any(|seen| seen == tool))
    {
        failed_checks.push(format!(
            "未命任一期望工具: {}",
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

fn collect_seen_tools(summaries: &[SseTurnSummary]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for summary in summaries {
        for tool in &summary.tool_names {
            seen.insert(tool.clone());
        }
    }
    seen.into_iter().collect()
}

fn compress_timeline(event_names: &[String]) -> String {
    if event_names.is_empty() {
        return "none".to_string();
    }

    let mut parts = Vec::new();
    let mut current = &event_names[0];
    let mut count = 1usize;
    for name in event_names.iter().skip(1) {
        if name == current {
            count += 1;
            continue;
        }
        parts.push(format_timeline_part(current, count));
        current = name;
        count = 1;
    }
    parts.push(format_timeline_part(current, count));
    parts.join(" -> ")
}

fn format_timeline_part(name: &str, count: usize) -> String {
    if count == 1 {
        name.to_string()
    } else {
        format!("{} x{}", name, count)
    }
}

fn format_tool_names(tool_names: &[String]) -> String {
    if tool_names.is_empty() {
        "none".to_string()
    } else {
        tool_names.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

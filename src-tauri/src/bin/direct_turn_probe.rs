#[path = "../agent/mod.rs"]
mod agent;

use agent::runtime::{AgentRuntime, TurnInput, TurnResult};
use agent::telemetry::{TurnToolActivity, TurnTraceStep};
use std::env;

struct DirectScenario {
    name: &'static str,
    description: &'static str,
    session_id: &'static str,
    expected_signal: &'static str,
    prompts: &'static [&'static str],
}

const DIRECT_SCENARIOS: &[DirectScenario] = &[
    DirectScenario {
        name: "workspace-tool",
        description: "验证 workspace 工具命中与文件读取链路。",
        session_id: "probe-workspace-tool",
        expected_signal: "第一轮应命中 workspace.list_files，第二轮应命中 workspace.read_file。",
        prompts: &[
            "当前文件夹中有哪些文件？",
            "这是什么文件？tauri.conf.json，都有哪些配置？",
        ],
    },
    DirectScenario {
        name: "history-carry",
        description: "验证 session history carry，第二轮不显式给文件名。",
        session_id: "probe-history-carry",
        expected_signal: "第二轮应基于上一轮历史继续命中 workspace.read_file。",
        prompts: &[
            "请记住 tauri.conf.json 这个文件，后面我会继续问它。",
            "继续说这个文件里都配置了什么。",
        ],
    },
    DirectScenario {
        name: "followup-fallback",
        description: "验证多轮 follow-up 与 provider/fallback 诊断信号。",
        session_id: "probe-followup-fallback",
        expected_signal:
            "若真实 provider 不可用，应在 provider_mode / fallback_reason 中直接体现。",
        prompts: &[
            "请用中文概括当前 Pony Agent 的 agent core 目标。",
            "继续上一轮，再补充当前是否发生了 provider fallback，以及原因是什么。",
        ],
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
    eprintln!("用法: cargo run --bin direct_turn_probe -- [all|workspace-tool|history-carry|followup-fallback|--list]");

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

    for (index, message) in scenario.prompts.iter().enumerate() {
        eprintln!("\n--- turn {} ---", index + 1);
        eprintln!("user: {}", message);

        let result = runtime.run_turn(TurnInput {
            message: (*message).to_string(),
            provider_id: None,
            model_id: None,
            session_id: Some(scenario.session_id.to_string()),
            history: vec![],
        });

        print_turn_result(&result);
    }
}

fn print_turn_result(result: &TurnResult) {
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
        "assistant_preview: {}",
        preview(&result.assistant_message, 280)
    );
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

fn preview(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized;
    }

    let compact = normalized.chars().take(max_chars).collect::<String>();
    format!("{}...(+{} chars)", compact, char_count - max_chars)
}

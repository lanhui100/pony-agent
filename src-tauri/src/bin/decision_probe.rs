#![allow(dead_code)]

use pony_agent_core::agent::config::ProviderRegistryStore;
use pony_agent_core::agent::provider::{
    ProviderManager, ProviderMessage, ProviderRequest, ProviderRequestObservation,
};
use pony_agent_core::agent::tools::{builtin_tools, ToolCall, ToolDefinition};
use std::env;

struct DecisionScenario {
    name: &'static str,
    description: &'static str,
    expected_signal: &'static str,
    prompts: &'static [&'static str],
}

const DECISION_SCENARIOS: &[DecisionScenario] = &[
    DecisionScenario {
        name: "workspace-tool",
        description: "观察 provider 原生 tools 决策是否能命中工作区类请求。",
        expected_signal: "tool_call 至少一次应为 workspace.list_files 或 workspace.read_file。",
        prompts: &[
            "当前文件夹中有哪些文件？",
            "继续，tauri.conf.json 是什么文件，都有哪些配置？",
        ],
    },
    DecisionScenario {
        name: "history-followup",
        description: "观察 provider 在携带上一轮历史时的 follow-up 行为。",
        expected_signal: "第二轮应延续上一轮语境，不需要重新解释上下文。",
        prompts: &[
            "请记住 tauri.conf.json 这个文件，后面我会继续问它。",
            "继续上一轮，补充这个文件里与桌面窗口相关的配置。",
        ],
    },
    DecisionScenario {
        name: "fallback-diagnosis",
        description: "观察真实 provider 不可用时的 fallback 诊断输出。",
        expected_signal: "provider_mode / fallback_reason / provider_error 应足以判断是否已退回 mock 或请求失败。",
        prompts: &[
            "请用一句话概括 Pony Agent 当前的 agent core 目标。",
            "继续上一轮，并说明当前是否需要 provider fallback 诊断。",
        ],
    },
];

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("--list")) {
        print_decision_scenarios();
        return;
    }

    let scenario_name = args.first().map(String::as_str).unwrap_or("all");
    let selected = select_decision_scenarios(scenario_name);
    if selected.is_empty() {
        eprintln!(
            "未知场景 `{}`。可先运行 `cargo run --bin decision_probe -- --list` 查看可用场景。",
            scenario_name
        );
        std::process::exit(2);
    }

    eprintln!("decision_probe");
    eprintln!("用法: cargo run --bin decision_probe -- [all|workspace-tool|history-followup|fallback-diagnosis|--list]");
    eprintln!(
        "available_tools: {}",
        builtin_tools()
            .iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
            .join(", ")
    );

    for scenario in selected {
        run_sequence("main-thread", scenario);

        let handle = std::thread::spawn(move || {
            run_sequence("worker-thread", scenario);
        });

        let _ = handle.join();
    }
}

fn print_decision_scenarios() {
    eprintln!("decision_probe 场景列表：");
    for scenario in DECISION_SCENARIOS {
        eprintln!(
            "- {}: {} | 期望信号: {}",
            scenario.name, scenario.description, scenario.expected_signal
        );
    }
}

fn select_decision_scenarios(name: &str) -> Vec<&'static DecisionScenario> {
    if name == "all" {
        return DECISION_SCENARIOS.iter().collect();
    }

    DECISION_SCENARIOS
        .iter()
        .filter(|scenario| scenario.name == name)
        .collect()
}

fn run_sequence(label: &str, scenario: &DecisionScenario) {
    eprintln!("\n=== decision scenario: {} / {} ===", scenario.name, label);
    eprintln!("description: {}", scenario.description);
    eprintln!("expected_signal: {}", scenario.expected_signal);

    let provider_store = ProviderRegistryStore::new();
    let provider = ProviderManager::new(provider_store.resolve_selection(None, None));
    let tools = builtin_tools();
    let mut conversation = vec![
        ProviderMessage::system(
            "你是 Pony Agent 的学习模式助手。请始终使用中文回答，并在需要时通过 provider 的原生工具调用能力请求工具。",
        ),
        ProviderMessage::developer(
            "当前会话摘要：Pony Agent 本地开发会话 / graph=state-machine-v1 / session=decision-probe",
        ),
    ];

    eprintln!(
        "provider: name={} protocol={} model={}",
        provider.name(),
        provider.protocol_label(),
        provider.model()
    );

    for (index, prompt) in scenario.prompts.iter().enumerate() {
        eprintln!("\n--- prompt {} ---", index + 1);
        eprintln!("user: {}", prompt);

        let mut input = conversation.clone();
        input.push(ProviderMessage::user((*prompt).to_string()));
        let request = ProviderRequest {
            model: provider.model().to_string(),
            input,
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        };

        match provider.decide_with_tools(&request, &tools) {
            Ok(decision) => {
                eprintln!("request_messages: {}", request.input.len());
                eprintln!("provider_mode: {}", decision.provider_mode);
                eprintln!(
                    "fallback_reason: {}",
                    decision.fallback_reason.as_deref().unwrap_or("none")
                );
                eprintln!(
                    "tool_call: {}",
                    format_tool_call(decision.tool_call.as_ref(), &tools)
                );
                eprintln!("output_preview: {}", preview(&decision.output_text, 220));

                conversation.push(ProviderMessage::user((*prompt).to_string()));
                let assistant_summary = if let Some(tool_call) = decision.tool_call.as_ref() {
                    format!(
                        "Provider requested tool {} with args {}",
                        tool_call.name,
                        preview(&tool_call.arguments.to_string(), 120)
                    )
                } else {
                    decision.output_text.clone()
                };
                conversation.push(ProviderMessage::developer(format!(
                    "Previous assistant reply: {}",
                    assistant_summary
                )));
            }
            Err(error) => {
                eprintln!("provider_error: {}", error);
                conversation.push(ProviderMessage::user((*prompt).to_string()));
                conversation.push(ProviderMessage::developer(format!(
                    "Previous assistant reply: provider_error={}",
                    error
                )));
            }
        }
    }
}

fn format_tool_call(call: Option<&ToolCall>, tools: &[ToolDefinition]) -> String {
    let Some(call) = call else {
        return "none".to_string();
    };

    let description = tools
        .iter()
        .find(|tool| tool.name == call.name)
        .map(|tool| tool.description)
        .unwrap_or("unknown tool");
    format!(
        "{} | {} | args={}",
        call.name,
        description,
        preview(&call.arguments.to_string(), 160)
    )
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

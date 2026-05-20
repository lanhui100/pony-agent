#[path = "../agent/mod.rs"]
mod agent;

use agent::runtime::{AgentRuntime, TurnInput};

fn main() {
    let mut runtime = AgentRuntime::new();
    let prompts = vec![
        "当前文件夹中有哪些文件？".to_string(),
        "这是什么文件？tauri.conf.json，都有哪些配置？".to_string(),
    ];

    for (index, message) in prompts.iter().enumerate() {
        eprintln!("\n=== probe turn {} ===", index + 1);
        eprintln!("user: {}", message);

        let result = runtime.run_turn(TurnInput {
            message: message.clone(),
            provider_id: None,
            model_id: None,
            session_id: Some("direct-turn-probe".to_string()),
            history: vec![],
        });

        eprintln!("phase: {}", result.phase);
        eprintln!("provider_requested_name: {}", result.provider_requested_name);
        eprintln!("provider_name: {}", result.provider_name);
        eprintln!("provider_protocol: {}", result.provider_protocol);
        eprintln!("provider_model: {}", result.provider_model);
        eprintln!("provider_mode: {}", result.provider_mode);
        eprintln!(
            "fallback_reason: {}",
            result.fallback_reason.as_deref().unwrap_or("none")
        );
        eprintln!(
            "token_usage: in={:?} out={:?} total={:?}",
            result.input_tokens, result.output_tokens, result.total_tokens
        );
        eprintln!("trace_steps: {}", result.trace_steps.len());
        eprintln!("tool_activities: {}", result.tool_activities.len());
        eprintln!("assistant: {}", result.assistant_message);
    }
}

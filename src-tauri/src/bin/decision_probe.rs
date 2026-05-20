#[path = "../agent/mod.rs"]
mod agent;

use agent::config::ProviderRegistryStore;
use agent::provider::{ProviderManager, ProviderMessage, ProviderRequest};
use agent::tools::builtin_tools;

fn main() {
    let prompts = vec![
        "当前文件夹中有哪些文件？".to_string(),
        "这是什么文件？tauri.conf.json，都有哪些配置？".to_string(),
    ];

    run_sequence("main-thread", prompts.clone());

    let handle = std::thread::spawn(move || {
        run_sequence("worker-thread", prompts);
    });

    let _ = handle.join();
}

fn run_sequence(label: &str, prompts: Vec<String>) {
    eprintln!("\n=== decision probe: {} ===", label);

    let provider_store = ProviderRegistryStore::new();
    let provider = ProviderManager::new(provider_store.resolve_selection(None, None));
    let tools = builtin_tools();

    for (index, prompt) in prompts.iter().enumerate() {
        eprintln!("\n--- prompt {} ---", index + 1);
        eprintln!("user: {}", prompt);

        let request = ProviderRequest {
            model: provider.model().to_string(),
            input: vec![
                ProviderMessage::system(
                    "你是 Pony Agent 的学习模式助手。请始终使用中文回答，并在需要时通过 provider 的原生工具调用能力请求工具。",
                ),
                ProviderMessage::developer(
                    "当前会话摘要：Pony Agent 本地开发会话 / graph=state-machine-v1 / session=local-dev-session",
                ),
                ProviderMessage::user(prompt.clone()),
            ],
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        };

        match provider.decide_with_tools(&request, &tools) {
            Ok(decision) => {
                eprintln!("provider_mode: {}", decision.provider_mode);
                eprintln!(
                    "fallback_reason: {}",
                    decision.fallback_reason.as_deref().unwrap_or("none")
                );
                eprintln!(
                    "tool_call: {}",
                    decision
                        .tool_call
                        .as_ref()
                        .map(|call| call.name.as_str())
                        .unwrap_or("none")
                );
                eprintln!("output_text: {}", decision.output_text);
            }
            Err(error) => {
                eprintln!("provider_error: {}", error);
            }
        }
    }
}

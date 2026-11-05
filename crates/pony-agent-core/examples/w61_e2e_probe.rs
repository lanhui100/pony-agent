//! Wave-61 端到端举证脚本（临时证据产物，不属于交付代码；跑完即删/移出仓库）。
//!
//! 目的：用**真实运行时 + 本地 mock provider**证明两件事：
//!  A. 主 turn 发出的 provider 请求里，`tools` 数组含 9 项团队/子代理工具，
//!     且 system 消息含团队策略锚串 `Agent Teams is available in this session`；
//!  B. 团队路径是**真执行**：`spawn_teammate_in_workspace` 派生的队友由
//!     `HostTurnEngine` 起真实子运行时并发出独立 provider 请求，回执能进 Lead 收件箱。
//!
//! 运行：`XDG_CONFIG_HOME=<tmp> XDG_DATA_HOME=<tmp> cargo run -p pony-agent-core --example w61_e2e_probe`

use pony_agent_core::agent::orchestration::{
    drain_inbox, inbox_pending_len, send_message_in_workspace, spawn_teammate_in_workspace,
    ReadInboxArgs, SendMessageArgs, SpawnTeammateArgs,
};
use pony_agent_core::agent::config::{
    ProviderConfigView, ProviderModelCapabilities, ProviderModelConfig, ProviderRegistryStore,
    ProviderRegistryView, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput};

struct MockResolver {
    port: u16,
}

impl ProviderSelectionResolver for MockResolver {
    fn resolve_provider_selection(
        &self,
        requested_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        let name = requested_id.unwrap_or("mock").to_string();
        ResolvedProviderSelection {
            requested_name: name.clone(),
            provider_name: name,
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: format!("http://127.0.0.1:{}/v1", self.port),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "MOCK_KEY".to_string(),
            api_key: Some("mock-key".to_string()),
            model: "mock-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 256,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities::default(),
            thinking_param_pattern: ThinkingParamPattern::None,
        }
    }
}
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = rt.enter();
    std::env::set_var("MOCK_KEY", "mock-key");
    let root = std::env::temp_dir().join(format!("w61-e2e-{}", std::process::id()));
    let cfg = std::env::var("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| root.join("config"));
    std::fs::create_dir_all(cfg.join("pony-agent")).expect("cfg dir");
    std::fs::create_dir_all(root.join("ws")).expect("ws dir");
    std::fs::create_dir_all(root.join("data")).expect("data dir");

    // ---- 本地 mock provider（OpenAI completions SSE）----
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let port = listener.local_addr().unwrap().port();
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_bg = captured.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            // 读到 header 结束
            let mut header_end = None;
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let Some(header_end) = header_end else { continue };
            let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_len = head
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            while buf.len() < header_end + content_len {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(_) => break,
                }
            }
            let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
            {
                let mut g = captured_bg.lock().unwrap();
                g.push(body.clone());
                eprintln!("MOCK_REQ n={} bytes={} is_teammate={}", g.len(), body.len(), body.contains("teammate"));
            }

            let body_json = concat!(
                "{\"id\":\"probe-1\",\"object\":\"chat.completion\",\"model\":\"mock-model\",",
                "\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"probe-ok\"},",
                "\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,",
                "\"total_tokens\":12}}"
            );
            let sse = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_json.len(),
                body_json
            );
            let _ = stream.write_all(sse.as_bytes());
            let _ = stream.flush();
        }
    });

    // ---- providers.json（用应用自身 API 写入，保证 schema 正确；子代理运行时读同一份）----
    let store = ProviderRegistryStore::with_path(cfg.join("pony-agent").join("providers.json"));
    let view = ProviderRegistryView {
        providers: vec![ProviderConfigView {
            id: "mock".to_string(),
            name: "Mock".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "MOCK_KEY".to_string(),
            api_key_value: "mock-key".to_string(),
            api_key_present: true,
            supported_protocols: vec![],
            endpoints: vec![],
            models: vec![ProviderModelConfig {
                id: "mock-model".to_string(),
                name: "Mock".to_string(),
                model: "mock-model".to_string(),
                temperature: 0.0,
                max_output_tokens: 256,
                capability_preset: Default::default(),
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                protocol: None,
                base_url: None,
                capabilities: Default::default(),
            }],
            selected_model_id: Some("mock-model".to_string()),
        }],
        selected_provider_id: Some("mock".to_string()),
    };
    let saved = store.save_view(view);
    println!("STEP0b providers_saved={}", saved.is_ok());

    // ---- 真实运行时（C2：build() 内注册 HostTurnEngine）----
    let ws = root.join("ws");
    let runtime = AgentRuntimeBuilder::new()
        .workspace_root(ws.clone())
        .provider_resolver(Box::new(MockResolver { port }))
        .build();
    let engine_wired = pony_agent_core::agent::subagent_runtime::subagent_engine().is_some();
    println!("STEP0 engine_wired={engine_wired}");

    // ---- A：主 turn 的对外请求 ----
    let input = TurnInput {
        message: "用 dev team 帮我做这件事".to_string(),
        display_message: None,
        provider_id: Some("mock".to_string()),
        model_id: Some("mock-model".to_string()),
        reasoning_effort: None,
        workspace_mode: None,
        session_id: Some(format!("w61-e2e-{}", std::process::id())),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
        tool_authorization_mode: None,
    };
    let turn = runtime.run_turn(input);
    let turn_phase = turn.phase.clone(); let turn_msg_len = turn.assistant_message.len(); println!("STEP1 phase={turn_phase} assistant_len={turn_msg_len} assistant_preview={:?}", turn.assistant_message.chars().take(120).collect::<String>());
    println!("STEP1b tool_activities={:?}", turn.tool_activities.iter().map(|a| a.name.clone()).collect::<Vec<_>>());

    let first = captured.lock().unwrap().first().cloned().unwrap_or_default();
    let missing: Vec<&str> = TEAM_TOOLS
        .iter()
        .copied()
        .filter(|name| !first.contains(&format!("\"{name}\"")))
        .collect();
    let has_policy = first.contains("Agent Teams is available in this session");
    let has_run = first.contains("\"Run\"");
    println!("STEP2 request_bytes={} missing_team_tools={:?} policy_in_system={} has_Run={}", first.len(), missing, has_policy, has_run);

    // ---- B：团队路径的真实执行 ----
    let spawn = spawn_teammate_in_workspace(
        SpawnTeammateArgs {
            name: "tester".to_string(),
            role: Some("tester".to_string()),
            description: "probe teammate".to_string(),
            prompt: "probe: 报告一句话".to_string(),
            context: None,
            provider_id: None,
            model_id: None,
        },
        Some(ws.as_path()),
    );
    println!("STEP3 spawn_ok={}", spawn.is_ok());

    let send = send_message_in_workspace(
        SendMessageArgs {
            target: "tester".to_string(),
            message: "probe-second-message".to_string(),
            mode: None,
        },
        Some(ws.as_path()),
    );
    println!("STEP4 send_ok={}", send.is_ok());

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last_state = String::new();
    let mut lead_replies: Vec<String> = Vec::new();
    while Instant::now() < deadline {
        let pending = inbox_pending_len("tester");
        let got = drain_inbox(ReadInboxArgs {
            target: "lead".to_string(),
        })
        .map(|r| r.messages.into_iter().map(|m| m.content).collect::<Vec<_>>())
        .unwrap_or_default();
        if !got.is_empty() {
            lead_replies.extend(got);
            break;
        }
        let st = format!(
            "pending={} driving={} requests={}",
            pending,
            pony_agent_core::agent::orchestration::is_driving("tester"),
            captured.lock().unwrap().len()
        );
        if st != last_state {
            println!("POLL {st}");
            last_state = st;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let requests = captured.lock().unwrap().len();
    let teammate_requests = captured
        .lock()
        .unwrap()
        .iter()
        .filter(|b| b.contains("teammate 'tester'") || b.contains("probe-second-message"))
        .count();
    println!(
        "STEP5 provider_requests={requests} teammate_requests={teammate_requests} lead_replies={lead_replies:?}"
    );

    // ---- 判定 ----
    let ok = engine_wired
        && turn_msg_len > 0
        && missing.is_empty()
        && has_policy
        && spawn.is_ok()
        && send.is_ok()
        && teammate_requests >= 1
        && !lead_replies.is_empty();
    println!("RESULT={}", if ok { "E2E_OK" } else { "E2E_FAIL" });
    let _ = std::fs::remove_dir_all(&root);
    if !ok {
        std::process::exit(1);
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

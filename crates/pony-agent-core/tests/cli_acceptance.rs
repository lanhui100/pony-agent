// wave-13：本测试把 `src/bin/cli.rs` 以模块方式 include 进来（bin-as-module），
// 而 bin 的 `main` 不会被调用，导致该模块内仅被 `main` 链路使用的 item 被判为 `dead_code`。
// 这些 item 在真正的 bin 目标（`cargo check --bins`，同样受 `-D warnings` 约束）中已被完整校验，
// 故此处仅对「bin-as-module 固有产物」加最小 allow；不使用 `#![allow(warnings)]`。
// 注意：该 allow 只作用于 `mod cli` 内部，**不会**掩盖本测试文件自身的真实 unused import。
#[path = "../src/bin/cli.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod cli;

use cli::parse_cli_args;
use std::path::PathBuf;

#[test]
fn test_cli_args_parsing_defaults() {
    let args = vec!["pony-cli".to_string()];
    let parsed = parse_cli_args(&args).expect("should parse defaults");
    assert_eq!(parsed.model, "gemini-3.8-flash");
    assert_eq!(parsed.gateway_url, "http://10.42.0.67:8080");
    assert!(parsed.workspace.is_absolute());
    assert!(parsed.prompt.is_none());
}

#[test]
fn test_cli_args_parsing_custom() {
    let args = vec![
        "pony-cli".to_string(),
        "-p".to_string(),
        "hello agent".to_string(),
        "-m".to_string(),
        "deepseek-flash".to_string(),
        "-g".to_string(),
        "http://127.0.0.1:8080".to_string(),
        "-w".to_string(),
        "/tmp".to_string(),
        "--key".to_string(),
        "sk-test".to_string(),
    ];
    let parsed = parse_cli_args(&args).expect("should parse custom args");
    assert_eq!(parsed.prompt.as_deref(), Some("hello agent"));
    assert_eq!(parsed.model, "deepseek-flash");
    assert_eq!(parsed.gateway_url, "http://127.0.0.1:8080");
    assert_eq!(parsed.workspace, PathBuf::from("/tmp"));
    assert_eq!(parsed.api_key.as_deref(), Some("sk-test"));
}

#[test]
fn test_cli_args_parsing_subcommand_init() {
    let args = vec!["pony".to_string(), "init".to_string(), "-w".to_string(), "/tmp".to_string()];
    let parsed = parse_cli_args(&args).expect("should parse init");
    assert_eq!(parsed.subcommand, cli::SubCommand::Init);
    assert_eq!(parsed.workspace, PathBuf::from("/tmp"));
}

#[test]
fn test_cli_args_parsing_continue_and_resume() {
    let args_continue = vec!["pony".to_string(), "-c".to_string()];
    let parsed_c = parse_cli_args(&args_continue).expect("should parse -c");
    assert!(parsed_c.continue_last);

    let args_resume = vec!["pony".to_string(), "-r".to_string(), "sess-123".to_string()];
    let parsed_r = parse_cli_args(&args_resume).expect("should parse -r");
    assert_eq!(parsed_r.resume_session_id.as_deref(), Some("sess-123"));
}

#[test]
fn test_slash_command_parsing() {
    use cli::SlashCommand;
    assert_eq!(SlashCommand::parse("/clear"), Some(SlashCommand::Clear));
    assert_eq!(SlashCommand::parse("/compact"), Some(SlashCommand::Compact));
    // wave-slash-1（契约 C-4）：`/cost` 已从 CLI 与 TUI 命令表移除（金额/配额核算为非目标），
    // 因此它必须退化为普通未知斜杠输入，而不是被静默吞掉。
    assert_eq!(
        SlashCommand::parse("/cost"),
        Some(SlashCommand::Unknown("/cost".to_string()))
    );
    assert_eq!(SlashCommand::parse("/help"), Some(SlashCommand::Help));
    assert_eq!(SlashCommand::parse("/exit"), Some(SlashCommand::Exit));
    assert_eq!(SlashCommand::parse("/quit"), Some(SlashCommand::Exit));
    assert_eq!(SlashCommand::parse("/unknown"), Some(SlashCommand::Unknown("/unknown".to_string())));
    assert_eq!(SlashCommand::parse("hello world"), None);
}

// ── `ponycli models`：逐模型上下文窗口配置面（与 TUI `/models` 的 `w` 同一真源）────────
//
// 纪律：用**真实** `ProviderRegistryStore`（临时目录）+ 真实落盘内容断言，
// 不 mock 成功；「改一个不动别人」「清除=删键」是两条用户可见承诺，必须有反向看守。

/// 夹具：一个 provider 两个模型（`model-declared` 显式 200000；`model-undeclared` 未声明）。
fn models_fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pony-cli-window-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let path = root.join("providers.json");
    let raw = serde_json::json!({
        "providers": [{
            "id": "provider-demo",
            "name": "Demo",
            "protocol": "openai-completions",
            "base_url": "https://example.test/v1",
            "api_key_env_var": "DEMO_API_KEY",
            "models": [
                {
                    "id": "model-declared",
                    "name": "Declared 200K",
                    "model": "declared-200k",
                    "temperature": 0.2,
                    "maxOutputTokens": 64000,
                    "capabilityPreset": "open-ai-chat",
                    "capabilities": { "contextWindowTokens": 200000 }
                },
                {
                    "id": "model-undeclared",
                    "name": "Undeclared",
                    "model": "undeclared-1m",
                    "temperature": 0.2,
                    "maxOutputTokens": 64000,
                    "capabilityPreset": "open-ai-chat"
                }
            ],
            "selectedModelId": "model-declared"
        }],
        "selectedProviderId": "provider-demo"
    });
    std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).expect("write fixture");
    path
}

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn test_models_command_is_opt_in_and_parses_both_actions() {
    // 不是 models 子命令 ⇒ 不拦截（其它命名空间照旧解析）。
    assert_eq!(
        cli::parse_models_command(&argv(&["ponycli", "init"])).expect("not models"),
        None
    );

    assert_eq!(
        cli::parse_models_command(&argv(&["ponycli", "models", "list"])).expect("list"),
        Some(cli::ModelsCommand::List { provider: None })
    );
    assert_eq!(
        cli::parse_models_command(&argv(&["ponycli", "models", "list", "--provider", "p1"]))
            .expect("list filtered"),
        Some(cli::ModelsCommand::List {
            provider: Some("p1".to_string())
        })
    );
    assert_eq!(
        cli::parse_models_command(&argv(&[
            "ponycli",
            "models",
            "set-window",
            "--provider",
            "p1",
            "--model",
            "m1",
            "--window",
            "200K",
        ]))
        .expect("set-window"),
        Some(cli::ModelsCommand::SetWindow {
            provider: "p1".to_string(),
            model: "m1".to_string(),
            window: Some(200_000),
        })
    );
    // `clear` 是「删掉声明、恢复跟随预设」——不是写 0。
    assert_eq!(
        cli::parse_models_command(&argv(&[
            "ponycli",
            "models",
            "set-window",
            "--provider",
            "p1",
            "--model",
            "m1",
            "--window",
            "clear",
        ]))
        .expect("clear"),
        Some(cli::ModelsCommand::SetWindow {
            provider: "p1".to_string(),
            model: "m1".to_string(),
            window: None,
        })
    );
}

#[test]
fn test_models_command_rejects_incomplete_or_unknown_arguments() {
    let missing = cli::parse_models_command(&argv(&[
        "ponycli",
        "models",
        "set-window",
        "--provider",
        "p1",
        "--model",
        "m1",
    ]))
    .expect_err("missing --window");
    assert!(missing.contains("--window"), "{missing}");

    let unknown = cli::parse_models_command(&argv(&["ponycli", "models", "nope"]))
        .expect_err("unknown action");
    assert!(unknown.contains("未知"), "{unknown}");

    let invalid_window = cli::parse_models_command(&argv(&[
        "ponycli",
        "models",
        "set-window",
        "--provider",
        "p1",
        "--model",
        "m1",
        "--window",
        "200KB",
    ]))
    .expect_err("invalid window");
    assert!(!invalid_window.is_empty(), "解析失败必须带原因");
}

#[test]
fn test_models_list_reports_declared_and_effective_window_per_model() {
    let path = models_fixture("list");
    let store = pony_agent_core::agent::config::ProviderRegistryStore::with_path(&path);

    let reports = cli::execute_models_command(&store, &cli::ModelsCommand::List { provider: None })
        .expect("list");
    assert_eq!(reports.len(), 2, "两个模型都要出现在报告里：{reports:?}");

    let declared = reports
        .iter()
        .find(|report| report["modelId"] == "model-declared")
        .expect("declared report");
    assert_eq!(declared["declaredContextWindowTokens"], serde_json::json!(200_000));
    assert_eq!(declared["contextWindowTokens"], serde_json::json!(200_000));
    assert_eq!(declared["source"], "declared");
    assert_eq!(declared["capabilityPreset"], "open-ai-chat");

    let inherited = reports
        .iter()
        .find(|report| report["modelId"] == "model-undeclared")
        .expect("inherited report");
    assert_eq!(
        inherited["declaredContextWindowTokens"],
        serde_json::Value::Null,
        "未声明必须是 null，不能与「声明了 262144」混同"
    );
    assert_eq!(
        inherited["contextWindowTokens"],
        serde_json::json!(pony_agent_core::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS)
    );
    assert_eq!(inherited["source"], "preset");
}

#[test]
fn test_models_set_window_touches_one_key_only() {
    let path = models_fixture("set");
    let store = pony_agent_core::agent::config::ProviderRegistryStore::with_path(&path);

    let reports = cli::execute_models_command(
        &store,
        &cli::ModelsCommand::SetWindow {
            provider: "provider-demo".to_string(),
            model: "undeclared-1m".to_string(),
            window: Some(1_000_000),
        },
    )
    .expect("set window");
    assert_eq!(reports[0]["modelId"], "model-undeclared");
    assert_eq!(reports[0]["contextWindowTokens"], serde_json::json!(1_000_000));
    assert_eq!(reports[0]["source"], "declared");

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
    assert_eq!(
        raw["providers"][0]["models"][1]["capabilities"],
        serde_json::json!({ "contextWindowTokens": 1_000_000 }),
        "未声明的模型只应长出这一个键"
    );
    assert_eq!(
        raw["providers"][0]["models"][0]["capabilities"]["contextWindowTokens"],
        serde_json::json!(200_000),
        "另一个模型的声明值不得被顺带改写"
    );
    // 清除声明 ⇒ 删键（不是写默认值）。
    cli::execute_models_command(
        &store,
        &cli::ModelsCommand::SetWindow {
            provider: "provider-demo".to_string(),
            model: "undeclared-1m".to_string(),
            window: None,
        },
    )
    .expect("clear window");
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
    assert!(raw["providers"][0]["models"][1]["capabilities"]
        .get("contextWindowTokens")
        .is_none());
}

#[test]
fn test_models_set_window_fails_closed_on_unknown_model() {
    let path = models_fixture("unknown");
    let store = pony_agent_core::agent::config::ProviderRegistryStore::with_path(&path);
    let before = std::fs::read_to_string(&path).expect("read");

    let error = cli::execute_models_command(
        &store,
        &cli::ModelsCommand::SetWindow {
            provider: "provider-demo".to_string(),
            model: "no-such-model".to_string(),
            window: Some(200_000),
        },
    )
    .expect_err("unknown model must fail");
    assert!(error.contains("未找到模型"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        before,
        "失败路径绝不落盘"
    );
}

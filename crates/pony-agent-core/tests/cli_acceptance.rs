#[path = "../src/bin/cli.rs"]
mod cli;

use cli::{parse_cli_args, CliArgs};
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
    assert_eq!(SlashCommand::parse("/cost"), Some(SlashCommand::Cost));
    assert_eq!(SlashCommand::parse("/help"), Some(SlashCommand::Help));
    assert_eq!(SlashCommand::parse("/exit"), Some(SlashCommand::Exit));
    assert_eq!(SlashCommand::parse("/quit"), Some(SlashCommand::Exit));
    assert_eq!(SlashCommand::parse("/unknown"), Some(SlashCommand::Unknown("/unknown".to_string())));
    assert_eq!(SlashCommand::parse("hello world"), None);
}

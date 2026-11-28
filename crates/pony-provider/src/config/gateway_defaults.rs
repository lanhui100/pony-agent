//! 网关兜底面的单源实现（`pony-provider::config` 的子模块）。
//!
//! 这里的两个函数都只服务同一条退路：「注册表里没有可用 provider 时怎么办」。
//! 它们此前在 `pony-tui/src/tui.rs` 与 `pony-cli`（`main.rs` + `lib.rs`）里各有
//! 一份逐字重复的实现，2026-11-28 收敛到这里（见同批 ADR
//! `.agents/notes/implemented/simplification/2026-11-28-path-resolution-single-source.md`）。
//!
//! 边界：本模块**只**固化形状与读取顺序，不做选择策略。两个 bin 的策略仍然各自持有
//! （TUI 注册表优先 / CLI 恒走网关），差异属对外行为，未经裁决不得在此合并。

use std::path::PathBuf;

use crate::provider::{ProviderAuthType, ProviderProtocol};

use super::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
    DEFAULT_CONTEXT_WINDOW_TOKENS,
};

/// 网关 API Key 的解析链：`PONYLLM_API_KEY` > `PONY_API_KEY` > 本地遗留
/// `~/.config/ponyllm/ponyllm.toml` 的 `api_key = "..."`。
///
/// 为什么是单源：TUI（`build_control_plane`）与 CLI（`CliArgs::default`）此前各写一份
/// 逐字相同的实现，两条链一旦漂移，同机同配置下 `pony` 与 `ponycli` 会拿不同的 key
/// （一个 401 一个 200），而没有任何测试会红。
///
/// 语义边界：**只读**遗留 toml，不写回、不做迁移（迁移属配置面决策，须单独立项）；
/// 文件不存在/不可读一律返回 `None`，绝不 panic、绝不打印路径。
pub fn gateway_api_key_from_env_or_legacy_config() -> Option<String> {
    std::env::var("PONYLLM_API_KEY")
        .or_else(|_| std::env::var("PONY_API_KEY"))
        .ok()
        .or_else(|| {
            let cfg_path = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
                .join("ponyllm")
                .join("ponyllm.toml");
            let content = std::fs::read_to_string(cfg_path).ok()?;
            content.lines().find_map(|line| {
                let trimmed = line.trim();
                if !trimmed.starts_with("api_key") {
                    return None;
                }
                trimmed
                    .split('=')
                    .nth(1)
                    .map(|val| val.trim().trim_matches('"').to_string())
            })
        })
}


/// 构造「网关兜底」选择结果：注册表里没有该 provider 时，TUI 与 CLI 都回落到
/// `gateway_url` + 环境变量 key 的硬编码形状。
///
/// 为什么抽出来：这段构造（base_url 补 `/v1`、固定 `ponyllm` 命名、温度/输出上限、
/// 能力位、`ThinkingParamPattern::None`）在两个 bin 里逐字重复了约 30 行。**选择策略**
/// 仍然各自持有（TUI 是注册表优先，CLI 当前恒走网关——两者语义差异见 backlog
/// B1795839268766，改动需人工裁决），本函数只固化「兜底形状」这一份事实。
pub fn gateway_fallback_selection(
    gateway_url: &str,
    api_key: Option<String>,
    model: &str,
) -> ResolvedProviderSelection {
    let mut base = gateway_url.trim_end_matches('/').to_string();
    if !base.ends_with("/v1") {
        base.push_str("/v1");
    }

    ResolvedProviderSelection {
        requested_name: "ponyllm".to_string(),
        provider_name: "ponyllm".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: base,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "PONYLLM_API_KEY".to_string(),
        api_key,
        model: model.to_string(),
        temperature: 0.2,
        max_output_tokens: 4096,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: false,
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    }
}

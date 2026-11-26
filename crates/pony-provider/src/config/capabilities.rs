//! 能力目录与模型能力声明（从 `agent/config.rs` 抽出的独立单元）。
//!
//! 为什么独立：`config.rs` 的增长主要来自「注册表读写 + 持久化 + 一堆纯声明」混在一个文件里，
//! 文件行数棘轮门（size:check，≤3000 行/文件）因此反复被顶到基线边缘。本模块只保留
//! 「模型支持什么」这一层纯数据与查表逻辑，不碰文件 IO、不碰注册表可变状态。
// 抽取注记：这三项在 `config.rs` 里是「已计入基线」的存量（棘轮按 (lint, 文件) 冻结计数）；
// 搬到新文件后不带声明就会被记成「新增债」。故在新家显式 expect，理由与原语义一致。
#![expect(
    missing_debug_implementations,
    reason = "能力声明是纯数据快照，不进日志/不参与错误链，Debug 对调用方无价值"
)]
#![expect(
    clippy::derivable_impls,
    reason = "ProviderCapabilityPreset 的 Default 显式写出 DeepseekChat，比 derive 的隐式首变体更达意"
)]
#![expect(
    clippy::struct_excessive_bools,
    reason = "模型能力本就是一组布尔开关；拆子结构会改变 wire 形状，得不偿失"
)]

use super::{default_true, DEFAULT_CONTEXT_WINDOW_TOKENS};
use serde::{Deserialize, Serialize};

pub struct CapabilityCatalogEntry {
    pub protocol: Option<&'static str>,
    pub patterns: &'static [&'static str],
    pub preset: ProviderCapabilityPreset,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderReasoningEffort {
    Low,
    Medium,
    High,
    Max,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderCapabilityPreset {
    Auto,
    OpenAiChat,
    OpenAiReasoning,
    AnthropicThinking,
    DeepseekChat,
    DeepseekReasoner,
    Custom,
}

impl Default for ProviderCapabilityPreset {
    fn default() -> Self {
        Self::Auto
    }
}

pub const CAPABILITY_CATALOG: &[CapabilityCatalogEntry] = &[
    CapabilityCatalogEntry {
        protocol: Some("anthropic"),
        patterns: &["claude-3-7", "claude-sonnet-4", "claude-opus-4"],
        preset: ProviderCapabilityPreset::AnthropicThinking,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["gpt-5", "gpt-5.4", "gpt-5.5", "o1", "o3", "reason"],
        preset: ProviderCapabilityPreset::OpenAiReasoning,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["gpt-4.1", "vision"],
        preset: ProviderCapabilityPreset::OpenAiChat,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["deepseek-reasoner", "deepseek-r1", "deepseek-v4-pro"],
        preset: ProviderCapabilityPreset::DeepseekReasoner,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["deepseek-chat", "deepseek-v4-flash"],
        preset: ProviderCapabilityPreset::DeepseekChat,
    },
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCapabilities {
    pub context_window_tokens: Option<u32>,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_image_input: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub supports_video_input: bool,
    #[serde(default)]
    pub supports_audio_input: bool,
    #[serde(default)]
    pub supports_image_output: bool,
    #[serde(default)]
    pub supports_video_output: bool,
    #[serde(default)]
    pub supports_audio_output: bool,
}

impl Default for ProviderModelCapabilities {
    fn default() -> Self {
        Self {
            context_window_tokens: Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: true,
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        }
    }
}

/// `capabilities` 整段缺席时的「未声明」哨兵：窗口为 `None`，其余布尔能力取默认值。
///
/// 必须与 [`ProviderModelCapabilities::default`] 区分开：`default()` 的
/// `context_window_tokens` 是**补缺值**（1M 默认窗口）；若把它当成声明值，
/// 「用户从未填过窗口」就与「用户填了 1000000」不可分辨，预设再也补不了缺。
pub fn undeclared_model_capabilities() -> ProviderModelCapabilities {
    ProviderModelCapabilities {
        context_window_tokens: None,
        ..ProviderModelCapabilities::default()
    }
}

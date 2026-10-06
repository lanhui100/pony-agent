//! 统一错误码注册表骨架（0021 方案 B，task-2）。
//!
//! 只新增不改旧分支：`retry.rs classify` / `DispatchError` / `ToolError` /
//! provider 重试链行为零变更。本模块提供 `ErrorCode` 枚举 + wire 兼容的
//! `as_str` / `from_str` + 分类 + `unknown:*` 包装，以及
//! `CapabilityFailureKind` / `SkillFailureLayer` 的 `From` 映射。

use crate::agent::capability_bridge::{CapabilityFailureKind, SkillFailureLayer};

/// 错误码分层前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Tool,
    Upstream,
    Protocol,
    Capability,
    Skill,
    Hook,
    Turn,
    Unknown,
}

impl ErrorCategory {
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Tool => "tool_",
            Self::Upstream => "upstream_",
            Self::Protocol => "protocol_",
            Self::Capability => "capability_",
            Self::Skill => "skill_",
            Self::Hook => "hook_",
            Self::Turn => "turn_",
            Self::Unknown => "unknown:",
        }
    }
}

/// 统一错误码注册表。
///
/// 线码约束（error-contract C6.2）：`upstream_timeout` 的线码冻结为 `timeout`，
/// 不得改名（`docs/architecture/runtime.md` 收口行为依赖）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    // ── tool_*（调度执行；以 dispatcher/tools 现有码为首批） ──
    ToolUnknownDescriptor,
    ToolOriginNotAuthorized,
    ToolTurnViewSnapshotMismatch,
    ToolInvalidArguments,
    ToolPreDispatchHookError,
    ToolSandboxUnavailable,
    ToolSandboxDenied,
    ToolNoHandlerRegistered,
    ToolOutputBudgetExceeded,
    ToolBudgetExhausted,
    ToolTimeout,
    ToolCancelled,
    ToolInteractionUnavailable,
    ToolHandlerError,
    // ── upstream_*（LLM/出网上游；C3 八类） ──
    /// 线码 `timeout`（冻结别名）。
    UpstreamTimeout,
    UpstreamRateLimited,
    UpstreamServerError,
    UpstreamAuth,
    UpstreamQuota,
    UpstreamContextOverflow,
    UpstreamNetwork,
    UpstreamBadRequest,
    // ── protocol_*（协议不匹配；C4 五类） ──
    ProtocolSseParse,
    ProtocolToolCallSchema,
    ProtocolModelCatalog,
    ProtocolUnknownModel,
    ProtocolReasoningEffort,
    ProtocolEventVersion,
    // ── capability_*（C2.4；CapabilityFailureKind 映射目标） ──
    CapabilitySourceUnavailable,
    CapabilityPermissionDenied,
    CapabilityOutOfScope,
    CapabilityMalformedResponse,
    CapabilityInvocationFailed,
    CapabilityNotFound,
    // ── skill_*（C2.4；SkillFailureLayer 映射目标） ──
    SkillResolution,
    SkillSourceUnavailable,
    SkillPermissionDenied,
    SkillMalformedComposition,
    SkillUnsupportedComposition,
    SkillUnderlyingCapabilityExecution,
    // ── hook_*（C5.2） ──
    HookFailTurn,
    HookDegraded,
    HookIgnored,
    // ── turn_*（C5.1/C5.3；recovery 收口以 turn_recovery_ 开头） ──
    TurnFailed,
    TurnCancelled,
    TurnHopLimit,
    TurnFollowupLimit,
    TurnRepeatedFailure,
    TurnRecoveryLocalFallback,
    // ── unknown:*（C7.3 Fail-Closed 包装，原样保留） ──
    Unknown(String),
}

impl ErrorCode {
    /// 线码（wire `String` 取值）。
    pub fn as_str(&self) -> String {
        match self {
            Self::ToolUnknownDescriptor => "tool_unknown_descriptor".to_string(),
            Self::ToolOriginNotAuthorized => "tool_origin_not_authorized".to_string(),
            Self::ToolTurnViewSnapshotMismatch => {
                "tool_turn_view_snapshot_mismatch".to_string()
            }
            Self::ToolInvalidArguments => "tool_invalid_arguments".to_string(),
            Self::ToolPreDispatchHookError => "tool_pre_dispatch_hook_error".to_string(),
            Self::ToolSandboxUnavailable => "tool_sandbox_unavailable".to_string(),
            Self::ToolSandboxDenied => "tool_sandbox_denied".to_string(),
            Self::ToolNoHandlerRegistered => "tool_no_handler_registered".to_string(),
            Self::ToolOutputBudgetExceeded => "tool_output_budget_exceeded".to_string(),
            Self::ToolBudgetExhausted => "tool_budget_exhausted".to_string(),
            Self::ToolTimeout => "tool_timeout".to_string(),
            Self::ToolCancelled => "tool_cancelled".to_string(),
            Self::ToolInteractionUnavailable => "tool_interaction_unavailable".to_string(),
            Self::ToolHandlerError => "handler_error".to_string(),
            Self::UpstreamTimeout => "timeout".to_string(),
            Self::UpstreamRateLimited => "upstream_rate_limited".to_string(),
            Self::UpstreamServerError => "upstream_server_error".to_string(),
            Self::UpstreamAuth => "upstream_auth".to_string(),
            Self::UpstreamQuota => "upstream_quota".to_string(),
            Self::UpstreamContextOverflow => "upstream_context_overflow".to_string(),
            Self::UpstreamNetwork => "upstream_network".to_string(),
            Self::UpstreamBadRequest => "upstream_bad_request".to_string(),
            Self::ProtocolSseParse => "protocol_sse_parse".to_string(),
            Self::ProtocolToolCallSchema => "protocol_tool_call_schema".to_string(),
            Self::ProtocolModelCatalog => "protocol_model_catalog".to_string(),
            Self::ProtocolUnknownModel => "protocol_unknown_model".to_string(),
            Self::ProtocolReasoningEffort => "protocol_reasoning_effort".to_string(),
            Self::ProtocolEventVersion => "protocol_event_version".to_string(),
            Self::CapabilitySourceUnavailable => "capability_source_unavailable".to_string(),
            Self::CapabilityPermissionDenied => "capability_permission_denied".to_string(),
            Self::CapabilityOutOfScope => "capability_out_of_scope".to_string(),
            Self::CapabilityMalformedResponse => "capability_malformed_response".to_string(),
            Self::CapabilityInvocationFailed => "capability_invocation_failed".to_string(),
            Self::CapabilityNotFound => "capability_not_found".to_string(),
            Self::SkillResolution => "skill_resolution".to_string(),
            Self::SkillSourceUnavailable => "skill_source_unavailable".to_string(),
            Self::SkillPermissionDenied => "skill_permission_denied".to_string(),
            Self::SkillMalformedComposition => "skill_malformed_composition".to_string(),
            Self::SkillUnsupportedComposition => "skill_unsupported_composition".to_string(),
            Self::SkillUnderlyingCapabilityExecution => {
                "skill_underlying_capability_execution".to_string()
            }
            Self::HookFailTurn => "hook_fail_turn".to_string(),
            Self::HookDegraded => "hook_degraded".to_string(),
            Self::HookIgnored => "hook_ignored".to_string(),
            Self::TurnFailed => "turn_failed".to_string(),
            Self::TurnCancelled => "turn_cancelled".to_string(),
            Self::TurnHopLimit => "turn_hop_limit".to_string(),
            Self::TurnFollowupLimit => "turn_followup_limit".to_string(),
            Self::TurnRepeatedFailure => "turn_repeated_failure".to_string(),
            Self::TurnRecoveryLocalFallback => "turn_recovery_local_fallback".to_string(),
            Self::Unknown(raw) => raw.clone(),
        }
    }

    /// 线码 → 注册表。未注册码 Fail-Closed 为 `Unknown("unknown:<原文>")`，
    /// 永不 panic/吞错（C7.3）。
    pub fn from_str(code: &str) -> Self {
        let trimmed = code.trim();
        if trimmed.starts_with("unknown:") && !trimmed["unknown:".len()..].is_empty() {
            return Self::Unknown(trimmed.to_string());
        }
        let mapped = match trimmed {
            "tool_unknown_descriptor" | "unknown_descriptor" => Some(Self::ToolUnknownDescriptor),
            "tool_origin_not_authorized" | "origin_not_authorized" => {
                Some(Self::ToolOriginNotAuthorized)
            }
            "tool_turn_view_snapshot_mismatch" | "turn_view_snapshot_mismatch" => {
                Some(Self::ToolTurnViewSnapshotMismatch)
            }
            "tool_invalid_arguments" | "invalid_arguments" => Some(Self::ToolInvalidArguments),
            "tool_pre_dispatch_hook_error" | "pre_dispatch_hook_error" => {
                Some(Self::ToolPreDispatchHookError)
            }
            "tool_sandbox_unavailable" | "sandbox_unavailable" => {
                Some(Self::ToolSandboxUnavailable)
            }
            "tool_sandbox_denied" | "sandbox_denied" => Some(Self::ToolSandboxDenied),
            "tool_no_handler_registered" | "no_handler_registered" => {
                Some(Self::ToolNoHandlerRegistered)
            }
            "tool_output_budget_exceeded" | "output_budget_exceeded" => {
                Some(Self::ToolOutputBudgetExceeded)
            }
            "tool_budget_exhausted" | "budget_exhausted" => Some(Self::ToolBudgetExhausted),
            "tool_timeout" | "timeout" => Some(Self::UpstreamTimeout),
            "tool_cancelled" | "cancelled" => Some(Self::ToolCancelled),
            "tool_interaction_unavailable" | "interaction_unavailable" => {
                Some(Self::ToolInteractionUnavailable)
            }
            "handler_error" => Some(Self::ToolHandlerError),
            "upstream_rate_limited" => Some(Self::UpstreamRateLimited),
            "upstream_server_error" => Some(Self::UpstreamServerError),
            "upstream_auth" => Some(Self::UpstreamAuth),
            "upstream_quota" => Some(Self::UpstreamQuota),
            "upstream_context_overflow" => Some(Self::UpstreamContextOverflow),
            "upstream_network" => Some(Self::UpstreamNetwork),
            "upstream_bad_request" => Some(Self::UpstreamBadRequest),
            "protocol_sse_parse" => Some(Self::ProtocolSseParse),
            "protocol_tool_call_schema" => Some(Self::ProtocolToolCallSchema),
            "protocol_model_catalog" => Some(Self::ProtocolModelCatalog),
            "protocol_unknown_model" => Some(Self::ProtocolUnknownModel),
            "protocol_reasoning_effort" => Some(Self::ProtocolReasoningEffort),
            "protocol_event_version" => Some(Self::ProtocolEventVersion),
            "capability_source_unavailable" | "source_unavailable" => {
                Some(Self::CapabilitySourceUnavailable)
            }
            "capability_permission_denied" | "permission_denied" => {
                Some(Self::CapabilityPermissionDenied)
            }
            "capability_out_of_scope" | "out_of_scope" => Some(Self::CapabilityOutOfScope),
            "capability_malformed_response" | "malformed_response" => {
                Some(Self::CapabilityMalformedResponse)
            }
            "capability_invocation_failed" | "invocation_failed" => {
                Some(Self::CapabilityInvocationFailed)
            }
            "capability_not_found" => Some(Self::CapabilityNotFound),
            "skill_resolution" => Some(Self::SkillResolution),
            "skill_source_unavailable" => Some(Self::SkillSourceUnavailable),
            "skill_permission_denied" => Some(Self::SkillPermissionDenied),
            "skill_malformed_composition" | "malformed_composition" => {
                Some(Self::SkillMalformedComposition)
            }
            "skill_unsupported_composition" | "unsupported_composition" => {
                Some(Self::SkillUnsupportedComposition)
            }
            "skill_underlying_capability_execution"
            | "underlying_capability_execution" => {
                Some(Self::SkillUnderlyingCapabilityExecution)
            }
            "hook_fail_turn" => Some(Self::HookFailTurn),
            "hook_degraded" => Some(Self::HookDegraded),
            "hook_ignored" => Some(Self::HookIgnored),
            "turn_failed" => Some(Self::TurnFailed),
            "turn_cancelled" => Some(Self::TurnCancelled),
            "turn_hop_limit" => Some(Self::TurnHopLimit),
            "turn_followup_limit" => Some(Self::TurnFollowupLimit),
            "turn_repeated_failure" => Some(Self::TurnRepeatedFailure),
            "turn_recovery_local_fallback" => Some(Self::TurnRecoveryLocalFallback),
            _ => None,
        };
        match mapped {
            Some(code) => code,
            None => Self::Unknown(format!("unknown:{trimmed}")),
        }
    }

    /// 分层归类。
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::ToolUnknownDescriptor
            | Self::ToolOriginNotAuthorized
            | Self::ToolTurnViewSnapshotMismatch
            | Self::ToolInvalidArguments
            | Self::ToolPreDispatchHookError
            | Self::ToolSandboxUnavailable
            | Self::ToolSandboxDenied
            | Self::ToolNoHandlerRegistered
            | Self::ToolOutputBudgetExceeded
            | Self::ToolBudgetExhausted
            | Self::ToolTimeout
            | Self::ToolCancelled
            | Self::ToolInteractionUnavailable
            | Self::ToolHandlerError => ErrorCategory::Tool,
            Self::UpstreamTimeout
            | Self::UpstreamRateLimited
            | Self::UpstreamServerError
            | Self::UpstreamAuth
            | Self::UpstreamQuota
            | Self::UpstreamContextOverflow
            | Self::UpstreamNetwork
            | Self::UpstreamBadRequest => ErrorCategory::Upstream,
            Self::ProtocolSseParse
            | Self::ProtocolToolCallSchema
            | Self::ProtocolModelCatalog
            | Self::ProtocolUnknownModel
            | Self::ProtocolReasoningEffort
            | Self::ProtocolEventVersion => ErrorCategory::Protocol,
            Self::CapabilitySourceUnavailable
            | Self::CapabilityPermissionDenied
            | Self::CapabilityOutOfScope
            | Self::CapabilityMalformedResponse
            | Self::CapabilityInvocationFailed
            | Self::CapabilityNotFound => ErrorCategory::Capability,
            Self::SkillResolution
            | Self::SkillSourceUnavailable
            | Self::SkillPermissionDenied
            | Self::SkillMalformedComposition
            | Self::SkillUnsupportedComposition
            | Self::SkillUnderlyingCapabilityExecution => ErrorCategory::Skill,
            Self::HookFailTurn | Self::HookDegraded | Self::HookIgnored => ErrorCategory::Hook,
            Self::TurnFailed
            | Self::TurnCancelled
            | Self::TurnHopLimit
            | Self::TurnFollowupLimit
            | Self::TurnRepeatedFailure
            | Self::TurnRecoveryLocalFallback => ErrorCategory::Turn,
            Self::Unknown(_) => ErrorCategory::Unknown,
        }
    }

    /// 重试提示（仅提示，不改变现有 `classify/decide` 行为）。
    /// `None` = 由调用方既有逻辑决定（未知码不擅断）。
    pub fn is_retryable_hint(&self) -> Option<bool> {
        match self {
            Self::UpstreamTimeout | Self::UpstreamRateLimited | Self::UpstreamServerError | Self::UpstreamNetwork => {
                Some(true)
            }
            Self::UpstreamAuth
            | Self::UpstreamQuota
            | Self::UpstreamContextOverflow
            | Self::UpstreamBadRequest
            | Self::ToolInvalidArguments
            | Self::ToolOriginNotAuthorized
            | Self::ToolSandboxDenied
            | Self::CapabilityPermissionDenied
            | Self::CapabilityOutOfScope
            | Self::ProtocolToolCallSchema
            | Self::ProtocolReasoningEffort => Some(false),
            _ => None,
        }
    }
}

impl From<CapabilityFailureKind> for ErrorCode {
    fn from(kind: CapabilityFailureKind) -> Self {
        match kind {
            CapabilityFailureKind::SourceUnavailable => Self::CapabilitySourceUnavailable,
            CapabilityFailureKind::PermissionDenied => Self::CapabilityPermissionDenied,
            CapabilityFailureKind::OutOfScope => Self::CapabilityOutOfScope,
            CapabilityFailureKind::MalformedResponse => Self::CapabilityMalformedResponse,
            CapabilityFailureKind::InvocationFailed => Self::CapabilityInvocationFailed,
            CapabilityFailureKind::CapabilityNotFound => Self::CapabilityNotFound,
        }
    }
}

impl From<SkillFailureLayer> for ErrorCode {
    fn from(layer: SkillFailureLayer) -> Self {
        match layer {
            SkillFailureLayer::SkillResolution => Self::SkillResolution,
            SkillFailureLayer::SourceUnavailable => Self::SkillSourceUnavailable,
            SkillFailureLayer::PermissionDenied => Self::SkillPermissionDenied,
            SkillFailureLayer::MalformedComposition => Self::SkillMalformedComposition,
            SkillFailureLayer::UnsupportedComposition => Self::SkillUnsupportedComposition,
            SkillFailureLayer::UnderlyingCapabilityExecution => {
                Self::SkillUnderlyingCapabilityExecution
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_timeout_wire_code_is_frozen_timeout() {
        assert_eq!(ErrorCode::UpstreamTimeout.as_str(), "timeout");
        assert_eq!(ErrorCode::from_str("timeout"), ErrorCode::UpstreamTimeout);
    }

    #[test]
    fn unknown_code_is_fail_closed_wrapped() {
        let code = ErrorCode::from_str("some_future_thing");
        assert_eq!(code, ErrorCode::Unknown("unknown:some_future_thing".to_string()));
        assert_eq!(code.category(), ErrorCategory::Unknown);
    }

    #[test]
    fn capability_and_skill_from_mappings_cover_all_variants() {
        assert_eq!(
            ErrorCode::from(CapabilityFailureKind::OutOfScope),
            ErrorCode::CapabilityOutOfScope
        );
        assert_eq!(
            ErrorCode::from(SkillFailureLayer::UnsupportedComposition),
            ErrorCode::SkillUnsupportedComposition
        );
    }
}

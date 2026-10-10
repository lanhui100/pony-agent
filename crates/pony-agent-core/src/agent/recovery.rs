//! 统一运行时错误与重试恢复管线架构 (Execution Recovery Pipeline)
//!
//! 融合 Claude-Code 的应用自愈哲学与 DSH 的严密状态机规范：
//! - **L1 传输与上游层 (Transport / Upstream)**：涵盖 Provider 网络抖动、限流、超限微调、流降级与备用模型倒换。
//! - **L2 工具执行层 (Tool Execution)**：涵盖工具超时、权限拒绝、非幂等防护，以及 Model-native 错误反哺。
//! - **L3 会话与步进层 (Turn / Step Progression)**：涵盖连续调用死循环熔断、未决工具补全及轮次级状态回滚。

use crate::agent::error_code::ErrorCode;
use crate::agent::retry::{BackoffConfig, RetryBudget, StreamState};
use serde::{Deserialize, Serialize};

/// 错误所处的运行时层级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryLayer {
    /// L1: 模型服务与协议传输层
    L1TransportUpstream,
    /// L2: 工具调度与沙箱执行层
    L2ToolExecution,
    /// L3: Agent 会话与轮次步进层
    L3TurnProgression,
}

/// 统一恢复动作决策（借鉴 Claude-Code 自愈分支与 DSH 状态守卫）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// 立即按退避延迟执行重试（L1 瞬态抖动）
    RetryWithDelay { delay_ms: u64 },
    /// 流式传输失败时，降级为非流式同步调用（Claude-Code: stream fallback）
    FallbackToSyncRequest { reason: String },
    /// 上下文超限时，动态微调 max_tokens 或裁剪窗口后重试（Claude-Code: context overflow adjustment）
    AdjustTokensAndRetry {
        suggested_max_tokens: u32,
        reason: String,
    },
    /// 模型过载 (如 529) 或连续失败时，倒换至备用模型（Claude-Code: model fallback）
    SwitchModelFallback {
        fallback_model: String,
        reason: String,
    },
    /// 工具执行失败，但属于安全且由模型可纠正的错误：反哺给大模型以引导自愈（Model-native 机制）
    FeedbackToModelAsToolResult {
        tool_call_id: String,
        error_message: String,
    },
    /// 步骤/轮次异常中断时，为未完成的工具调用补齐未知状态，保证会话历史语法配对（DSH: ToolCallRecovery）
    RepairOrphanedToolCalls {
        orphaned_call_ids: Vec<String>,
        reason: String,
    },
    /// 连续死循环或高危违规，熔断当前轮次并向用户报警
    CircuitBreakTurn {
        error_code: String,
        reason: String,
    },
    /// 不可重试或预算耗尽，直接终止
    Abort {
        reason: String,
        is_exhausted: bool,
    },
}

/// 恢复上下文输入
#[derive(Debug, Clone)]
pub struct RecoveryContext {
    pub layer: RecoveryLayer,
    pub raw_error: String,
    pub error_code: Option<ErrorCode>,
    pub stream_state: Option<StreamState>,
    pub current_attempt: u32,
    pub consecutive_failures: u32,
    pub primary_model: Option<String>,
    pub fallback_model: Option<String>,
    pub tool_name: Option<String>,
    pub tool_is_idempotent: bool,
}

/// 恢复管线核心配置
#[derive(Debug, Clone)]
pub struct RecoveryPipelineConfig {
    pub backoff: BackoffConfig,
    pub max_consecutive_tool_failures: u32,
    pub floor_output_tokens: u32,
}

impl Default for RecoveryPipelineConfig {
    fn default() -> Self {
        Self {
            backoff: BackoffConfig::default(),
            max_consecutive_tool_failures: 3,
            floor_output_tokens: 1024,
        }
    }
}

/// 统一恢复管线策略器
pub struct ExecutionRecoveryPolicy {
    config: RecoveryPipelineConfig,
}

impl ExecutionRecoveryPolicy {
    pub fn new(config: RecoveryPipelineConfig) -> Self {
        Self { config }
    }

    /// 核心决策接口
    pub fn decide(&self, ctx: &RecoveryContext, budget: &RetryBudget) -> RecoveryAction {
        match ctx.layer {
            RecoveryLayer::L1TransportUpstream => self.decide_l1(ctx, budget),
            RecoveryLayer::L2ToolExecution => self.decide_l2(ctx),
            RecoveryLayer::L3TurnProgression => self.decide_l3(ctx),
        }
    }

    fn decide_l1(&self, ctx: &RecoveryContext, budget: &RetryBudget) -> RecoveryAction {
        let err_lower = ctx.raw_error.to_lowercase();

        // 1. 检查是否为 Context 窗口与 max_tokens 越界（Claude-Code: VY2 机制）
        if err_lower.contains("exceed context limit") || err_lower.contains("maximum context length") {
            let adjusted = self.config.floor_output_tokens;
            return RecoveryAction::AdjustTokensAndRetry {
                suggested_max_tokens: adjusted,
                reason: "Context window limit reached; clamp max_tokens".to_string(),
            };
        }

        // 2. 检查是否为模型过载且配置了备用模型（Claude-Code: PX5 / m71 机制）
        if (err_lower.contains("overloaded") || err_lower.contains("529")) && ctx.fallback_model.is_some() {
            if ctx.consecutive_failures >= 2 {
                return RecoveryAction::SwitchModelFallback {
                    fallback_model: ctx.fallback_model.clone().unwrap(),
                    reason: "Repeated upstream overload (529); switching to fallback model".to_string(),
                };
            }
        }

        // 3. 检查流式中断降级为同步（StreamState 可降级）
        if let Some(stream_state) = ctx.stream_state {
            if stream_state.may_stream_to_sync_fallback() && (err_lower.contains("stream") || err_lower.contains("chunk")) {
                return RecoveryAction::FallbackToSyncRequest {
                    reason: format!("Stream interrupted in state {:?}; falling back to non-streaming sync", stream_state),
                };
            }
            if !stream_state.may_auto_retry() {
                return RecoveryAction::Abort {
                    reason: format!("Stream state {:?} forbids retry", stream_state),
                    is_exhausted: false,
                };
            }
        }

        // 4. 检查预算与重试限制
        if budget.is_exhausted() {
            return RecoveryAction::Abort {
                reason: format!("Retry budget exhausted (attempts: {}, elapsed: {}ms)", budget.attempts_used, budget.elapsed_ms),
                is_exhausted: true,
            };
        }

        // 5. 判断错误瞬态性
        let is_transient = err_lower.contains("timeout")
            || err_lower.contains("connection")
            || err_lower.contains("429")
            || err_lower.contains("rate limit")
            || err_lower.contains("500")
            || err_lower.contains("502")
            || err_lower.contains("503")
            || err_lower.contains("504")
            || err_lower.contains("529");

        if is_transient {
            let delay = crate::agent::retry::compute_delay(ctx.current_attempt + 1, &self.config.backoff, 0.5);
            RecoveryAction::RetryWithDelay {
                delay_ms: delay.as_millis() as u64,
            }
        } else {
            RecoveryAction::Abort {
                reason: format!("Non-retryable upstream error: {}", ctx.raw_error),
                is_exhausted: false,
            }
        }
    }

    fn decide_l2(&self, ctx: &RecoveryContext) -> RecoveryAction {
        // 非幂等工具发生不确定性错误，拒绝自动重试（DSH: TOOL_OUTCOME_UNKNOWN 防护）
        if !ctx.tool_is_idempotent && ctx.raw_error.to_lowercase().contains("timeout") {
            return RecoveryAction::Abort {
                reason: format!(
                    "Tool '{}' is non-idempotent and encountered timeout; refusing blind retry",
                    ctx.tool_name.as_deref().unwrap_or("unknown")
                ),
                is_exhausted: false,
            };
        }

        // 常规业务/参数报错，反哺给模型自愈（Claude-Code: is_error=true 反哺机制）
        RecoveryAction::FeedbackToModelAsToolResult {
            tool_call_id: ctx.tool_name.clone().unwrap_or_else(|| "call_unknown".to_string()),
            error_message: ctx.raw_error.clone(),
        }
    }

    fn decide_l3(&self, ctx: &RecoveryContext) -> RecoveryAction {
        // 连续相同工具失败熔断
        if ctx.consecutive_failures >= self.config.max_consecutive_tool_failures {
            return RecoveryAction::CircuitBreakTurn {
                error_code: "consecutive_tool_failures".to_string(),
                reason: format!(
                    "Tool '{}' failed {} times consecutively with same error; breaking loop to save tokens",
                    ctx.tool_name.as_deref().unwrap_or("unknown"),
                    ctx.consecutive_failures
                ),
            };
        }

        RecoveryAction::Abort {
            reason: format!("Turn-level unhandled exception: {}", ctx.raw_error),
            is_exhausted: false,
        }
    }
}

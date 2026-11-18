//! B214：provider 重试事件（`llm/retry`）的投影。
//!
//! 与其它投影同构（纯函数折叠、按 seq 升序、`>=` 水位防重放）：投影只消费已落盘并
//! 重载的 [`TurnEvent::LlmRetry`]，因此会话重载后仍能重建「正在重试模型请求
//! （`retry`/`max_attempts`）· `delay_ms`」所需的全部字段，不再依赖进程内的实时通道。
use serde::{Deserialize, Serialize};

use crate::agent::projection::Projection;
use crate::agent::turn_event::TurnEvent;

/// 一次真实重试的事实（`llm/retry` 的投影条目）。
///
/// 字段与事件一一对应；`retry` 为已失败尝试次数（1 起），`max_attempts` 为**尝试总数
/// （含首次）**——客户端直接以 `retry/max_attempts` 显示，不需要自行 +1。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRetryFact {
    /// 事件 seq（同一 turn 内按此升序）。
    pub seq: u64,
    pub turn_id: String,
    /// provider 调用名（`decision` / `decision_stream` / `followup` …）。
    pub request: String,
    /// 重试作用域（`decision` / `followup`）。
    pub scope: String,
    /// 失败分类（`timeout` / `rate_limited` / `quota` / `network` / `auth` /
    /// `bad_request` / `other`）。
    pub failure_code: String,
    /// 已失败尝试次数（1 起）。
    pub retry: u32,
    /// 本跳允许的尝试总数（含首次）。
    pub max_attempts: u32,
    /// 本次退避 sleep 的毫秒数。
    pub delay_ms: u64,
}

/// 重试投影状态：按事件顺序累积全部重试事实。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderRetryProjectionState {
    /// 按 seq 升序的重试事实（同一 turn 可多条）。
    pub retries: Vec<ProviderRetryFact>,
    /// 已折叠到的最大 seq（`>=` 语义防同 seq 重放；`None` = 尚未折叠）。
    watermark: Option<u64>,
}

impl ProviderRetryProjectionState {
    /// 最近一次重试（状态行「正在重试…」的最新事实）。
    pub fn latest(&self) -> Option<&ProviderRetryFact> {
        self.retries.last()
    }

    /// 指定 turn 的全部重试事实（重载后按 turn 复盘）。
    pub fn retries_for_turn(&self, turn_id: &str) -> Vec<&ProviderRetryFact> {
        self.retries
            .iter()
            .filter(|fact| fact.turn_id == turn_id)
            .collect()
    }
}

impl Projection<ProviderRetryProjectionState> for ProviderRetryProjectionState {
    fn init() -> Self {
        Self::default()
    }

    fn apply(state: &mut Self, seq: u64, event: &TurnEvent) {
        if state.watermark.is_some_and(|watermark| watermark >= seq) {
            return;
        }
        state.watermark = Some(seq);
        let TurnEvent::LlmRetry {
            turn_id,
            request,
            scope,
            failure_code,
            retry,
            max_attempts,
            delay_ms,
        } = event
        else {
            return;
        };
        state.retries.push(ProviderRetryFact {
            seq,
            turn_id: turn_id.clone(),
            request: request.clone(),
            scope: scope.clone(),
            failure_code: failure_code.clone(),
            retry: *retry,
            max_attempts: *max_attempts,
            delay_ms: *delay_ms,
        });
    }
}

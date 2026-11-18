//! B201：会话最近一轮的 provider/model 事实读取（自动标题选择 provider 用）。
//!
//! 标题是**会话级**能力：最近 turn trace 已记录实际使用的 provider/model，取出作为
//! `resolve_provider_selection` 的提示即可，不新增持久化字段、不做 schema 迁移。
//! 独立成文件是为了不撑大已触 file-size 棘轮的 `store.rs`。

use super::{SessionStore, TurnTraceRecord};

impl SessionStore {
    /// 读取会话最近一轮记录的 provider 和 model（若有）。
    ///
    /// 优先从最近 turn trace 中提取 `provider_name` / `provider_requested_name` 与
    /// `provider_model`；live state 缺席时回溯最近 history node 的 trace。
    pub fn session_provider_and_model(
        &self,
        session_id: &str,
    ) -> (Option<String>, Option<String>) {
        let Some(session) = self.get_session(session_id) else {
            return (None, None);
        };
        for trace in session.turn_trace_history.iter().rev() {
            if let Some(facts) = trace_provider_and_model(trace) {
                return facts;
            }
        }
        for node in session.history_nodes.iter().rev() {
            for trace in node.turn_trace_history.iter().rev() {
                if let Some(facts) = trace_provider_and_model(trace) {
                    return facts;
                }
            }
        }
        (None, None)
    }
}

/// 单条 trace 的 provider/model（二者至少一个非空才返回）。
fn trace_provider_and_model(
    trace: &TurnTraceRecord,
) -> Option<(Option<String>, Option<String>)> {
    let provider = trace
        .provider_name
        .as_deref()
        .or(trace.provider_requested_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let model = trace
        .provider_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if provider.is_some() || model.is_some() {
        Some((provider, model))
    } else {
        None
    }
}

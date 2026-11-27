//! B-CTX-LOAD (Option A)：C5 宽限监督收口的**持久化先于解锁**通道。
//!
//! 解决「监督线程已放 `turn:cancelled` 解锁 UI，而后台工作者尚未写盘」引发的
//! 跨轮次读写竞态：监督者在发射 `turn:cancelled` 前，先将已落盘事件 / checkpoint
//! 中本轮原话、部分回复、已闭合工具活动及取消 trace 原子提交到 `SessionStore`
//!（按 `turn_id` 幂等），之后再派发 `turn:cancelled` 並刷新事件水位。

use crate::agent::execution_control::ExecutionControlRegistry;
use crate::agent::runtime::AgentRuntime;
use crate::agent::session::{HistoryMessageKind, SessionStore, CANCELLED_TURN_SENTINEL};
use crate::agent::stop_escalation::{StopReason, TerminalEmitter};
use crate::agent::turn_flow::{emit_supervisor_cancelled, ProviderEventMeta, TurnEventSink};
use pony_protocol::turn_event::TurnEvent;
use std::sync::{Arc, RwLock};

pub(super) struct SupervisorCloseoutEmitter<S> {
    pub(super) sink: Arc<S>,
    pub(super) sessions: Arc<RwLock<SessionStore>>,
    pub(super) execution_control: ExecutionControlRegistry,
}

impl<S> TerminalEmitter for SupervisorCloseoutEmitter<S>
where
    S: TurnEventSink + Send + Sync + 'static,
{
    fn emit_cancelled(
        &self,
        turn_id: &str,
        session_id: Option<&str>,
        stop_reason: StopReason,
    ) {
        let session_key = session_id.filter(|s| !s.trim().is_empty());
        let flush_key = session_key.unwrap_or(crate::agent::session::DEFAULT_SESSION_ID);

        // 1) 在获取 sessions 写锁前 flush 缓冲事件，确保 user/tool 事件已在事件表可见。
        let _ = crate::agent::turn_flow::flush_session_buffered_events(flush_key);

        let checkpoint = self.execution_control.load_checkpoint(Some(turn_id), session_key);
        let checkpoint_user = checkpoint
            .as_ref()
            .and_then(|c| c.user_message.clone())
            .filter(|text| !text.trim().is_empty());
        let event_user = if checkpoint_user.is_none() {
            self.sessions.read().ok().and_then(|store| {
                store
                    .load_turn_events(flush_key, None)
                    .into_iter()
                    .rev()
                    .find_map(|(_, _, event)| match event {
                        TurnEvent::UserMessage {
                            turn_id: tid, text, ..
                        } if tid == turn_id && !text.trim().is_empty() => Some(text),
                        _ => None,
                    })
            })
        } else {
            None
        };
        let resolved_user = checkpoint_user.or(event_user);
        let has_progress = checkpoint.as_ref().is_some_and(|c| {
            !c.tool_activities.is_empty()
                || c.partial_assistant_text
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty())
        });

        // 只有真实入口启动的轮次（带 user 原话或检查点进度）才写入持久化层；
        // 纯 registry 单测（仅 register_turn + stop_turn）不制造空占位会话。
        if resolved_user.is_some() || has_progress {
            let user_message = resolved_user.unwrap_or_default();
            let partial_text = checkpoint
                .as_ref()
                .and_then(|c| c.partial_assistant_text.as_deref())
                .unwrap_or("");
            let tool_activities = checkpoint
                .as_ref()
                .map(|c| c.tool_activities.clone())
                .unwrap_or_default();
            let trace_steps = checkpoint
                .as_ref()
                .map(|c| c.trace_steps.clone())
                .unwrap_or_default();
            let provider_meta = checkpoint.as_ref().and_then(|c| {
                let provider_name = c.provider_name.clone()?;
                Some(ProviderEventMeta {
                    requested_name: c
                        .provider_requested_name
                        .clone()
                        .unwrap_or_else(|| provider_name.clone()),
                    provider_name,
                    protocol: c.provider_protocol.clone().unwrap_or_default(),
                    model: c.provider_model.clone().unwrap_or_default(),
                    context_window_tokens: None,
                })
            });
            let header_step_lines: Vec<String> = tool_activities
                .iter()
                .map(|activity| {
                    if activity.description.trim().is_empty() {
                        activity.name.clone()
                    } else {
                        format!("{} {}", activity.name, activity.description.trim())
                    }
                })
                .collect();
            let (assistant_text, transcript) = AgentRuntime::build_interrupted_turn_close_out(
                partial_text,
                &header_step_lines,
                &tool_activities,
                &[],
                None,
                None,
            );
            let assistant_message = if assistant_text.trim().is_empty() {
                CANCELLED_TURN_SENTINEL.to_string()
            } else {
                assistant_text
            };
            let error = format!("stopped_by_{}", stop_reason.as_str());
            let trace_record = AgentRuntime::build_turn_trace_record(
                session_key,
                turn_id,
                &user_message,
                "cancelled",
                trace_steps,
                tool_activities,
                &[],
                Vec::new(),
                Vec::new(),
                provider_meta.as_ref(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(error),
            );
            let mut store = self.sessions.write().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            if let Err(err) = store.append_turn_with_kind_and_trace_fallible(
                session_key,
                &user_message,
                &assistant_message,
                transcript,
                Vec::new(),
                Some(HistoryMessageKind::Cancelled),
                Some(turn_id),
                Some(trace_record),
            ) {
                tracing::warn!(
                    target: "control_plane",
                    turn_id,
                    session_id = flush_key,
                    error = %err,
                    "[pony-agent] supervisor closeout failed to persist cancelled turn"
                );
            }
        }

        // 2) 持久化完成之后再发射 turn:cancelled 解锁宿主/UI。
        emit_supervisor_cancelled(
            self.sink.as_ref(),
            turn_id.to_string(),
            session_id.map(str::to_string),
            stop_reason,
        );

        // 3) 立即 flush 并推进事件水位与引用化，确保 Turn N+1 读取即见到完整收敛节点。
        let _ = crate::agent::turn_flow::flush_session_buffered_events(flush_key);
        if let Ok(mut store) = self.sessions.write() {
            store.finalize_event_watermark(flush_key, turn_id);
        }
    }
}

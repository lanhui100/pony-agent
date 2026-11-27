//! B130：goal 领域事件 → Tauri 宿主事件的唯一映射层。
//!
//! 真值源与发射时机：`docs/architecture/plan-goal-contract.md` §7（`goal://changed` /
//! `goal://round`）。领域事实由 `pony_agent_core::agent::goal` 在**每次写盘成功后**广播
//! （`GoalEvent`），本文件只负责把事实映射成线缆载荷；core 不依赖 Tauri。
//!
//! 为什么不在命令层逐个 `emit`：goal 变更同时来自三条路径——Tauri IPC 命令
//! （`goal_create` / `goal_update` / `goal_command`）、模型工具（`update_goal`）与宿主续轮
//! （`advance_goal_round`）。只在命令层发射会让「模型自己推进的目标」对 UI 不可见；观察者
//! 挂在领域写锁释放之后，三条路径天然同源。
//!
//! `goal://cleared` 是 B130 的 **additive** 事件：`clear` 没有 GoalData 快照，复用
//! `goal://changed` 会让 `phase` 字段说谎；其载荷只有会话与（被删目标的）id，供 GoalBar
//! 立即收起面板。§7 冻结的两个事件名与载荷**零改动**。

use pony_agent_core::agent::goal::{
    install_goal_event_observer, GoalActivation, GoalData, GoalEvent,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

/// §7 冻结字面量：前端 `GOAL_CHANGED_EVENT` 与本常量必须同源。
pub(crate) const GOAL_CHANGED_EVENT: &str = "goal://changed";
/// §7 冻结字面量：每成功发起一轮 goal-round。
pub(crate) const GOAL_ROUND_EVENT: &str = "goal://round";
/// B130 additive：`clear_goal` 真删了记录（无墓碑，附带被删 id）。
pub(crate) const GOAL_CLEARED_EVENT: &str = "goal://cleared";
/// O6（2026-11-27）：一次自动续轮**停止/暂停**（原因可见）。
///
/// 为什么必须新增：旧实现只有 TUI 的 `UiEvent::GoalRoundsStopped`，桌面端零事件面 ⇒
/// 「目标为什么不动了」在所有桌面停止路径上都不可见（审核报告 P1-3）。
pub(crate) const GOAL_ROUNDS_STOPPED_EVENT: &str = "goal://rounds_stopped";

/// `goal://changed` 载荷（camelCase，字段集合按 §7 冻结）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoalChangedPayload {
    pub goal_id: String,
    pub session_id: String,
    pub phase: String,
    pub activation: GoalActivation,
    pub revision: u64,
    pub rounds_started: u32,
    pub max_goal_rounds: u32,
}

impl From<&GoalData> for GoalChangedPayload {
    fn from(goal: &GoalData) -> Self {
        Self {
            goal_id: goal.id.clone(),
            session_id: goal.session_id.clone(),
            phase: goal.phase.clone(),
            activation: goal.activation.clone(),
            revision: goal.revision,
            rounds_started: goal.rounds_started,
            max_goal_rounds: goal.max_goal_rounds,
        }
    }
}

/// `goal://round` 载荷。`status` 在 wave-1 只有 `"started"`（§7：其他取值需重锚契约）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoalRoundPayload {
    pub goal_id: String,
    pub round: u32,
    pub max_rounds: u32,
    pub status: &'static str,
}

/// `goal://rounds_stopped` 载荷（O6，additive）。
///
/// 字段与 TUI `UiEvent::GoalRoundsStopped` 同源同义：`reason` 是
/// `GoalRoundStopReason::code()`（稳定机器识别码），`visibleLine` 是 `visible_line()`
/// （用户可见文案的结构化真源）——前端不得自行拼措辞。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoalRoundsStoppedPayload {
    pub session_id: String,
    pub goal_id: String,
    pub reason: String,
    pub visible_line: String,
    pub rounds_started: u32,
    pub max_rounds: u32,
    pub consecutive_failures: u32,
}

/// `goal://cleared` 载荷（additive）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoalClearedPayload {
    pub session_id: String,
    pub goal_id: Option<String>,
}

/// 安装进程级观察者（`run()` 的 setup 内调用一次）。
pub(crate) fn install(app: AppHandle) {
    install_goal_event_observer(Arc::new(move |event: &GoalEvent| match event {
        GoalEvent::Changed { goal } => {
            emit(&app, GOAL_CHANGED_EVENT, GoalChangedPayload::from(goal));
        }
        GoalEvent::RoundStarted { goal } => {
            emit(
                &app,
                GOAL_ROUND_EVENT,
                GoalRoundPayload {
                    goal_id: goal.id.clone(),
                    round: goal.rounds_started,
                    max_rounds: goal.max_goal_rounds,
                    status: "started",
                },
            );
        }
        GoalEvent::Cleared {
            session_id,
            goal_id,
        } => {
            emit(
                &app,
                GOAL_CLEARED_EVENT,
                GoalClearedPayload {
                    session_id: session_id.clone(),
                    goal_id: goal_id.clone(),
                },
            );
        }
        GoalEvent::RoundsStopped {
            session_id,
            goal_id,
            reason_code,
            visible_line,
            rounds_started,
            max_rounds,
            consecutive_failures,
        } => {
            emit(
                &app,
                GOAL_ROUNDS_STOPPED_EVENT,
                GoalRoundsStoppedPayload {
                    session_id: session_id.clone(),
                    goal_id: goal_id.clone(),
                    reason: reason_code.clone(),
                    visible_line: visible_line.clone(),
                    rounds_started: *rounds_started,
                    max_rounds: *max_rounds,
                    consecutive_failures: *consecutive_failures,
                },
            );
        }
    }));
}

/// 发射失败只记结构化诊断，绝不回滚已落盘的目标（§7）。
fn emit<T: Serialize + Clone>(app: &AppHandle, name: &str, payload: T) {
    if let Err(error) = app.emit(name, payload) {
        pony_agent_core::agent::otel::log::log_json_with_fields(
            "warn",
            "goal_event_emit_failed",
            None,
            &[
                ("event", serde_json::json!(name)),
                ("error", serde_json::json!(error.to_string())),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pony_agent_core::agent::goal::GoalActivation;
    use serde_json::Value;

    fn sample_goal() -> GoalData {
        GoalData {
            id: "goal-1".to_string(),
            session_id: "session-1".to_string(),
            objective: "把报告写完".to_string(),
            phase: "active".to_string(),
            activation: GoalActivation::Armed,
            revision: 2,
            rounds_started: 3,
            max_goal_rounds: 10,
            blocked_reason: None,
        }
    }

    /// 事件名是前端 `safeListen` 的唯一锚点：改名 = 前端静默收不到刷新。
    #[test]
    fn goal_event_names_match_frontend_contract() {
        assert_eq!(GOAL_CHANGED_EVENT, "goal://changed");
        assert_eq!(GOAL_ROUND_EVENT, "goal://round");
        assert_eq!(GOAL_CLEARED_EVENT, "goal://cleared");
        assert_eq!(GOAL_ROUNDS_STOPPED_EVENT, "goal://rounds_stopped");
        assert!(GOAL_CHANGED_EVENT.starts_with("goal://"));
        assert!(GOAL_ROUND_EVENT.starts_with("goal://"));
        assert!(GOAL_ROUNDS_STOPPED_EVENT.starts_with("goal://"));
    }

    /// §7 载荷形状锁定：camelCase 键 + 与 GoalData 同步的 round 计数。
    #[test]
    fn changed_payload_serializes_to_contract_shape() {
        let wire = serde_json::to_value(GoalChangedPayload::from(&sample_goal()))
            .expect("payload must serialize");
        let object = wire.as_object().expect("payload must be a JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "activation",
                "goalId",
                "maxGoalRounds",
                "phase",
                "revision",
                "roundsStarted",
                "sessionId"
            ]
        );
        assert_eq!(object["activation"], Value::String("armed".to_string()));
        assert_eq!(object["roundsStarted"], Value::from(3));
    }

    #[test]
    fn round_payload_matches_contract_shape() {
        let goal = sample_goal();
        let wire = serde_json::to_value(GoalRoundPayload {
            goal_id: goal.id.clone(),
            round: goal.rounds_started,
            max_rounds: goal.max_goal_rounds,
            status: "started",
        })
        .expect("payload must serialize");
        let object = wire.as_object().expect("payload must be a JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["goalId", "maxRounds", "round", "status"]);
        assert_eq!(object["round"], Value::from(3));
        assert_eq!(object["status"], Value::String("started".to_string()));
    }

    #[test]
    fn cleared_payload_carries_session_and_removed_goal_id() {
        let wire = serde_json::to_value(GoalClearedPayload {
            session_id: "session-1".to_string(),
            goal_id: Some("goal-1".to_string()),
        })
        .expect("payload must serialize");
        assert_eq!(wire["sessionId"], Value::String("session-1".to_string()));
        assert_eq!(wire["goalId"], Value::String("goal-1".to_string()));
        // 无目标时 clear 是幂等 no-op（core 不广播），但如果将来广播，null 是合法形状。
        let empty = serde_json::to_value(GoalClearedPayload {
            session_id: "session-1".to_string(),
            goal_id: None,
        })
        .expect("payload must serialize");
        assert!(empty["goalId"].is_null());
    }
}

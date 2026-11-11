//! wave-60「可中断停止」：结构化停止原因词汇表 + 宽限期收口（stop escalation）**类型面冻结**。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md`
//! （§1 唯一词汇表、C5 宽限期强制收口、§4 NFR `stop_grace_ms=2000` / `stop_poll_tick_ms=250`）。
//!
//! **表面中立是硬约束**：本模块属于 core 协议面，只允许出现 wire 值与会话/轮次标识，
//! 严禁引用 tauri（或任何宿主/UI 表面类型）；宿主适配层负责把 [`TerminalEmitter`] 接到自己的事件总线上。
//!
//! 冻结范围（wave60-T0）：类型、签名与缺省参数。函数体一律为 `todo!("wave60")`，
//! 行为实现归属 wave60-T1（core-contract）/ wave60-T3（tauri-wiring），不得在本文件内展开。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent::execution_control::ExecutionControlRegistry;

/// §1 结构化停止原因（唯一词汇表）。
///
/// 契约纪律：**只能追加**，禁止重命名/删除既有值；wire 值即 `serde` 的 snake_case 值。
/// 缺省（无原因记录）由调用方落到 [`StopReason::User`]；历史迁移无原因记录用 [`StopReason::Legacy`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// UI 显式停止（缺省值）。
    User,
    /// 子代理/上游级联停父轮。
    Parent,
    /// 钩子触发（附带的 reason 文本不单独建枚举值）。
    Hook,
    /// 前端看门狗兜底。
    Watchdog,
    /// 历史迁移无原因记录。
    Legacy,
}

impl StopReason {
    /// wire 值（与 TS `StopReason` 联合类型逐字对齐）。
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::User => "user",
            StopReason::Parent => "parent",
            StopReason::Hook => "hook",
            StopReason::Watchdog => "watchdog",
            StopReason::Legacy => "legacy",
        }
    }
}

impl FromStr for StopReason {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_lowercase().as_str() {
            "user" => Ok(StopReason::User),
            "parent" => Ok(StopReason::Parent),
            "hook" => Ok(StopReason::Hook),
            "watchdog" => Ok(StopReason::Watchdog),
            "legacy" => Ok(StopReason::Legacy),
            _ => Err(()),
        }
    }
}

/// C5 宽限期收口参数（NFR：缺省 2000/250，测试必须可注入毫秒级小值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopEscalationConfig {
    /// 宽限期上界：stop 被接受后若 checkpoint 仍为 `running`，到点即由监督者收口。
    pub stop_grace_ms: u64,
    /// 监督者的观测节奏（与读通道 `stop_poll_tick_ms` 同值）。
    pub stop_poll_tick_ms: u64,
}

impl Default for StopEscalationConfig {
    fn default() -> Self {
        Self {
            stop_grace_ms: 2000,
            stop_poll_tick_ms: 250,
        }
    }
}

impl StopEscalationConfig {
    /// 测试注入构造器（禁用 sleep 2s 的真实宽限期）。
    pub fn for_test(grace_ms: u64, tick_ms: u64) -> Self {
        Self {
            stop_grace_ms: grace_ms,
            stop_poll_tick_ms: tick_ms,
        }
    }
}

/// C5 终端发射器：core 侧的表面中立出口，由宿主注入（每 turn 至多注册一个）。
///
/// 实现方负责把一条 `turn:cancelled` 送到自己的表面；core 侧保证监督者自身**不重复**派发。
pub trait TerminalEmitter: Send + Sync {
    fn emit_cancelled(&self, turn_id: &str, session_id: Option<&str>, stop_reason: StopReason);
}

/// C5 宽限期监督者（类型面冻结；行为归属 wave60-T1）。
///
/// 语义（实现时遵守）：stop 被接受后启动监督；宽限期内 checkpoint 仍 `running` ⇒ 幂等地
/// ① 置 checkpoint `cancelled`（含 `stop_reason`）② 经已注册发射器派发**一条** `turn:cancelled`；
/// 宽限期内循环已终止 ⇒ **不派发**；未注册发射器 ⇒ 只落盘 checkpoint，**不得 panic**。
#[allow(dead_code)] // wave60-T0：字段面冻结，读取方在 wave60-T1/T3 实现时接入。
pub struct StopEscalation {
    config: StopEscalationConfig,
    /// C5 判定所需的 checkpoint 读写面（宽限期内判定 `running` ⇒ 置 `cancelled` 并落 `stop_reason`）。
    registry: Option<ExecutionControlRegistry>,
    emitters: Mutex<HashMap<String, Arc<dyn TerminalEmitter>>>,
}

impl StopEscalation {
    pub fn new(config: StopEscalationConfig) -> Self {
        Self {
            config,
            registry: None,
            emitters: Mutex::new(HashMap::new()),
        }
    }

    /// C5 的 checkpoint 读写注入面（测试与宿主接线用；`new` 构造的实例无 registry）。
    pub fn with_registry(config: StopEscalationConfig, registry: ExecutionControlRegistry) -> Self {
        Self {
            config,
            registry: Some(registry),
            emitters: Mutex::new(HashMap::new()),
        }
    }

    /// 按 `turn_id` 注册终端发射器（幂等键的作用域是单轮，见 C5）。
    ///
    /// 重注册会覆盖前一个发射器（"每 turn 至多一个"）；发射器在 `on_stop_accepted`
    /// 时被**取走**并由监督线程独占持有，因此表项不随 turn 数量无限增长。
    pub fn register_emitter(&self, turn_id: &str, emitter: Arc<dyn TerminalEmitter>) {
        let mut emitters = self.emitters.lock().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] stop escalation emitter lock poisoned");
            e.into_inner()
        });
        emitters.insert(turn_id.to_string(), emitter);
    }

    /// stop 被接受（C1/C2 路径的唯一入口）后启动宽限期监督。
    ///
    /// 线程模型（C5 + 线程安全红线）：
    /// - **每 turn 一个**监督线程，宽限到点（或观察到循环已终止）即退出，不常驻；
    /// - 监督线程**只在锁内读写 checkpoint**（`ExecutionControlRegistry` 自带短锁），
    ///   拿到判定结果并释放锁**之后**才调用 emitter —— 绝不在持锁时回调宿主表面，
    ///   避免与 `stop_turn`/`register_turn` 争锁造成死锁或饥饿；
    /// - 每 turn 至多派发一次终态（本线程派发后立即退出，天然幂等）。
    pub fn on_stop_accepted(
        &self,
        turn_id: &str,
        session_id: Option<&str>,
        stop_reason: StopReason,
    ) {
        let Some(registry) = self.registry.clone() else {
            // 未注入 checkpoint 读写面（纯类型面实例）⇒ 无可监督，安全返回。
            return;
        };
        let emitter = self.take_emitter(turn_id);
        let config = self.config;
        let turn_id = turn_id.to_string();
        let session_id = session_id.map(str::to_string);

        // 线程内独占持有；失败路径需要保留一份句柄做**同步降级**（故先克隆）。
        let registry_for_thread = registry.clone();
        let emitter_for_thread = emitter.clone();
        let turn_for_thread = turn_id.clone();
        let session_for_thread = session_id.clone();

        // 监督线程的句柄不外传：宽限到点/循环终止即自行退出（不泄漏常驻线程）。
        let thread_name = format!("stop-escalation-{turn_id}");
        if let Err(error) = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                supervise_until_grace(
                    &registry_for_thread,
                    emitter_for_thread.as_deref(),
                    &turn_for_thread,
                    session_for_thread.as_deref(),
                    stop_reason,
                    config,
                );
            })
        {
            // 起不了监督线程 ⇒ **真正的同步降级**（等价 grace=0）：立即做一次与宽限到点
            // 完全相同的收口（同一 [`escalate_now`]，无行为分叉），保证 C5 终态不丢。
            crate::agent::otel::log::log_json_with_fields(
                "error",
                "stop escalation supervisor thread spawn failed; escalating synchronously",
                None,
                &[
                    ("turn_id", serde_json::json!(&turn_id)),
                    ("error", serde_json::json!(error.to_string())),
                ],
            );
            escalate_now(
                &registry,
                emitter.as_deref(),
                &turn_id,
                session_id.as_deref(),
                stop_reason,
            );
        }
    }

    /// 注销某 turn 的终端发射器（幂等）：与 `register_emitter` 配对，供宿主在 turn 结束
    /// 时回收（否则 emitter 表按 turn 无界增长）。
    pub fn unregister_emitter(&self, turn_id: &str) {
        let mut emitters = self.emitters.lock().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] stop escalation emitter lock poisoned");
            e.into_inner()
        });
        emitters.remove(turn_id);
    }

    /// 取走（并从表中移除）该 turn 的发射器：监督线程独占持有，表项随取随清。
    fn take_emitter(&self, turn_id: &str) -> Option<Arc<dyn TerminalEmitter>> {
        let mut emitters = self.emitters.lock().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] stop escalation emitter lock poisoned");
            e.into_inner()
        });
        emitters.remove(turn_id)
    }
}

/// C5 监督主体：按 `stop_poll_tick_ms` 观测 checkpoint，宽限到点仍 `running` ⇒ 幂等收口。
///
/// 返回时线程退出（不常驻）；**先落盘、后派发**，且派发前已释放 registry 锁。
fn supervise_until_grace(
    registry: &ExecutionControlRegistry,
    emitter: Option<&dyn TerminalEmitter>,
    turn_id: &str,
    session_id: Option<&str>,
    stop_reason: StopReason,
    config: StopEscalationConfig,
) {
    let tick = Duration::from_millis(config.stop_poll_tick_ms.max(1));
    let deadline = Instant::now() + Duration::from_millis(config.stop_grace_ms);

    loop {
        // ① 观测：checkpoint 已非 running（循环自行终止）⇒ 零派发，直接退出。
        let status = registry
            .load_checkpoint(Some(turn_id), None)
            .map(|checkpoint| checkpoint.status);
        match status {
            Some(status) if status == "running" => {}
            // 未知 turn / 已终态 ⇒ 宽限期内循环已终止：监督者**不得**派发、不得改写。
            _ => return,
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep(tick.min(deadline.saturating_duration_since(now)));
    }

    // ② 宽限到点 ⇒ 收口（与同步降级共用同一实现）。
    escalate_now(registry, emitter, turn_id, session_id, stop_reason);
}

/// C5 收口本体（**唯一实现**：宽限到点路径与"起不了监督线程"的同步降级路径共用）。
///
/// 线程安全红线：**在锁内**只做"仍是 `running` 才置 `cancelled`"的原子判定（避免与循环
/// 自然终止竞态而改写已落地终态），锁释放**之后**才调用 emitter（绝不在持锁时回调宿主表面）。
/// 返回 `true` 表示本次真的收口并（若有 emitter）派发了一条终态；`false` 表示循环已自行终止
/// ⇒ 零派发（契约 C5）。
fn escalate_now(
    registry: &ExecutionControlRegistry,
    emitter: Option<&dyn TerminalEmitter>,
    turn_id: &str,
    session_id: Option<&str>,
    stop_reason: StopReason,
) -> bool {
    let mut escalated = false;
    registry.update(turn_id, |checkpoint| {
        if checkpoint.status != "running" {
            return;
        }
        checkpoint.status = "cancelled".to_string();
        checkpoint.phase = "cancelled".to_string();
        checkpoint.stop_reason = Some(stop_reason.as_str().to_string());
        escalated = true;
    });
    if !escalated {
        // 宽限期内循环已自行终止 ⇒ 监督者零派发（契约 C5）。
        return false;
    }
    // 锁已释放（`update` 内部短锁）——此处才允许回调宿主表面。
    let recorded_reason = registry
        .load_checkpoint(Some(turn_id), None)
        .and_then(|checkpoint| checkpoint.stop_reason)
        .and_then(|wire| wire.parse::<StopReason>().ok())
        .unwrap_or(stop_reason);

    if let Some(emitter) = emitter {
        emitter.emit_cancelled(turn_id, session_id, recorded_reason);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingEmitter {
        calls: AtomicUsize,
    }

    impl TerminalEmitter for CountingEmitter {
        fn emit_cancelled(&self, _turn_id: &str, _session_id: Option<&str>, _reason: StopReason) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// wave-60 收口修复（LEAK）：`unregister_emitter` 必须把已注册发射器从表中回收，
    /// 否则宿主注册 sink + emitter 后按 turn 只增不减（无界增长）。幂等（重复注销不 panic）。
    #[test]
    fn unregister_emitter_releases_emitter_idempotently() {
        let escalation = StopEscalation::new(StopEscalationConfig::for_test(50, 5));
        let emitter = Arc::new(CountingEmitter::default());

        // Arrange：注册 ⇒ 可被取走（正常 C5 路径）。
        escalation.register_emitter("turn-leak", emitter.clone());
        assert!(
            escalation.take_emitter("turn-leak").is_some(),
            "注册后的发射器必须能被收口路径取走"
        );

        // Act：重新注册后注销 ⇒ 表项必须已回收。
        escalation.register_emitter("turn-leak", emitter);
        escalation.unregister_emitter("turn-leak");

        // Assert
        assert!(
            escalation.take_emitter("turn-leak").is_none(),
            "unregister_emitter 后不得残留表项（防 emitter 表按 turn 无界增长）"
        );

        // Act + Assert：幂等——重复注销不得 panic，且仍为空。
        escalation.unregister_emitter("turn-leak");
        assert!(
            escalation.take_emitter("turn-leak").is_none(),
            "重复注销必须幂等（仍无表项）"
        );
    }
}

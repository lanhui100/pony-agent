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
    pub fn register_emitter(&self, turn_id: &str, emitter: Arc<dyn TerminalEmitter>) {
        let _ = (turn_id, emitter);
        todo!("wave60")
    }

    /// stop 被接受（C1/C2 路径的唯一入口）后启动宽限期监督。
    pub fn on_stop_accepted(
        &self,
        turn_id: &str,
        session_id: Option<&str>,
        stop_reason: StopReason,
    ) {
        let _ = (turn_id, session_id, stop_reason);
        todo!("wave60")
    }
}

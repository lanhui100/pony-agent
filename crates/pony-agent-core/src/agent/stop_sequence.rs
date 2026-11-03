//! wave-60「可中断停止」：双击 Esc 的**纯判定状态机**（类型面冻结，无 IO / 无表面依赖）。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C8（`stop_sequence_ms = 500`，可注入）。
//!
//! 判据（本模块只表达状态，不决定谁去发停止请求）：
//! - 判定窗口内**同一身份** = (`session_id`, `turn_id`, `generation`) 全等；
//! - 第 1 次 `press`（流式中）仅「上膛」⇒ 返回 `false`，**不**发停止请求；
//! - 窗口内第 2 次同身份 `press` ⇒ 返回 `true`，调用方据此发**恰好一次**停止请求；
//! - 超出窗口 / 身份变化 / turn 结束 / 会话切换 ⇒ 上膛失效（重新上膛）。
//!
//! 冻结范围（wave60-T0）：类型与签名。`press` 函数体一律为 `todo!("wave60")`，
//! 行为实现归属 wave60-T5（TUI）与桌面端各自实现，但语义必须逐条对齐本判据表。

/// 双 Esc 的身份三元组：任一字段变化即视为「身份变化 ⇒ 上膛失效」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopIdentity {
    pub session_id: Option<String>,
    pub turn_id: String,
    pub generation: u64,
}

/// 纯判定状态机（不持锁、不产生副作用；调用方负责在正确时机 `reset`）。
#[allow(dead_code)] // wave60-T0：字段面冻结，读取方在 wave60-T5 实现时接入。
#[derive(Clone, Debug)]
pub struct StopSequenceState {
    interval_ms: u64,
    armed: Option<StopIdentity>,
    armed_at_ms: u64,
}

impl StopSequenceState {
    /// `interval_ms` = 判定窗口（`stop_sequence_ms`，NFR 缺省 500，测试可注入）。
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            armed: None,
            armed_at_ms: 0,
        }
    }

    /// 记一次按键。返回 `true` 表示「命中双击 ⇒ 应发恰好一次停止请求」。
    pub fn press(&mut self, identity: StopIdentity, now_ms: u64) -> bool {
        let _ = (identity, now_ms);
        todo!("wave60")
    }

    /// 清空上膛状态（turn 结束 / 会话切换 / 退出流式态时由调用方触发）。
    pub fn reset(&mut self) {
        self.armed = None;
        self.armed_at_ms = 0;
    }
}

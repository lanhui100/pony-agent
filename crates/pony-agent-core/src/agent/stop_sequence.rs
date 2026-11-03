//! wave-60「可中断停止」：双击 Esc 的**纯判定状态机**（无 IO / 无表面依赖）。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C8（`stop_sequence_ms = 500`，可注入）。
//!
//! 判据（本模块只表达状态，不决定谁去发停止请求）：
//! - 判定窗口内**同一身份** = (`session_id`, `turn_id`, `generation`) 全等；
//! - 第 1 次 `press`（流式中）仅「上膛」⇒ 返回 `false`，**不**发停止请求；
//! - 窗口内第 2 次同身份 `press` ⇒ 返回 `true`，调用方据此发**恰好一次**停止请求，随即复位；
//! - 超出窗口 / 身份变化 / turn 结束 / 会话切换 ⇒ 上膛失效（重新上膛）。
//!
//! 两端对齐：桌面端 `src/lib/runtime/stop-sequence.ts` 的 `StopSequenceState` 与本实现
//! 逐条同判据（同一「第 1 次只上膛 / 命中即复位 / 超窗重新上膛」表）。

/// NFR `stop_sequence_ms`：双击 Esc 的判定窗口缺省值（毫秒）。
pub const STOP_SEQUENCE_MS: u64 = 500;

/// 双 Esc 的身份三元组：任一字段变化即视为「身份变化 ⇒ 上膛失效」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopIdentity {
    pub session_id: Option<String>,
    pub turn_id: String,
    pub generation: u64,
}

/// 纯判定状态机（不持锁、不产生副作用；调用方负责在正确时机 `reset`）。
#[derive(Clone, Debug)]
pub struct StopSequenceState {
    interval_ms: u64,
    armed: Option<StopIdentity>,
    armed_at_ms: u64,
}

impl Default for StopSequenceState {
    fn default() -> Self {
        Self::new(STOP_SEQUENCE_MS)
    }
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

    /// 当前生效的判定窗口（毫秒）。
    pub fn interval_ms(&self) -> u64 {
        self.interval_ms
    }

    /// 记一次按键。返回 `true` 表示「命中双击 ⇒ 应发恰好一次停止请求」。
    ///
    /// 命中即复位（保证「恰好一次」）；超窗/换身份只重新上膛，**不**触发。
    /// 时钟回退（`now_ms < armed_at_ms`）按「无效窗口」处理：判不命中并以上次时刻重新上膛。
    pub fn press(&mut self, identity: StopIdentity, now_ms: u64) -> bool {
        let hit = match &self.armed {
            Some(armed) => {
                *armed == identity
                    && now_ms >= self.armed_at_ms
                    && now_ms - self.armed_at_ms < self.interval_ms
            }
            None => false,
        };

        if hit {
            self.armed = None;
            self.armed_at_ms = 0;
            return true;
        }

        self.armed = Some(identity);
        self.armed_at_ms = now_ms;
        false
    }

    /// 清空上膛状态（turn 结束 / 会话切换 / 退出流式态时由调用方触发）。
    pub fn reset(&mut self) {
        self.armed = None;
        self.armed_at_ms = 0;
    }
}

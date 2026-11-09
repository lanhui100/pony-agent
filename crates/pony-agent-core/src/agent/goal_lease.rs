//! PA-171 C1-T2（PA-172 §7.1 抽库）：goal 自动续轮的**单循环租约**。
//!
//! 租约是**按 `session_id` 分区的 `Arc<Mutex<HashSet<String>>>`**：键集合里每个元素是一个
//! 正在推进 goal 轮的会话 id，`insert` 返回 `false` 即该会话已有循环持有 ⇒ 判互斥。
//!
//! 为什么需要：每个用户提交都会各起一个轮次线程（TUI 侧 `F14`），两个线程各自的
//! `run_goal_rounds` 叠加成「2×8=16 轮」，正是 429 被重放的放大面。租约保证同一时刻
//! 同一会话只有一个续轮循环。
//!
//! 按会话分区（NFR `concurrency_lock` 与 ADR 0103：按会话互斥，跨会话互不阻塞）：
//! 同会话只允许一个续轮循环（消除 16=2×8 叠加），切换会话后 B 的续轮不会被 A 卡住。
//!
//! RAII：`Drop` 必释（正常返回 / panic 展开 / 提前 `return` 三条路径）。锁中毒按既有
//! fail-open 口径 `into_inner()` —— 临界区仅 `HashSet` 的 insert/remove，毒化后继续可用。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// 续轮循环租约注册表：**按 `session_id` 分区**（`HashSet` 元素 = 正在推进 goal 轮的会话）。
pub type GoalRoundLoopLeaseRegistry = Arc<Mutex<HashSet<String>>>;

/// 新建空注册表（TUI 事件循环持有一份，跨轮次线程共享）。
pub fn new_goal_round_loop_lease_registry() -> GoalRoundLoopLeaseRegistry {
    Arc::new(Mutex::new(HashSet::new()))
}

/// 单循环租约（RAII 释放）。
pub struct GoalRoundLoopLease {
    registry: GoalRoundLoopLeaseRegistry,
    session_id: String,
}

impl GoalRoundLoopLease {
    /// 抢不到（同会话已有循环）⇒ `None`，并落 `goal_rounds_loop_already_owned` 结构化告警。
    pub fn try_acquire(
        registry: &GoalRoundLoopLeaseRegistry,
        session_id: &str,
    ) -> Option<Self> {
        let acquired = {
            let mut held = registry
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            held.insert(session_id.to_string())
        };
        if acquired {
            return Some(Self {
                registry: Arc::clone(registry),
                session_id: session_id.to_string(),
            });
        }
        crate::agent::otel::log::log_json_with_fields(
            "warn",
            "goal_rounds_loop_already_owned",
            None,
            &[("session_id", serde_json::json!(session_id))],
        );
        None
    }

    /// 本租约所属会话 id。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for GoalRoundLoopLease {
    fn drop(&mut self) {
        let mut held = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        held.remove(&self.session_id);
    }
}

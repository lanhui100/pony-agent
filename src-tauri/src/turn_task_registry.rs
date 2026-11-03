//! wave-60 C6：turn 级任务注册表（key = **turn_id**，不再是 session_id）。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §2 C6。
//!
//! 背景：旧实现按 `session_id` 索引，同会话两轮并发时停止 A 会把 B 的句柄一起 abort。
//! wave-60 起 key 改为 `turn_id` —— 停止一轮只影响该轮自己的表项。
//!
//! **已知事实（不得对外谎报）**：`spawn_blocking` 不可抢占，`abort_turn` 成功只代表
//! 表项回收/取消通知已发出，不代表任务已停止；用户可见的"已停止"只能由终态事件驱动。
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;

pub struct TurnTaskRegistry {
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    max_concurrent: Option<usize>,
}

impl TurnTaskRegistry {
    pub fn with_max_concurrent(max: usize) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_concurrent: Some(max),
        }
    }

    /// 注册一轮任务（key = `turn_id`）。
    ///
    /// 并发上限按**活跃 turn 数**计（同一 turn_id 重注册不额外占额）；超限时本方法会
    /// abort 传入的句柄并返回错误（宿主据此提示用户等待）。
    pub fn register(&self, turn_id: String, handle: JoinHandle<()>) -> Result<(), String> {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        if let Some(limit) = self.max_concurrent {
            let current = if tasks.contains_key(&turn_id) {
                tasks.len().saturating_sub(1)
            } else {
                tasks.len()
            };
            if current >= limit {
                handle.abort();
                return Err(format!(
                    "已达最大并行对话数（{}），请等待当前对话完成后继续",
                    limit
                ));
            }
        }
        if let Some(prev) = tasks.remove(&turn_id) {
            prev.abort();
        }
        tasks.insert(turn_id, handle);
        Ok(())
    }

    /// 回收指定 turn 的表项（正常结束/清理时调用；**不** abort 句柄）。
    pub fn unregister(&self, turn_id: &str) {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        tasks.remove(turn_id);
    }

    /// abort 指定 turn 的句柄（L2 兜底）。命中并回收表项返回 `true`；未知/已回收返回 `false`（幂等）。
    pub fn abort_turn(&self, turn_id: &str) -> bool {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        if let Some(handle) = tasks.remove(turn_id) {
            handle.abort();
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn abort_all(&self) {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
    }
}

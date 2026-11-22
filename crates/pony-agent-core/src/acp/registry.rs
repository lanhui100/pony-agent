//! ACP 委派会话的**活跃注册表**（进程内、无网络、无磁盘）。
//!
//! 为什么需要这一层：`acp_delegate` 是**同步**工具调用 —— 子进程与 ACP 会话只存在于这一次
//! 调用的栈上，宿主界面上除了一行工具回执之外**没有任何可见面**（用户看不到「现在有哪些
//! ACP 子智能体在跑」）。本注册表在 spawn 成功后登记、在委派返回（成功/失败/取消/panic）时
//! 随 RAII guard 自动移除，给 TUI Agents 页签提供只读快照。
//!
//! 纪律：
//! - **不参与** ACP 生命周期、预算与权限裁决：注册表读写失败只影响可见性，绝不影响委派结果；
//! - 只登记不可变快照（agent / ACP session id / 宿主会话 / cwd / 起始时刻），不追踪输出正文，
//!   避免把委派内容泄露进任何状态查询通道；
//! - 与会话归属一致：`host_session_id` 来自 `ToolExecutionContext.session_id`，TUI 按当前会话过滤。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 一条活跃 ACP 委派（登记时的不可变快照）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveDelegate {
    /// 进程内单调编号（登记顺序，TUI 用它稳定排序）。
    pub delegate_id: u64,
    /// 外部 agent 的稳定标识（`AcpAgentSpec::id`，如 `opencode`）。
    pub agent_id: String,
    /// ACP 协议层的会话 id（`session/new` 回执）。
    pub acp_session_id: String,
    /// 发起委派的宿主会话（`ToolExecutionContext.session_id`；`None` = 无会话上下文）。
    pub host_session_id: Option<String>,
    /// `session/new` 使用的工作目录（绝对路径）。
    pub cwd: String,
    /// 登记时刻（Unix 毫秒）。
    pub started_at_ms: u64,
}

/// 登记入参：`delegate_id` / `started_at_ms` 由注册表生成，调用方不得自行编造。
#[derive(Debug, Clone)]
pub struct DelegateRegistration {
    pub agent_id: String,
    pub acp_session_id: String,
    pub host_session_id: Option<String>,
    pub cwd: String,
}

static ACTIVE_DELEGATES: Mutex<Option<BTreeMap<u64, ActiveDelegate>>> = Mutex::new(None);
static NEXT_DELEGATE_ID: AtomicU64 = AtomicU64::new(1);

/// 注册表锁：中毒不 panic（可见性面不得把委派线程带崩）。
fn registry_guard() -> std::sync::MutexGuard<'static, Option<BTreeMap<u64, ActiveDelegate>>> {
    ACTIVE_DELEGATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// RAII guard：`drop` 即从注册表移除。
///
/// 委派的四条收口路径（成功返回 / 错误返回 / 取消 / panic unwind）都经过 Drop，
/// 因此不存在「跑完还挂在界面上」的残留。
#[derive(Debug)]
pub struct ActiveDelegateGuard {
    delegate_id: u64,
}

impl ActiveDelegateGuard {
    /// 登记一条活跃委派并返回自动注销 guard。
    pub fn register(registration: DelegateRegistration) -> Self {
        let delegate_id = NEXT_DELEGATE_ID.fetch_add(1, Ordering::Relaxed);
        let entry = ActiveDelegate {
            delegate_id,
            agent_id: registration.agent_id,
            acp_session_id: registration.acp_session_id,
            host_session_id: registration
                .host_session_id
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty()),
            cwd: registration.cwd,
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                // Unix 毫秒 → u64：`as` 转换被 lint 表拒绝（可能截断），按 try_from 收口；
                // 超出 u64 的时间戳在物理上不可达，`unwrap_or(MAX)` 只作定理外的兜底。
                .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0),
        };
        registry_guard()
            .get_or_insert_with(BTreeMap::new)
            .insert(delegate_id, entry);
        Self { delegate_id }
    }

    pub fn delegate_id(&self) -> u64 {
        self.delegate_id
    }
}

impl Drop for ActiveDelegateGuard {
    fn drop(&mut self) {
        if let Some(map) = registry_guard().as_mut() {
            map.remove(&self.delegate_id);
        }
    }
}

/// 当前全部活跃委派（按登记顺序，稳定排序）。
pub fn active_delegates() -> Vec<ActiveDelegate> {
    registry_guard()
        .as_ref()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default()
}

/// 某个宿主会话名下的活跃委派（`None` 宿主 = 无会话上下文，不归任何会话）。
pub fn active_delegates_for_session(host_session_id: &str) -> Vec<ActiveDelegate> {
    let target = host_session_id.trim();
    active_delegates()
        .into_iter()
        .filter(|delegate| delegate.host_session_id.as_deref() == Some(target))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(host_session_id: Option<&str>) -> DelegateRegistration {
        DelegateRegistration {
            agent_id: "opencode".to_string(),
            acp_session_id: "acp-session-1".to_string(),
            host_session_id: host_session_id.map(str::to_string),
            cwd: "/tmp".to_string(),
        }
    }

    // 注册表是进程级静态：同一测试内串行使用，避免并发用例互相看见条目。
    #[test]
    fn guard_registers_and_deregisters_on_drop() {
        let before = active_delegates().len();
        let guard = ActiveDelegateGuard::register(registration(Some("host-a")));
        let id = guard.delegate_id();

        let listed = active_delegates();
        assert_eq!(listed.len(), before + 1, "登记后必须可见");
        let entry = listed
            .iter()
            .find(|delegate| delegate.delegate_id == id)
            .expect("登记条目必须可按 id 找到");
        assert_eq!(entry.agent_id, "opencode");
        assert_eq!(entry.acp_session_id, "acp-session-1");
        assert_eq!(entry.host_session_id.as_deref(), Some("host-a"));
        assert_eq!(entry.cwd, "/tmp");

        drop(guard);
        assert!(
            !active_delegates()
                .iter()
                .any(|delegate| delegate.delegate_id == id),
            "guard drop 后必须从注册表移除（不得残留「跑完还挂着」的条目）"
        );
    }

    #[test]
    fn session_filter_keeps_ownership_and_drops_unattributed() {
        let guard_a = ActiveDelegateGuard::register(registration(Some("host-a")));
        let id_a = guard_a.delegate_id();
        let guard_b = ActiveDelegateGuard::register(registration(Some("host-b")));
        let id_b = guard_b.delegate_id();
        let guard_none = ActiveDelegateGuard::register(registration(None));
        let id_none = guard_none.delegate_id();

        let for_a: Vec<u64> = active_delegates_for_session("host-a")
            .iter()
            .map(|delegate| delegate.delegate_id)
            .collect();
        assert!(for_a.contains(&id_a), "host-a 必须看到自己的委派");
        assert!(!for_a.contains(&id_b), "host-a 不得看到 host-b 的委派");
        assert!(
            !for_a.contains(&id_none),
            "无宿主会话的委派不归任何会话（不得串到任意会话的页签里）"
        );
    }
}

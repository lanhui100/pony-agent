//! turn 事件的全局持久化 / 冲刷注册表（PA-091 / PA-094）。
//!
//! 自 `agent/turn_flow.rs` 拆分（file-size 收口，行为逐字等价）：
//! 注册表静态量、会话绑定 guard 与 flush 入口整体搬迁，函数体与锁策略一字未改；
//! 父模块以 `pub use turn_flow_registry::*;` 保持既有 `turn_flow::<fn>` 寻址契约。

use super::*;
use std::sync::Arc;

static EVENT_PERSIST_REGISTRY: OnceLock<Mutex<Option<Arc<EventPersistFn>>>> = OnceLock::new();

fn event_persist_registry() -> &'static Mutex<Option<Arc<EventPersistFn>>> {
    EVENT_PERSIST_REGISTRY.get_or_init(|| Mutex::new(None))
}

/// 注册生产事件持久化通道（幂等：重复注册覆盖）。
pub fn register_event_persist(f: Arc<EventPersistFn>) {
    let mut slot = event_persist_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // PA-095 #3（实施后审核 P2）：覆盖非同源通道时告警——多控制面并存时
    // 后建者的单槽注册会劫持先建者未绑定会话的事件流（结构性已知问题，
    // 登记于任务卡；测试侧应使用会话绑定路由）。
    if let Some(previous) = slot.as_ref() {
        if !Arc::ptr_eq(previous, &f) {
            eprintln!(
                "[pony-agent][runtime] event persist channel overwritten by a new control plane (multi-control-plane setups must route per session)"
            );
        }
    }
    *slot = Some(f);
}

/// 清空事件持久化通道（测试用）。
pub fn clear_event_persist() {
    let mut slot = event_persist_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *slot = None;
}

/// PA-093：非 turn 生命周期事实（checkpoint/checkout、fork/created）经全局
/// 注册表落盘。turn_id 用语义化字面量（事件无 turn 归属，列非空）。
/// 未注册通道（测试/老路径）时静默跳过——与 turn 事件路径的 contained 语义一致。
/// PA-095 #3：is_terminal=false——history 命令在持有 sessions 写锁时调用本函数，
/// 终态内联 flush 会同线程重入写锁死锁（fork 实测挂死）；改为仅入缓冲，由
/// history 命令包装释放写锁后的 flush_session_buffered_events 提交（或并入下一条
/// turn 终态批）。内存后端 flush 失败 contained（与既有语义一致）。
pub fn emit_global_event(
    session_id: &str,
    turn_id: &str,
    event: crate::agent::turn_event::TurnEvent,
) {
    if let Some(persist) = resolve_event_persist(session_id) {
        persist(session_id, turn_id, event, false);
    }
}

/// 当前注册的持久化通道（None = 未注册，事件不落盘）。
fn current_event_persist() -> Option<Arc<EventPersistFn>> {
    event_persist_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// PA-095：分发事件到生产通道与测试多播 sink。生产单槽可被任意
/// HostControlPlane 构建覆盖（并行测试下互相抢占），测试多播 sink 只能被
/// 注册它的测试清除——依赖全局通道断言的测试必须走多播 sink。
pub(super) fn dispatch_event_persist(
    session_id: &str,
    turn_id: &str,
    event: crate::agent::turn_event::TurnEvent,
    is_terminal: bool,
) {
    // PA-095 #3（实施后审核 P1）：匿名 turn（session_id=None）归一到默认会话——
    // 其 history/trace 本就落默认会话，事件缓冲键若保留 "" 将无人 flush（永久滞留）。
    let resolved_session_id = if session_id.is_empty() {
        crate::agent::session::DEFAULT_SESSION_ID
    } else {
        session_id
    };
    if let Some(persist) = resolve_event_persist(resolved_session_id) {
        persist(resolved_session_id, turn_id, event.clone(), is_terminal);
    }
    #[cfg(test)]
    for sink in event_persist_test_sinks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
    {
        sink(session_id, turn_id, event.clone(), is_terminal);
    }
}

/// PA-095 #3：事件通道解析——会话绑定优先，回退全局默认单槽。会话绑定是
/// "按 session 所有权路由的多通道模型"（任务卡登记的结构性改进）的落地：
/// 并行测试中各场景把自身控制面的通道绑到自己的 session，全局默认槽被其他
/// 测试构建覆盖不再影响本会话的事件落盘；生产路径无绑定时行为不变。
///
/// TODO(PA-095 #3 后续任务)：session 绑定路由目前仅测试构建生效（见下方
/// `#[cfg(test)]` 分支）；生产构建恒走全局默认单槽（last-build-wins）。
/// 生产侧按 session 接线尚未实施，勿因本签名看似接收 session_id 而误以为已接线。
///
/// （参数仅测试构建使用，故带下划线前缀以通过非测试编译的 unused 检查。）
fn resolve_event_persist(_session_id: &str) -> Option<Arc<EventPersistFn>> {
    #[cfg(test)]
    if let Some(bound) = session_event_persist_binding(_session_id) {
        return Some(bound);
    }
    current_event_persist()
}

/// PA-095：测试专用多播 sink 注册表（与生产单槽独立）。
#[cfg(test)]
static EVENT_PERSIST_TEST_SINKS: OnceLock<Mutex<Vec<Arc<EventPersistFn>>>> = OnceLock::new();

#[cfg(test)]
fn event_persist_test_sinks() -> &'static Mutex<Vec<Arc<EventPersistFn>>> {
    EVENT_PERSIST_TEST_SINKS.get_or_init(|| Mutex::new(Vec::new()))
}

/// PA-095：注册测试专用多播持久化 sink（不受生产单槽覆盖影响）。
/// 返回守卫——Drop 时仅移除本 sink（并行测试互不清除对方注册）。
#[cfg(test)]
pub fn register_event_persist_test_sink(
    f: Arc<EventPersistFn>,
) -> EventPersistTestSinkGuard {
    let mut sinks = event_persist_test_sinks()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    sinks.push(Arc::clone(&f));
    drop(sinks);
    EventPersistTestSinkGuard { sink: f }
}

/// PA-095：测试多播 sink 守卫——Drop 按指针相等移除自身。
#[cfg(test)]
pub struct EventPersistTestSinkGuard {
    sink: Arc<EventPersistFn>,
}

#[cfg(test)]
impl Drop for EventPersistTestSinkGuard {
    fn drop(&mut self) {
        let mut sinks = event_persist_test_sinks()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        sinks.retain(|sink| !Arc::ptr_eq(sink, &self.sink));
    }
}

/// PA-095 #3：会话绑定事件持久化通道（测试专用）——session 所有权路由，
/// 优先于全局默认单槽；守卫 Drop 时解除绑定。
#[cfg(test)]
static SESSION_EVENT_PERSIST_BINDINGS: OnceLock<Mutex<HashMap<String, Arc<EventPersistFn>>>> =
    OnceLock::new();

#[cfg(test)]
fn session_event_persist_bindings(
) -> &'static Mutex<HashMap<String, Arc<EventPersistFn>>> {
    SESSION_EVENT_PERSIST_BINDINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn session_event_persist_binding(session_id: &str) -> Option<Arc<EventPersistFn>> {
    session_event_persist_bindings()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
}

/// PA-095 #3：绑定会话的事件持久化通道（测试专用）；返回守卫，Drop 解绑。
#[cfg(test)]
pub fn bind_event_persist_session(
    session_id: &str,
    f: Arc<EventPersistFn>,
) -> SessionEventPersistBindingGuard {
    session_event_persist_bindings()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.to_string(), Arc::clone(&f));
    SessionEventPersistBindingGuard {
        session_id: session_id.to_string(),
        bound: f,
    }
}

#[cfg(test)]
pub struct SessionEventPersistBindingGuard {
    session_id: String,
    bound: Arc<EventPersistFn>,
}

#[cfg(test)]
impl Drop for SessionEventPersistBindingGuard {
    fn drop(&mut self) {
        // PA-095 #3（实施后审核 P2）：仅当当前占用者仍是自己时才解绑——
        // 同名 session 的嵌套/并行绑定不得被先结束的守卫误摘。
        let mut bindings = session_event_persist_bindings()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if bindings
            .get(&self.session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.bound))
        {
            bindings.remove(&self.session_id);
        }
    }
}

/// PA-095 #2：会话级事件缓冲 flush 请求——append_turn 物化前确保该会话的
/// 缓冲事件已提交事件表（commit 先于清空，由注册闭包保证）。生产由
/// HostControlPlaneBuilder 注册（与持久化闭包同源缓冲）；未注册（无控制面 /
/// 测试直连 store）返回 Ok(0)。
pub type EventFlushFn = dyn Fn(&str) -> Result<usize, String> + Send + Sync;

static EVENT_FLUSH_REGISTRY: OnceLock<Mutex<Option<Arc<EventFlushFn>>>> = OnceLock::new();

/// PA-095 #3：会话绑定 flush 通道（测试专用）——与持久化绑定同原理。
#[cfg(test)]
static SESSION_EVENT_FLUSH_BINDINGS: OnceLock<Mutex<HashMap<String, Arc<EventFlushFn>>>> =
    OnceLock::new();

#[cfg(test)]
fn session_event_flush_bindings() -> &'static Mutex<HashMap<String, Arc<EventFlushFn>>> {
    SESSION_EVENT_FLUSH_BINDINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// PA-095 #3：绑定会话的 flush 请求通道（测试专用）；返回守卫，Drop 解绑。
#[cfg(test)]
pub fn bind_event_flush_session(
    session_id: &str,
    f: Arc<EventFlushFn>,
) -> SessionEventFlushBindingGuard {
    session_event_flush_bindings()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.to_string(), Arc::clone(&f));
    SessionEventFlushBindingGuard {
        session_id: session_id.to_string(),
        bound: f,
    }
}

#[cfg(test)]
pub struct SessionEventFlushBindingGuard {
    session_id: String,
    bound: Arc<EventFlushFn>,
}

#[cfg(test)]
impl Drop for SessionEventFlushBindingGuard {
    fn drop(&mut self) {
        // 同 persist 守卫——仅解绑自己（Arc::ptr_eq 校验）。
        let mut bindings = session_event_flush_bindings()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if bindings
            .get(&self.session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.bound))
        {
            bindings.remove(&self.session_id);
        }
    }
}

fn event_flush_registry() -> &'static Mutex<Option<Arc<EventFlushFn>>> {
    EVENT_FLUSH_REGISTRY.get_or_init(|| Mutex::new(None))
}

/// 注册会话级 flush 请求通道（幂等：重复注册覆盖）。
pub fn register_event_flush(f: Arc<EventFlushFn>) {
    let mut slot = event_flush_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *slot = Some(f);
}

/// 清空 flush 请求通道（测试用）。
pub fn clear_event_flush() {
    let mut slot = event_flush_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *slot = None;
}

/// 请求提交某会话的全部缓冲事件；返回提交的事件数（0 = 无缓冲）。
/// PA-095 #3：会话绑定 flush 通道优先（session 所有权路由），回退全局注册。
pub fn flush_session_buffered_events(session_id: &str) -> Result<usize, String> {
    #[cfg(test)]
    if let Some(bound) = session_event_flush_bindings()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
    {
        return bound(session_id);
    }
    match event_flush_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|f| Arc::clone(f) as Arc<EventFlushFn>)
    {
        Some(flush) => flush(session_id),
        None => Ok(0),
    }
}

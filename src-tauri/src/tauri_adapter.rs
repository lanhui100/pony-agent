use crate::agent::control_plane::{HostControlPlane, PreparedGraphRunStream};
use crate::agent::retry::{install_retry_observer, RetryNotice, RetryObserverGuard};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use crate::turn_task_registry::TurnTaskRegistry;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// 上游重试进度的桌面侧事件名（`turn:*` 家族，但**不是**回合生命周期事件）。
pub const TURN_RETRY_EVENT: &str = "turn:retry";

/// `turn:retry` 的线缆负载（camelCase，与前端 `TurnRetryEvent` 一一对应）。
///
/// 真值源是 [`crate::agent::retry::RetryNotice`]（与 TUI 状态行同一出口）：
/// `attempt` = **已失败**的尝试次数（1 起），`max_attempts` = 本跳允许的尝试总数（含首次）
/// ⇒ 前端口径 `attempt/maxAttempts`，分母与分子同源，不虚构进度。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRetryPayload {
    pub turn_id: String,
    pub session_id: String,
    pub attempt: u32,
    pub max_attempts: u32,
    /// 本次退避将 sleep 的毫秒数（尚未执行），供前端做说明性展示。
    pub delay_ms: u64,
    /// 失败分类（`agent::retry::classify_error_class` 口径：timeout / rate_limited / …）。
    pub error_class: String,
}

pub struct TauriTurnEventSink {
    app: AppHandle,
}

impl TauriTurnEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// 重试进度出口：复用**同一个** sink（本文件是桌面侧唯一的 `app.emit` 落点），
    /// 但走独立事件名 —— 重试进度不参与 `TurnStreamEvent` 回合状态机，故不复用其线缆形状
    /// （与 TUI 的 `UiEvent::ProviderRetry` 对称，不是第二条回合事件通道）。
    pub fn emit_retry(&self, payload: TurnRetryPayload) {
        if let Err(e) = self.app.emit(TURN_RETRY_EVENT, payload) {
            eprintln!(
                "[pony-agent] turn retry emit failed (name={TURN_RETRY_EVENT}): {e}"
            );
        }
    }
}

/// 在**当前线程**安装上游重试观察者（桌面侧装配点）。
///
/// 为什么必须装在执行回合的阻塞线程内：观察者是 `agent::retry` 的 thread_local，而一次
/// provider 调用（含其内部重试）与回合执行同步在同一线程 —— 装错线程 ⇒ 观察者永不触发
/// （界面 0 信号，历史上正是如此才被迫伪造进度）。返回值守卫生命周期 = 本回合，`drop` 即摘除。
fn install_turn_retry_observer(
    sink: Arc<TauriTurnEventSink>,
    turn_id: String,
    session_id: String,
) -> RetryObserverGuard {
    install_retry_observer(Box::new(move |notice: &RetryNotice| {
        sink.emit_retry(TurnRetryPayload {
            turn_id: turn_id.clone(),
            session_id: session_id.clone(),
            attempt: notice.attempt,
            max_attempts: notice.max_attempts,
            delay_ms: notice.delay_ms,
            error_class: notice.error_class.clone(),
        });
    }))
}

impl TurnEventSink for TauriTurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        if let Err(e) = self.app.emit(name, payload) {
            eprintln!("[pony-agent] turn event emit failed (name={name}): {e}");
        }
    }
}

/// wave-60 C6：清理守卫按 **turn_id** 回收表项（同会话其它轮次不受影响），
/// 并注销该轮注册的事件出口（sink 生命周期与 turn 对齐，避免长跑进程里表项累积）。
struct TaskCleanupGuard {
    app: AppHandle,
    turn_id: String,
}

impl TaskCleanupGuard {
    fn new(app: AppHandle, turn_id: String) -> Self {
        Self { app, turn_id }
    }
}

impl Drop for TaskCleanupGuard {
    fn drop(&mut self) {
        self.app
            .state::<TurnTaskRegistry>()
            .unregister(&self.turn_id);
        self.app
            .state::<HostControlPlane>()
            .unregister_turn_event_sink(&self.turn_id);
    }
}

/// 在阻塞线程中执行流式回合任务：建 sink、spawn_blocking 跑控制面、统一记录失败。
///
/// wave-60 C2/C5：spawn 时向控制面注册该 **turn** 的事件出口 ——
/// `register_turn_event_sink` 同时把"经同一 sink 发 `turn:cancelled`"的终端发射器
/// 绑到宽限期监督者上（sink 是唯一出口，避免 Tauri 侧再造第二条事件通道）。
async fn run_stream_task<F>(
    app: AppHandle,
    turn_id: String,
    session_id: String,
    task_kind: &'static str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&AppHandle, &TauriTurnEventSink) -> Result<(), String> + Send + 'static,
{
    let _cleanup = TaskCleanupGuard::new(app.clone(), turn_id.clone());
    // L3-A BLOCKING-4：sink/emitter 的注册**不在这里**（那已经在 spawn 之后，存在
    // "turn 可被 stop 却还没 sink" 的窗口）——统一由命令入口 `register_turn_sink`
    // 在 prepare 之后、spawn 之前同步完成。
    let sink = Arc::new(TauriTurnEventSink::new(app.clone()));
    let sink_for_task = Arc::clone(&sink);
    // 上游重试进度（`turn:retry`）：观察者是 thread_local，必须装在**真正执行回合的阻塞线程**内，
    // 生命周期 = 本闭包（回合结束即摘除）。这是桌面侧唯一装配点 —— 三个流式命令入口都走这里。
    let retry_turn_id = turn_id.clone();
    let retry_session_id = session_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _retry_observer = install_turn_retry_observer(
            Arc::clone(&sink_for_task),
            retry_turn_id,
            retry_session_id,
        );
        f(&app, sink_for_task.as_ref())
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            eprintln!(
                "[pony-agent] {task_kind} task turn={turn_id} session={session_id} failed: {e}"
            );
            Err(e)
        }
        Err(e) => {
            eprintln!(
                "[pony-agent] {task_kind} task turn={turn_id} session={session_id} join failed: {e}"
            );
            Err(format!("{task_kind} task turn={turn_id} join failed: {e}"))
        }
    }
}

/// spawn 任务并注册进 registry（**key = turn_id**，C6）。注册失败（如并发超限）时
/// `register` 内部已 abort 句柄，此处仅补充错误上下文返回给调用方。
fn spawn_stream_task<F>(
    app: &AppHandle,
    turn_id: String,
    session_id: String,
    task_kind: &'static str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&AppHandle, &TauriTurnEventSink) -> Result<(), String> + Send + 'static,
{
    let app_handle = app.clone();
    let task_turn_id = turn_id.clone();
    let task_session_id = session_id.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let _ = run_stream_task(app_handle, task_turn_id, task_session_id, task_kind, f).await;
    });
    app.state::<TurnTaskRegistry>()
        .register(turn_id, handle)
        .map_err(|e| format!("{task_kind} task register failed: {e}"))
}

/// wave-60 C2/C5（L3-A BLOCKING-4）：在 **turn 可被 stop 之前**为该轮注册事件出口。
///
/// 调用时序必须是 `prepare_*`（内部 `register_turn`，此后该 turn 就能被 `stop_turn`
/// 命中）**之后**、`spawn_*` **之前**、且在命令处理线程内**同步**完成 —— 否则
/// "turn 已可被 stop 但还没有 sink" 的窗口里 stop 被接受时：无 `turn:stop_requested`、
/// 监督者 `take_emitter()` 得 None ⇒ C5 终态永不派发（事后补注册也无效）。
/// 幂等：同一 turn 重复调用只是覆盖为等价出口。
pub fn register_turn_sink(app: &AppHandle, turn_id: &str) {
    let control_plane = app.state::<HostControlPlane>();
    control_plane.register_turn_event_sink(turn_id, Arc::new(TauriTurnEventSink::new(app.clone())));
}

pub fn spawn_graph_run_stream(
    app: &AppHandle,
    prepared: PreparedGraphRunStream,
) -> Result<(), String> {
    // C6：注册表 key 用 turn_id；session_id 仅用于日志/会话绑定。
    let turn_id = prepared.turn_id.clone();
    let session_id = prepared
        .input
        .session_id
        .clone()
        .unwrap_or_else(|| prepared.run_id.clone());
    spawn_stream_task(app, turn_id, session_id, "graph run", move |app, sink| {
        let control_plane = app.state::<HostControlPlane>();
        control_plane
            .execute_graph_run_stream(sink, prepared)
            .map(|_| ())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::retry::{notify_provider_retry, RetryNotice};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sample_payload() -> TurnRetryPayload {
        TurnRetryPayload {
            turn_id: "turn-1".to_string(),
            session_id: "session-1".to_string(),
            attempt: 2,
            max_attempts: 10,
            delay_ms: 4000,
            error_class: "timeout".to_string(),
        }
    }

    fn sample_notice() -> RetryNotice {
        RetryNotice {
            label: "decision".to_string(),
            scope: "decision",
            attempt: 2,
            max_attempts: 10,
            delay_ms: 4000,
            error_class: "timeout".to_string(),
        }
    }

    /// 线缆形状锁定：前端 `TurnRetryEvent` 依赖这批 camelCase 键名（改键名 = 前端静默丢字段）。
    #[test]
    fn retry_payload_serializes_to_camel_case_keys() {
        let wire = serde_json::to_value(sample_payload()).expect("payload 必须可序列化");
        let object = wire.as_object().expect("负载必须是 JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["attempt", "delayMs", "errorClass", "maxAttempts", "sessionId", "turnId"]
        );
        assert_eq!(object["turnId"], serde_json::json!("turn-1"));
        assert_eq!(object["attempt"], serde_json::json!(2));
        assert_eq!(object["maxAttempts"], serde_json::json!(10));
    }

    /// 事件名稳定：前端 `safeListen("turn:retry")` 与后端常量必须同一字面量。
    #[test]
    fn retry_event_name_is_stable() {
        assert_eq!(TURN_RETRY_EVENT, "turn:retry");
    }

    /// 观察者是 thread_local：装配点必须落在**执行回合的阻塞线程**，否则退避重试期间
    /// 观察者永不触发（这正是历史上界面拿不到真实进度、被迫伪造「重试中(1/5)」的根因）。
    #[test]
    fn retry_observer_only_fires_on_the_installing_thread() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_in_observer = Arc::clone(&hits);
        let _guard = install_retry_observer(Box::new(move |_notice: &RetryNotice| {
            hits_in_observer.fetch_add(1, Ordering::SeqCst);
        }));

        notify_provider_retry(&sample_notice());
        assert_eq!(hits.load(Ordering::SeqCst), 1, "同线程通知必须到达观察者");

        let hits_in_other_thread = Arc::clone(&hits);
        std::thread::spawn(move || {
            // 另一线程未安装观察者 ⇒ 静默 no-op，不得打到本线程装的闭包。
            notify_provider_retry(&sample_notice());
            assert_eq!(
                hits_in_other_thread.load(Ordering::SeqCst),
                1,
                "其它线程的通知不得投递到本线程观察者"
            );
        })
        .join()
        .expect("子线程必须正常结束");

        // 守卫 drop 后摘除：同一线程再通知一次不得再计。
        drop(_guard);
        notify_provider_retry(&sample_notice());
        assert_eq!(hits.load(Ordering::SeqCst), 1, "守卫 drop 必须摘除观察者");
    }
}

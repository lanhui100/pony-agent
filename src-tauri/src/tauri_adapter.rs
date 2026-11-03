use crate::agent::control_plane::{HostControlPlane, PreparedGraphRunStream};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use crate::turn_task_registry::TurnTaskRegistry;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

pub struct TauriTurnEventSink {
    app: AppHandle,
}

impl TauriTurnEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
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
    let result =
        tauri::async_runtime::spawn_blocking(move || f(&app, sink_for_task.as_ref())).await;
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

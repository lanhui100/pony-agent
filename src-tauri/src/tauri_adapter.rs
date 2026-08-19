use crate::agent::control_plane::{
    HostControlPlane, PreparedGraphRunStream, StartTurnStreamCommand,
};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use crate::turn_task_registry::TurnTaskRegistry;
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

struct TaskCleanupGuard {
    app: AppHandle,
    session_id: String,
}

impl TaskCleanupGuard {
    fn new(app: AppHandle, session_id: String) -> Self {
        Self { app, session_id }
    }
}

impl Drop for TaskCleanupGuard {
    fn drop(&mut self) {
        self.app
            .state::<TurnTaskRegistry>()
            .unregister(&self.session_id);
    }
}

/// 在阻塞线程中执行流式回合任务：建 sink、spawn_blocking 跑控制面、统一记录失败。
async fn run_stream_task<F>(
    app: AppHandle,
    session_id: String,
    task_kind: &'static str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&AppHandle, &TauriTurnEventSink) -> Result<(), String> + Send + 'static,
{
    let _cleanup = TaskCleanupGuard::new(app.clone(), session_id.clone());
    let sink = TauriTurnEventSink::new(app.clone());
    let result = tauri::async_runtime::spawn_blocking(move || f(&app, &sink)).await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            eprintln!("[pony-agent] {task_kind} task {session_id} failed: {e}");
            Err(e)
        }
        Err(e) => {
            eprintln!("[pony-agent] {task_kind} task {session_id} join failed: {e}");
            Err(format!("{task_kind} task {session_id} join failed: {e}"))
        }
    }
}

/// spawn 任务并注册进 registry。注册失败（如并发超限）时 `register` 内部已 abort 句柄，
/// 此处仅补充错误上下文返回给调用方。
fn spawn_stream_task<F>(
    app: &AppHandle,
    session_id: String,
    task_kind: &'static str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&AppHandle, &TauriTurnEventSink) -> Result<(), String> + Send + 'static,
{
    let app_handle = app.clone();
    let task_session_id = session_id.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let _ = run_stream_task(app_handle, task_session_id, task_kind, f).await;
    });
    app.state::<TurnTaskRegistry>()
        .register(session_id, handle)
        .map_err(|e| format!("{task_kind} task register failed: {e}"))
}

pub fn spawn_turn_stream(app: &AppHandle, command: StartTurnStreamCommand) -> Result<(), String> {
    // session_id 缺失时以 turn_id 兜底，避免多个无 session 任务共用空 key 互相替换
    let session_id = command
        .input
        .session_id
        .clone()
        .unwrap_or_else(|| command.turn_id.clone());
    spawn_stream_task(app, session_id, "turn", move |app, sink| {
        let control_plane = app.state::<HostControlPlane>();
        control_plane.start_turn_stream(sink, command);
        Ok(())
    })
}

pub fn spawn_graph_run_stream(
    app: &AppHandle,
    prepared: PreparedGraphRunStream,
) -> Result<(), String> {
    // session_id 缺失时以 run_id 兜底，避免多个无 session 任务共用空 key 互相替换
    let session_id = prepared
        .input
        .session_id
        .clone()
        .unwrap_or_else(|| prepared.run_id.clone());
    spawn_stream_task(app, session_id, "graph run", move |app, sink| {
        let control_plane = app.state::<HostControlPlane>();
        control_plane
            .execute_graph_run_stream(sink, prepared)
            .map(|_| ())
    })
}

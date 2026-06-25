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
        let _ = self.app.emit(name, payload);
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
        self.app.state::<TurnTaskRegistry>().unregister(&self.session_id);
    }
}

pub fn spawn_turn_stream(app: &AppHandle, command: StartTurnStreamCommand) -> Result<(), String> {
    let session_id = command.input.session_id.clone().unwrap_or_default();
    let app_handle = app.clone();
    let session_id_for_register = session_id.clone();

    let handle = tauri::async_runtime::spawn(async move {
        let _cleanup = TaskCleanupGuard::new(app_handle.clone(), session_id.clone());
        let sink = TauriTurnEventSink::new(app_handle.clone());
        let result = tauri::async_runtime::spawn_blocking(move || {
            let control_plane = app_handle.state::<HostControlPlane>();
            control_plane.start_turn_stream(&sink, command);
        })
        .await;
        if let Err(e) = result {
            eprintln!("turn task {session_id} failed: {e}");
        }
    });
    app.state::<TurnTaskRegistry>().register(session_id_for_register, handle)
}

pub fn spawn_graph_run_stream(app: &AppHandle, prepared: PreparedGraphRunStream) -> Result<(), String> {
    let session_id = prepared.input.session_id.clone().unwrap_or_default();
    let app_handle = app.clone();
    let session_id_for_register = session_id.clone();

    let handle = tauri::async_runtime::spawn(async move {
        let _cleanup = TaskCleanupGuard::new(app_handle.clone(), session_id.clone());
        let sink = TauriTurnEventSink::new(app_handle.clone());
        let result = tauri::async_runtime::spawn_blocking(move || {
            let control_plane = app_handle.state::<HostControlPlane>();
            let _ = control_plane.execute_graph_run_stream(&sink, prepared);
        })
        .await;
        if let Err(e) = result {
            eprintln!("graph run task {session_id} failed: {e}");
        }
    });
    app.state::<TurnTaskRegistry>().register(session_id_for_register, handle)
}

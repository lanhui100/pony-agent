use crate::agent::control_plane::{
    HostControlPlane, PreparedGraphRunStream, StartTurnStreamCommand,
};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
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

pub fn spawn_turn_stream(app: AppHandle, command: StartTurnStreamCommand) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let control_plane = app_handle.state::<HostControlPlane>();
        let sink = TauriTurnEventSink::new(app_handle.clone());
        control_plane.start_turn_stream(&sink, command);
    });
}

pub fn spawn_graph_run_stream(app: AppHandle, prepared: PreparedGraphRunStream) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let control_plane = app_handle.state::<HostControlPlane>();
        let sink = TauriTurnEventSink::new(app_handle.clone());
        let _ = control_plane.execute_graph_run_stream(&sink, prepared);
    });
}

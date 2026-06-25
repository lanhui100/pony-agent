use std::collections::HashMap;
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;

pub struct TurnTaskRegistry {
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl TurnTaskRegistry {
    pub fn new() -> Self {
        Self { tasks: Mutex::new(HashMap::new()) }
    }

    pub fn register(&self, session_id: String, handle: JoinHandle<()>) {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        if let Some(prev) = tasks.remove(&session_id) {
            prev.abort();
        }
        tasks.insert(session_id, handle);
    }

    pub fn unregister(&self, session_id: &str) {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        tasks.remove(session_id);
    }

    pub fn abort_all(&self) {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
    }
}

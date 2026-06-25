use std::collections::HashMap;
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;

pub struct TurnTaskRegistry {
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    max_concurrent: Option<usize>,
}

impl TurnTaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_concurrent: None,
        }
    }

    pub fn with_max_concurrent(max: usize) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_concurrent: Some(max),
        }
    }

    pub fn register(&self, session_id: String, handle: JoinHandle<()>) -> Result<(), String> {
        let mut tasks = self.tasks.lock().expect("turn task registry lock poisoned");
        if let Some(limit) = self.max_concurrent {
            let current = if tasks.contains_key(&session_id) {
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
        if let Some(prev) = tasks.remove(&session_id) {
            prev.abort();
        }
        tasks.insert(session_id, handle);
        Ok(())
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

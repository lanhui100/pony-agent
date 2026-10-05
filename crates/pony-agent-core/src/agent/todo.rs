use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoWriteArgs {
    pub todos: Vec<TodoItem>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoWriteResult {
    pub success: bool,
    pub count: usize,
    pub in_progress_count: usize,
    pub completed_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoListArgs {
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoListResult {
    pub todos: Vec<TodoItem>,
}

static TODO_STORE: Mutex<Option<HashMap<String, Vec<TodoItem>>>> = Mutex::new(None);

pub fn todo_write(args: TodoWriteArgs) -> Result<TodoWriteResult, String> {
    let session = args.session_id.unwrap_or_else(|| "default".to_string());
    let mut lock = TODO_STORE.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    let map = lock.as_mut().unwrap();

    let mut in_progress = 0;
    let mut completed = 0;
    for item in &args.todos {
        if item.content.trim().is_empty() {
            return Err("Todo item content cannot be empty".to_string());
        }
        match item.status {
            TodoStatus::InProgress => in_progress += 1,
            TodoStatus::Completed => completed += 1,
            TodoStatus::Pending => {}
        }
    }

    let count = args.todos.len();
    map.insert(session, args.todos);

    Ok(TodoWriteResult {
        success: true,
        count,
        in_progress_count: in_progress,
        completed_count: completed,
    })
}

pub fn todo_list(args: TodoListArgs) -> Result<TodoListResult, String> {
    let session = args.session_id.unwrap_or_else(|| "default".to_string());
    let lock = TODO_STORE.lock().unwrap();
    let todos = if let Some(ref map) = *lock {
        map.get(&session).cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(TodoListResult { todos })
}

//! PA-112: 计划与轻量任务管理工具 (todo_write / todo_list) 验收测试

use pony_agent_core::agent::tools::{
    todo_list, todo_write, TodoItem, TodoListArgs, TodoStatus, TodoWriteArgs,
};

#[test]
fn test_todo_write_and_list() {
    let session = "test-session-1".to_string();
    let write_args = TodoWriteArgs {
        session_id: Some(session.clone()),
        todos: vec![
            TodoItem {
                content: "Task 1".to_string(),
                status: TodoStatus::Completed,
            },
            TodoItem {
                content: "Task 2".to_string(),
                status: TodoStatus::InProgress,
            },
            TodoItem {
                content: "Task 3".to_string(),
                status: TodoStatus::Pending,
            },
        ],
    };

    let res = todo_write(write_args).expect("todo_write should succeed");
    assert!(res.success);
    assert_eq!(res.count, 3);
    assert_eq!(res.in_progress_count, 1);
    assert_eq!(res.completed_count, 1);

    let list_res = todo_list(TodoListArgs {
        session_id: Some(session),
    })
    .expect("todo_list should succeed");
    assert_eq!(list_res.todos.len(), 3);
    assert_eq!(list_res.todos[1].status, TodoStatus::InProgress);
}

#[test]
fn test_todo_empty_content_rejected() {
    let write_args = TodoWriteArgs {
        session_id: Some("test-session-2".to_string()),
        todos: vec![TodoItem {
            content: "   ".to_string(),
            status: TodoStatus::Pending,
        }],
    };
    assert!(todo_write(write_args).is_err(), "Empty todo content must fail");
}

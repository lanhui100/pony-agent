//! B1795774426544 回归：Todo 回灌标签必须是 `Todos:`，不得再冒名 `Plan:`。
//!
//! 旧实现把 `todo_list` 的结果标成 `Plan: x/y completed`，于是同一轮里模型同时看到
//! 「Plan 工具（步骤机）」与「Plan 回灌（其实是 Todo）」，两套语义互相顶替——这正是
//! ADR `2026-11-27-plan-mode-dsh-alignment-and-task-count` 里「Plan 与 Todo 串名」的
//! 直接证据。本用例把标签钉死，并把 `Plan:` 前缀列为负例。
//!
//! 单列文件的原因与 `host_history_tests.rs` 相同：`context.rs` 已顶到 `size:check`
//! 的 3000 行门禁，净增行数会直接判红（不得抬基线）。

use super::*;

#[test]
fn volatile_context_labels_todo_rollup_as_todos_never_plan() {
    // Arrange
    let builder = DefaultTurnContextBuilder;
    let provider = provider_manager(
        "deepseek-chat",
        1024,
        ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            ..Default::default()
        },
    );
    // 独立 session id 隔离进程级 todo 注册表（HashMap），避免与其它用例串数据。
    let session_id = "session-todos-label-contract";
    crate::agent::todo::todo_write(crate::agent::todo::TodoWriteArgs {
        todos: vec![
            crate::agent::todo::TodoItem::new(
                "step one",
                crate::agent::todo::TodoStatus::Completed,
            ),
            crate::agent::todo::TodoItem::new(
                "step two",
                crate::agent::todo::TodoStatus::InProgress,
            ),
        ],
        session_id: Some(session_id.to_string()),
    })
    .expect("todo_write must succeed");

    let mut session = session_snapshot(Vec::new(), Vec::new(), None);
    session.conversation_id = session_id.to_string();

    // Act
    let retrieved = builder.retrieve_context_state("hi", &[], None, &session, None, None);
    let request = builder.build_request("coding", &provider, &retrieved, &[]);

    // Assert
    let note = request
        .input
        .iter()
        .find(|message| message.content.contains("Todos:"))
        .map(|message| message.content.clone())
        .unwrap_or_else(|| {
            panic!(
                "B1795774426544 违约：动态上下文未出现 `Todos:` 回灌行；实际分层消息 = {:?}",
                request
                    .input
                    .iter()
                    .map(|message| message.content.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert!(
        note.contains("Todos: 1/2 completed; in_progress: step two."),
        "B1795774426544 违约：Todo 回灌行格式变化，实际 = {note}"
    );
    assert!(
        !note.contains("Plan: 1/2 completed"),
        "B1795774426544 违约：Todo 回灌仍冒名 `Plan:`（模型会把 Todo 当成 Plan 模式步骤机）"
    );
}

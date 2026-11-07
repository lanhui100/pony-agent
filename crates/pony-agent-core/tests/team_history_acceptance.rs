use pony_agent_core::agent::team_history::*;
use pony_agent_core::agent::orchestration::{TeamTaskItem, TeammateMember, TeammateStatus};
use pony_agent_core::agent::session::SessionOverview;
use pony_agent_core::agent::turn_event::TurnEvent;

#[test]
fn test_history_snapshot_from_events_logic() {
    let session = SessionOverview {
        conversation_id: "session-123".to_string(),
        title: "Test".to_string(),
        summary: "".to_string(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 1000,
        workspace_id: None,
        archived: false,
        continuation_only: false,
    };
    
    let fake_events = vec![
        (1, "main".to_string(), TurnEvent::ToolResult {
            turn_id: "t1".to_string(),
            step: 1,
            call_id: "c1".to_string(),
            result: Some(r#"{"tool":"team_task_create","result":{"task_id":"task-1","subject":"Task 1","description":"","status":"pending","owner":null,"revision":1,"write_scopes":[],"blocked_by":[],"created_at_ms":100}}"#.to_string()),
            error: None,
            status: None,
            duration_ms: None,
            artifacts: None,
            capability_invocation: None,
        })
    ];

    let snap = team_history_snapshot_from_events(&[session], |_| fake_events.clone());
    assert_eq!(snap.tasks.len(), 1);
    assert_eq!(snap.tasks[0].task.task_id, "task-1");
    assert_eq!(snap.tasks[0].source_session_id, Some("session-123".to_string()));
    assert!(snap.tasks[0].is_historical);
}

//! PA-110: 智能体派生与协作编排工具族黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-110-agent-teams-and-subagent-tools.md`

use pony_agent_core::agent::tools::{
    subagent, team_task_create, team_task_list, team_task_update, workflow,
    SubagentArgs, TeamTaskAction, TeamTaskCreateArgs, TeamTaskUpdateArgs,
    WorkflowArgs, WorkflowStep,
};

#[test]
fn test_ac01_subagent_dispatch() {
    let args = SubagentArgs {
        prompt: "Analyze rust codebase".to_string(),
        description: "Codebase analysis".to_string(),
        run_in_background: Some(false),
    };
    let res = subagent(args).expect("subagent call should succeed");
    assert!(!res.subagent_id.is_empty());
    assert_eq!(res.status, "completed");
    assert!(res.output.is_some());
}

#[test]
fn test_ac02_workflow_dag_execution() {
    let args = WorkflowArgs {
        name: "test-pipeline".to_string(),
        steps: vec![
            WorkflowStep {
                id: "step1".to_string(),
                action: "compile".to_string(),
                depends_on: vec![],
            },
            WorkflowStep {
                id: "step2".to_string(),
                action: "test".to_string(),
                depends_on: vec!["step1".to_string()],
            },
        ],
    };
    let res = workflow(args).expect("workflow should succeed");
    assert_eq!(res.status, "success");
    assert_eq!(res.executed_steps, vec!["step1", "step2"]);
}

#[test]
fn test_ac03_team_tasks_lifecycle_and_cas() {
    let create_args = TeamTaskCreateArgs {
        subject: "Implement PTY".to_string(),
        description: "Task description".to_string(),
        write_scopes: Some(vec!["src/agent/terminal.rs".to_string()]),
        blocked_by: None,
    };
    let task = team_task_create(create_args).expect("task create should succeed");
    assert_eq!(task.status, "pending");
    assert_eq!(task.revision, 1);

    // 认领
    let claim_args = TeamTaskUpdateArgs {
        task_id: task.task_id.clone(),
        expected_revision: task.revision,
        action: TeamTaskAction::Claim,
        owner: Some("executor-1".to_string()),
    };
    let claimed = team_task_update(claim_args).expect("claim should succeed");
    assert_eq!(claimed.status, "in_progress");
    assert_eq!(claimed.revision, 2);

    // CAS 保护验证：旧 revision 应失败
    let stale_args = TeamTaskUpdateArgs {
        task_id: task.task_id.clone(),
        expected_revision: 1,
        action: TeamTaskAction::Complete,
        owner: None,
    };
    assert!(team_task_update(stale_args).is_err(), "Stale update must fail");

    // 正确完成
    let comp_args = TeamTaskUpdateArgs {
        task_id: task.task_id.clone(),
        expected_revision: 2,
        action: TeamTaskAction::Complete,
        owner: None,
    };
    let completed = team_task_update(comp_args).expect("complete should succeed");
    assert_eq!(completed.status, "completed");

    let list = team_task_list().expect("list should succeed");
    assert!(list.tasks.iter().any(|t| t.task_id == task.task_id));
}

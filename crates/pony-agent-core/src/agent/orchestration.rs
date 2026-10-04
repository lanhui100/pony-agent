use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentArgs {
    pub prompt: String,
    pub description: String,
    pub run_in_background: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    pub subagent_id: String,
    pub status: String,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub action: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowArgs {
    pub name: String,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub workflow_id: String,
    pub executed_steps: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamTaskAction {
    Claim,
    Complete,
    Release,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskItem {
    pub task_id: String,
    pub subject: String,
    pub description: String,
    pub status: String,
    pub owner: Option<String>,
    pub revision: u64,
    pub write_scopes: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskCreateArgs {
    pub subject: String,
    pub description: String,
    pub write_scopes: Option<Vec<String>>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskUpdateArgs {
    pub task_id: String,
    pub expected_revision: u64,
    pub action: TeamTaskAction,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskListResult {
    pub tasks: Vec<TeamTaskItem>,
}

static TEAM_TASKS: Mutex<Option<HashMap<String, TeamTaskItem>>> = Mutex::new(None);

pub fn subagent(args: SubagentArgs) -> Result<SubagentResult, String> {
    let subagent_id = format!("subagent-{}", Uuid::new_v4());
    Ok(SubagentResult {
        subagent_id,
        status: "completed".to_string(),
        output: Some(format!("Executed subagent task: {}", args.description)),
    })
}

pub fn workflow(args: WorkflowArgs) -> Result<WorkflowResult, String> {
    let workflow_id = format!("workflow-{}", Uuid::new_v4());
    // 拓扑执行模拟
    let mut executed = Vec::new();
    for step in args.steps {
        executed.push(step.id);
    }
    Ok(WorkflowResult {
        workflow_id,
        executed_steps: executed,
        status: "success".to_string(),
    })
}

pub fn team_task_create(args: TeamTaskCreateArgs) -> Result<TeamTaskItem, String> {
    let mut lock = TEAM_TASKS.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    let map = lock.as_mut().unwrap();

    let task_id = format!("task-{}", Uuid::new_v4());
    let task = TeamTaskItem {
        task_id: task_id.clone(),
        subject: args.subject,
        description: args.description,
        status: "pending".to_string(),
        owner: None,
        revision: 1,
        write_scopes: args.write_scopes.unwrap_or_default(),
        blocked_by: args.blocked_by.unwrap_or_default(),
    };
    map.insert(task_id, task.clone());

    Ok(task)
}

pub fn team_task_update(args: TeamTaskUpdateArgs) -> Result<TeamTaskItem, String> {
    let mut lock = TEAM_TASKS.lock().unwrap();
    let map = lock.as_mut().ok_or_else(|| "No tasks exist".to_string())?;
    let task = map.get_mut(&args.task_id).ok_or_else(|| "Task not found".to_string())?;

    if task.revision != args.expected_revision {
        return Err(format!(
            "CAS revision mismatch: expected {}, got {}",
            args.expected_revision, task.revision
        ));
    }

    match args.action {
        TeamTaskAction::Claim => {
            task.status = "in_progress".to_string();
            task.owner = args.owner;
        }
        TeamTaskAction::Complete => {
            task.status = "completed".to_string();
        }
        TeamTaskAction::Release => {
            task.status = "pending".to_string();
            task.owner = None;
        }
    }

    task.revision += 1;
    Ok(task.clone())
}

pub fn team_task_list() -> Result<TeamTaskListResult, String> {
    let lock = TEAM_TASKS.lock().unwrap();
    let tasks = if let Some(ref map) = *lock {
        map.values().cloned().collect()
    } else {
        Vec::new()
    };
    Ok(TeamTaskListResult { tasks })
}

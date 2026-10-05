use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

// ==========================================
// 1. Subagent (子智能体) 契约
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentArgs {
    pub prompt: String,
    pub description: String,
    pub run_in_background: Option<bool>,
    pub fork_context: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    pub subagent_id: String,
    pub status: String,
    pub output: Option<String>,
}

// ==========================================
// 2. Workflow (DAG 任务流编排) 契约
// ==========================================

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

// ==========================================
// 3. Agent Teams (花名册 Roster + 邮箱 Mailbox) 契约
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeammateStatus {
    Provisioning,
    Active,
    Running,
    Inactive,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateMember {
    pub target: String,
    pub name: String,
    pub role: String,
    pub status: TeammateStatus,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeammateArgs {
    pub name: String,
    pub role: Option<String>,
    pub description: String,
    pub prompt: String,
    pub context: Option<String>, // fresh | fork
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeammateResult {
    pub member: TeammateMember,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAgentsResult {
    pub agents: Vec<TeammateMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageArgs {
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResult {
    pub message_id: String,
    pub delivered: bool,
}

// ==========================================
// 4. Team Task Board (共享看板与写域) 契约
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamTaskAction {
    Claim,
    Complete,
    Release,
    Delete,
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

// ==========================================
// 全局状态管理
// ==========================================

static TEAM_TASKS: Mutex<Option<HashMap<String, TeamTaskItem>>> = Mutex::new(None);
static TEAMMATES: Mutex<Option<HashMap<String, TeammateMember>>> = Mutex::new(None);

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

pub fn spawn_teammate(args: SpawnTeammateArgs) -> Result<SpawnTeammateResult, String> {
    let mut lock = TEAMMATES.lock().unwrap();
    if lock.is_none() {
        let mut map = HashMap::new();
        // 默认插入 Lead 节点
        map.insert(
            "lead".to_string(),
            TeammateMember {
                target: "lead".to_string(),
                name: "Lead".to_string(),
                role: "lead".to_string(),
                status: TeammateStatus::Running,
                description: "Team Lead Orchestrator".to_string(),
            },
        );
        *lock = Some(map);
    }
    let map = lock.as_mut().unwrap();

    let target = args.name.to_lowercase().replace(' ', "-");
    let member = TeammateMember {
        target: target.clone(),
        name: args.name,
        role: args.role.unwrap_or_else(|| "teammate".to_string()),
        status: TeammateStatus::Active,
        description: args.description,
    };
    map.insert(target, member.clone());

    Ok(SpawnTeammateResult { member })
}

pub fn list_agents() -> Result<ListAgentsResult, String> {
    let mut lock = TEAMMATES.lock().unwrap();
    if lock.is_none() {
        let mut map = HashMap::new();
        map.insert(
            "lead".to_string(),
            TeammateMember {
                target: "lead".to_string(),
                name: "Lead".to_string(),
                role: "lead".to_string(),
                status: TeammateStatus::Running,
                description: "Team Lead Orchestrator".to_string(),
            },
        );
        *lock = Some(map);
    }
    let map = lock.as_ref().unwrap();
    Ok(ListAgentsResult {
        agents: map.values().cloned().collect(),
    })
}

pub fn send_message(args: SendMessageArgs) -> Result<SendMessageResult, String> {
    let lock = TEAMMATES.lock().unwrap();
    let map = lock.as_ref().ok_or_else(|| "No active team".to_string())?;
    if !map.contains_key(&args.target) {
        return Err(format!("Teammate target not found: {}", args.target));
    }

    Ok(SendMessageResult {
        message_id: format!("msg-{}", Uuid::new_v4()),
        delivered: true,
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
        TeamTaskAction::Delete => {
            task.status = "deleted".to_string();
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

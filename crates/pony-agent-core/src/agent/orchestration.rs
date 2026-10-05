use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
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
// 3. Agent Teams (花名册 Roster + 邮箱 Mailbox + 消息调度) 契约
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

/// 投递优先级模式：普通排队 (Queue/Followup) vs 紧急插队 (Steer)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageDeliveryMode {
    Queue,
    Steer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageArgs {
    pub target: String,
    pub message: String,
    pub mode: Option<MessageDeliveryMode>, // 默认为 Queue (排队)，可指定 Steer (插队)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResult {
    pub message_id: String,
    pub delivered: bool,
    pub mode: MessageDeliveryMode,
    pub pending_inbox_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub id: String,
    pub content: String,
    pub is_steer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptAgentArgs {
    pub target: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptAgentResult {
    pub success: bool,
    pub previous_status: TeammateStatus,
    pub new_status: TeammateStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadInboxArgs {
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadInboxResult {
    pub target: String,
    pub messages: Vec<QueuedMessage>,
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
// 全局状态管理与双向信道
// ==========================================

static TEAM_TASKS: Mutex<Option<HashMap<String, TeamTaskItem>>> = Mutex::new(None);
static TEAMMATES: Mutex<Option<HashMap<String, TeammateMember>>> = Mutex::new(None);
/// 每个 Teammate 对应的优先级双端信箱 (双端队列支持普通排队 push_back 与插队 push_front)
static TEAMMATE_INBOXES: Mutex<Option<HashMap<String, VecDeque<QueuedMessage>>>> = Mutex::new(None);

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
    let target = args.name.to_lowercase().replace(' ', "-");
    let member = TeammateMember {
        target: target.clone(),
        name: args.name,
        role: args.role.unwrap_or_else(|| "teammate".to_string()),
        status: TeammateStatus::Active,
        description: args.description,
    };

    // 1. 在独立的作用域内先完成 TEAMMATES 登记，立即释放锁
    {
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
        let map = lock.as_mut().unwrap();
        map.insert(target.clone(), member.clone());
    }

    // 2. 初始化该 Agent 的独立收件箱（无锁嵌套，避免跨 Mutex 死锁）
    {
        let mut inbox_lock = TEAMMATE_INBOXES.lock().unwrap();
        if inbox_lock.is_none() {
            *inbox_lock = Some(HashMap::new());
        }
        inbox_lock.as_mut().unwrap().insert(target, VecDeque::new());
    }

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

/// 支持排队 (Queue) 与插队 (Steer) 发送消息
pub fn send_message(args: SendMessageArgs) -> Result<SendMessageResult, String> {
    let mode = args.mode.unwrap_or(MessageDeliveryMode::Queue);
    let msg_id = format!("msg-{}", Uuid::new_v4());

    // 1. 检验目标并更新状态，立即释放 TEAMMATES 锁
    {
        let mut lock = TEAMMATES.lock().unwrap();
        let map = lock.as_mut().ok_or_else(|| "No active team".to_string())?;
        let member = map
            .get_mut(&args.target)
            .ok_or_else(|| format!("Teammate target not found: {}", args.target))?;

        match mode {
            MessageDeliveryMode::Steer => {
                if member.status == TeammateStatus::Inactive || member.status == TeammateStatus::Active {
                    member.status = TeammateStatus::Running;
                }
            }
            MessageDeliveryMode::Queue => {
                if member.status == TeammateStatus::Inactive {
                    member.status = TeammateStatus::Active;
                }
            }
        }
    }

    // 2. 独立获取 TEAMMATE_INBOXES 锁并推入队列，绝不与 TEAMMATES 双重嵌套
    let len = {
        let mut inbox_lock = TEAMMATE_INBOXES.lock().unwrap();
        if inbox_lock.is_none() {
            *inbox_lock = Some(HashMap::new());
        }
        let inbox_map = inbox_lock.as_mut().unwrap();
        let deque = inbox_map.entry(args.target.clone()).or_insert_with(VecDeque::new);

        let queued_msg = QueuedMessage {
            id: msg_id.clone(),
            content: args.message,
            is_steer: mode == MessageDeliveryMode::Steer,
        };

        match mode {
            MessageDeliveryMode::Steer => {
                deque.push_front(queued_msg);
            }
            MessageDeliveryMode::Queue => {
                deque.push_back(queued_msg);
            }
        }
        deque.len()
    };

    Ok(SendMessageResult {
        message_id: msg_id,
        delivered: true,
        mode,
        pending_inbox_len: len,
    })
}

/// 打断正在运行的智能体（保留其信箱队列）
pub fn interrupt_agent(args: InterruptAgentArgs) -> Result<InterruptAgentResult, String> {
    let mut lock = TEAMMATES.lock().unwrap();
    let map = lock.as_mut().ok_or_else(|| "No active team".to_string())?;
    let member = map.get_mut(&args.target).ok_or_else(|| format!("Teammate target not found: {}", args.target))?;

    let prev = member.status.clone();
    member.status = TeammateStatus::Inactive;

    Ok(InterruptAgentResult {
        success: true,
        previous_status: prev,
        new_status: TeammateStatus::Inactive,
    })
}

/// 读取并消费目标智能体收件箱的消息
pub fn drain_inbox(args: ReadInboxArgs) -> Result<ReadInboxResult, String> {
    let mut inbox_lock = TEAMMATE_INBOXES.lock().unwrap();
    let inbox_map = inbox_lock.as_mut().ok_or_else(|| "No inboxes".to_string())?;
    let deque = inbox_map.get_mut(&args.target).ok_or_else(|| format!("No inbox for {}", args.target))?;

    let messages: Vec<QueuedMessage> = deque.drain(..).collect();
    Ok(ReadInboxResult {
        target: args.target,
        messages,
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

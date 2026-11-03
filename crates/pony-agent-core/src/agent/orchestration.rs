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
    Edit,
    SetDependencies,
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
    /// 创建时刻（Unix 毫秒）。任务板以它为「生成次序」的稳定排序键；
    /// 同一毫秒内创建的多个任务以 task_id 作为次级键保证确定性。
    pub created_at_ms: u64,
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
    pub subject: Option<String>,
    pub description: Option<String>,
    pub write_scopes: Option<Vec<String>>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskGetArgs {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitAgentArgs {
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitAgentNoProgress {
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitAgentResult {
    pub timed_out: bool,
    pub no_progress: Option<WaitAgentNoProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskListResult {
    pub tasks: Vec<TeamTaskItem>,
}

// ==========================================
// 全局状态管理与双向信道
// ==========================================

/// 当前 Unix 毫秒时间戳（系统时钟早于 UNIX_EPOCH 时退化为 0，不 panic）。
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

static TEAM_TASKS: Mutex<Option<HashMap<String, TeamTaskItem>>> = Mutex::new(None);

/// 子代理 turn 超时（NFR 基线：subagent_turn_timeout_ms=120000）。
const SUBAGENT_TURN_TIMEOUT_MS: u64 = 120_000;
/// 名册里的 Lead 伪成员 target（与 `list_agents` / `spawn_teammate` 建队时同源）。
const LEAD_TARGET: &str = "lead";
static TEAMMATES: Mutex<Option<HashMap<String, TeammateMember>>> = Mutex::new(None);
/// 每个 Teammate 对应的优先级双端信箱 (双端队列支持普通排队 push_back 与插队 push_front)
static TEAMMATE_INBOXES: Mutex<Option<HashMap<String, VecDeque<QueuedMessage>>>> = Mutex::new(None);

/// wave-37 C3：真实子代理执行入口。
///
/// - 引擎槽为空 → 结构化错误（含 `capability_unavailable`），不再返回伪字符串；
/// - 引擎在位 → 错误（busy/oversize/timeout/引擎失败）原样透传；
/// - 同步调用：成功填 `status:"completed"` + `output:Some(final_text)`；
/// - `run_in_background=true`：spawn 线程执行并立即返回 `status:"running"` +
///   `output:None`，后台结果仅记录日志后丢弃。
/// `workflow()` 保持 wave-38 范围现状，勿动。
pub fn subagent(args: SubagentArgs) -> Result<SubagentResult, String> {
    subagent_in_workspace(args, None)
}

/// Wave-61 C8.3：显式工作区根的子代理入口（`None` = 引擎自行解析）。
///
/// 桌面宿主的真实工作区在 `ToolRouter.workspace_root`（`tools.rs:1358`），并不等于
/// `compute_default_workspace_root()`；由 tools 层把该根传进来，子代理才能继承父会话的
/// 项目指令与项目技能（wave-58 F1-5）。许可门与错误契约与 [`subagent`] 完全一致
/// （`subagent` 只是本函数 `None` 的薄封装，旧签名保留给既有测试）。
pub fn subagent_in_workspace(
    args: SubagentArgs,
    workspace_root: Option<&std::path::Path>,
) -> Result<SubagentResult, String> {
    let engine = crate::agent::subagent_runtime::subagent_engine().ok_or_else(|| {
        "capability_unavailable: subagent engine not wired (HostTurnEngine missing)".to_string()
    })?;
    // 入口门（oversize / busy）：**只对不自持门的引擎**生效。
    //
    // Wave-61 C.1：`HostTurnEngine::run_turn` 内部已持门（`gates_concurrency_internally()`
    // 返回 true），入口若再持门则一次调用扣两个许可 → 有效并发从 2 掉到 1。因此对自持引擎
    // 跳过入口门（许可唯一真源＝引擎）。
    // 注入式引擎（测试 Stub / 宿主自定义）默认不自持，入口门必须兜住 —— wave-37 C3.2
    // 契约要求「Stub 引擎注入场景下 busy/oversize 契约同样生效」（t3_j / t3_k 的锚点）。
    let gate = if engine.gates_concurrency_internally() {
        None
    } else {
        Some(crate::agent::subagent_runtime::subagent_gate_enter(&args.prompt)?)
    };
    let subagent_id = format!("subagent-{}", Uuid::new_v4());
    let is_bg = args.run_in_background.unwrap_or(false);

    if is_bg {
        let bg_id = subagent_id.clone();
        let bg_root = workspace_root.map(std::path::Path::to_path_buf);
        std::thread::spawn(move || {
            // 后台 turn 全程占许可（仅非自持引擎会携带 Some）。
            let _gate = gate;
            if let Err(err) =
                engine.run_turn_in(&args.prompt, SUBAGENT_TURN_TIMEOUT_MS, bg_root.as_deref())
            {
                eprintln!("[subagent] background turn failed ({}): {}", bg_id, err);
            }
        });
        return Ok(SubagentResult {
            subagent_id,
            status: "running".to_string(),
            output: None,
        });
    }

    // 同步路径：许可 guard 活到本次调用结束（仅非自持引擎会携带 Some）。
    let _gate = gate;
    match engine.run_turn_in(&args.prompt, SUBAGENT_TURN_TIMEOUT_MS, workspace_root) {
        Ok(outcome) => Ok(SubagentResult {
            subagent_id,
            status: "completed".to_string(),
            output: Some(outcome.final_text),
        }),
        Err(err) => Err(err),
    }
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

/// 派生队友（**纯登记**，不含投递/驱动）。
///
/// 库层语义保持 wave-61 之前不变：冻结用例
/// （`tests/pa110_teams_acceptance.rs:209`、`tests/teammate_e2e_communication_test.rs`）
/// 依赖「派生本身不往收件箱塞消息」这一可观测事实。模型可见的工具入口用
/// [`spawn_teammate_in_workspace`]（登记 + 投递初始 prompt + 驱动）。
pub fn spawn_teammate(args: SpawnTeammateArgs) -> Result<SpawnTeammateResult, String> {
    let (result, _target, _prompt) = register_teammate(args)?;
    Ok(result)
}

/// Wave-61 C7 + C8.3：派生队友并在引擎在位时**真实启动**它的首个 turn。
///
/// 注册成员后把 `args.prompt` 作为首条消息投递进该队友收件箱并（引擎在位时）后台驱动一次
/// —— 否则模型收到 `status:ok` 却没人真正开始工作，与 `workflow` 桩同类的假能力
/// （L3-A/L3-P 裁决 BLOCKING-1）。无引擎时保持纯登记（fail-closed，不伪造进展）。
pub fn spawn_teammate_in_workspace(
    args: SpawnTeammateArgs,
    workspace_root: Option<&std::path::Path>,
) -> Result<SpawnTeammateResult, String> {
    let auto_drive = crate::agent::subagent_runtime::subagent_engine().is_some();
    let (result, target, initial_prompt) = register_teammate(args)?;

    // Wave-61 C7：引擎在位时把初始 prompt 投递并驱动（复用 send_message 的入队 + 后台
    // drive 通路，不新造存储；无引擎时保持纯登记，行为与既有 `spawn_teammate` 一致）。
    if auto_drive && !initial_prompt.trim().is_empty() {
        let _ = send_message_in_workspace(
            SendMessageArgs {
                target,
                message: initial_prompt,
                mode: Some(MessageDeliveryMode::Queue),
            },
            workspace_root,
        );
    }

    Ok(result)
}

/// 登记队友（成员表 + 独立收件箱）并返回 (结果, target, 初始 prompt)。
fn register_teammate(args: SpawnTeammateArgs) -> Result<(SpawnTeammateResult, String, String), String> {
    let initial_prompt = args.prompt.clone();
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

    // 2. 初始化该 Agent 的独立收件箱（无锁嵌套，避免跨 Mutex 死锁）。
    //    Wave-61 C7：同时为 `lead` 建箱 —— 队友 turn 的回执与错误都要投递到 lead 收件箱，
    //    `lead` 是建队时必定存在的伪成员（与 `list_agents` 同源），其收件箱必须同源初始化。
    {
        let mut inbox_lock = TEAMMATE_INBOXES.lock().unwrap();
        if inbox_lock.is_none() {
            *inbox_lock = Some(HashMap::new());
        }
        let inbox_map = inbox_lock.as_mut().unwrap();
        inbox_map
            .entry(LEAD_TARGET.to_string())
            .or_insert_with(VecDeque::new);
        inbox_map.insert(target.clone(), VecDeque::new());
    }

    Ok((SpawnTeammateResult { member }, target, initial_prompt))
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

/// 支持排队 (Queue) 与插队 (Steer) 发送消息。
///
/// Wave-61 C7.1：入队成功后，若子代理引擎在位则**后台**驱动一次队友 turn ——
/// 让 `send_message` 产生真实效果（不再是"只入队却永远没人跑"的假能力）。
/// 无引擎时仅入队（fail-closed）。工作区根见 [`send_message_in_workspace`]。
pub fn send_message(args: SendMessageArgs) -> Result<SendMessageResult, String> {
    send_message_in_workspace(args, None)
}

/// Wave-61 C7.1 + C8.3：入队后若子代理引擎在位，**后台**驱动一次队友 turn，
/// 让 `send_message` 在模型可见路径上产生真实效果（而不是只入队却永远没人跑）。
/// 无引擎时仅入队（保持 fail-closed，不伪造成功）。
pub fn send_message_in_workspace(
    args: SendMessageArgs,
    workspace_root: Option<&std::path::Path>,
) -> Result<SendMessageResult, String> {
    let (result, drive_target) = enqueue_message(args)?;

    if drive_target != "lead" && crate::agent::subagent_runtime::subagent_engine().is_some() {
        let owner = workspace_root.map(std::path::Path::to_path_buf);
        set_teammate_status(&drive_target, TeammateStatus::Running);
        std::thread::spawn(move || {
            if let Err(err) = drive_teammate_turn_in(&drive_target, owner.as_deref()) {
                set_teammate_status(&drive_target, TeammateStatus::Inactive);
                eprintln!("[team] teammate drive failed ({}): {}", drive_target, err);
            }
        });
    }

    Ok(result)
}

/// 纯入队 seam：只把消息写进目标收件箱，**不触发**任何驱动/状态机副作用。
///
/// 这是队列层入口：`send_message_in_workspace`（以及库层 `send_message`）在调用本函数
/// 拿到结果后，才按需把 `Running` 置位并派生后台 drive；因此：
/// - 需要「只验证排队 / 插队顺序」的调用方（含契约测试）用本 seam，观测面不含驱动竞态；
/// - 需要「入队即真实驱动」的调用方用 `send_message_in_workspace`。
/// 返回 `(入队结果, 目标 target)`，便于驱动方复用同一 target 文本。
pub fn enqueue_message(args: SendMessageArgs) -> Result<(SendMessageResult, String), String> {
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

    Ok((
        SendMessageResult {
            message_id: msg_id,
            delivered: true,
            mode,
            pending_inbox_len: len,
        },
        args.target,
    ))
}

/// 队友状态写入（独立作用域持锁，绝不与收件箱锁嵌套，避免锁序反转）。
fn set_teammate_status(target: &str, status: TeammateStatus) {
    if let Ok(mut lock) = TEAMMATES.lock() {
        if let Some(map) = lock.as_mut() {
            if let Some(member) = map.get_mut(target) {
                member.status = status;
            }
        }
    }
}

/// 读取队友当前状态（无锁嵌套）。
fn teammate_status(target: &str) -> Option<TeammateStatus> {
    TEAMMATES
        .lock()
        .ok()
        .and_then(|lock| lock.as_ref().and_then(|map| map.get(target).map(|m| m.status.clone())))
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
    // Wave-61 C7：`lead` 等**已存在成员**可能还没有独立信箱（历史上只有 spawn 的 target 才会
    // 建箱），此时惰性建空箱并返回空结果，而不是把「团队健在但某人还没收过信」误报成错误。
    // 完全不认识的 target 仍保持原错误语义（fail-closed，不放宽到任意字符串）。
    let known_member = {
        // 独立作用域持 TEAMMATES 锁，绝不与收件箱锁嵌套（防锁序反转）。
        TEAMMATES
            .lock()
            .ok()
            .and_then(|lock| lock.as_ref().map(|map| map.contains_key(&args.target)))
            .unwrap_or(false)
    };

    let mut inbox_lock = TEAMMATE_INBOXES.lock().unwrap();
    let inbox_map = inbox_lock.as_mut().ok_or_else(|| "No inboxes".to_string())?;
    if !inbox_map.contains_key(&args.target) {
        if !known_member {
            return Err(format!("No inbox for {}", args.target));
        }
        inbox_map.insert(args.target.clone(), VecDeque::new());
    }
    let deque = inbox_map
        .get_mut(&args.target)
        .ok_or_else(|| format!("No inbox for {}", args.target))?;

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
        created_at_ms: now_unix_ms(),
    };
    map.insert(task_id, task.clone());

    Ok(task)
}

pub fn team_task_get(args: TeamTaskGetArgs) -> Result<TeamTaskItem, String> {
    let lock = TEAM_TASKS.lock().unwrap();
    let map = lock.as_ref().ok_or_else(|| "No tasks exist".to_string())?;
    let task = map.get(&args.task_id).ok_or_else(|| format!("Task not found: {}", args.task_id))?;
    Ok(task.clone())
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
        TeamTaskAction::Edit => {
            if let Some(subject) = args.subject {
                task.subject = subject;
            }
            if let Some(description) = args.description {
                task.description = description;
            }
            if let Some(scopes) = args.write_scopes {
                task.write_scopes = scopes;
            }
            if let Some(owner) = args.owner {
                task.owner = Some(owner);
            }
        }
        TeamTaskAction::SetDependencies => {
            if let Some(deps) = args.blocked_by {
                task.blocked_by = deps;
            }
        }
    }

    task.revision += 1;
    Ok(task.clone())
}

/// 等待队友活动（对齐 DSH wait_agent 边沿机制）
pub fn wait_agent(args: WaitAgentArgs) -> Result<WaitAgentResult, String> {
    let has_active_peer = {
        let lock = TEAMMATES.lock().unwrap();
        if let Some(ref map) = *lock {
            map.iter().any(|(target, member)| {
                target != "lead" && (member.status == TeammateStatus::Running || member.status == TeammateStatus::Provisioning)
            })
        } else {
            false
        }
    };

    if !has_active_peer {
        return Ok(WaitAgentResult {
            timed_out: false,
            no_progress: Some(WaitAgentNoProgress {
                reason: "no-active-peer".to_string(),
                message: "No other Team member is running or provisioning.".to_string(),
            }),
        });
    }

    let _timeout = args.timeout_ms.unwrap_or(30_000);
    // 模拟等待返回
    Ok(WaitAgentResult {
        timed_out: false,
        no_progress: None,
    })
}

/// 自动驱动队员处理消息的循环执行回路：若在位真实引擎则驱动真实 Turn，并可回信给 sender/lead
pub fn drive_teammate_turn(target: &str) -> Result<Option<String>, String> {
    drive_teammate_turn_in(target, None)
}

/// Wave-61 C7 + C8.3：显式工作区根版本的队友驱动回路。
///
/// 状态机（C7.2）：`Running`（进入）→ 执行 turn → `Inactive`（收口，成功与失败都收口）。
/// - `interrupt_agent` 只需把状态翻成 `Inactive`，本函数在两个检查点观察它并提前返回；
/// - 已知限制（C7.4）：引擎无 stop API，**已经进入**的 turn 无法中途打断，只能在其返回后
///   由收口状态机覆盖 —— 这是当前引擎契约的硬边界，不做伪 stop；
/// - 失败不再伪造成 `[Execution error]` 文本回执，而是把错误文本投递到 Lead 收件箱并返回
///   `Err`（`drain_inbox` 可取），让上层看到真实失败。
pub fn drive_teammate_turn_in(
    target: &str,
    workspace_root: Option<&std::path::Path>,
) -> Result<Option<String>, String> {
    // 检查点 #1（drain 之前）：已被 `interrupt_agent` 置为 Inactive → 本轮不再启动。
    if teammate_status(target) == Some(TeammateStatus::Inactive) {
        return Ok(None);
    }

    let result = drain_inbox(ReadInboxArgs {
        target: target.to_string(),
    })?;
    if result.messages.is_empty() {
        return Ok(None);
    }

    // 启动前置：成员置为 Running（对 `list_agents` 可见，C7.5 的行为锚点）。
    set_teammate_status(target, TeammateStatus::Running);

    // 整合收件箱内的消息内容
    let mut combined_prompt = format!("You are teammate '{}'. Incoming messages:\n", target);
    for msg in &result.messages {
        combined_prompt.push_str(&format!("- {}\n", msg.content));
    }
    combined_prompt.push_str("\nProcess these messages and provide your response or action report.");

    // 如果挂载了真实模型引擎，则执行真实 Turn（C8：以父会话工作区根建栈）。
    let engine = crate::agent::subagent_runtime::subagent_engine();
    let engine_was_wired = engine.is_some();
    let final_reply = match engine {
        Some(engine) => {
            match engine.run_turn_in(&combined_prompt, SUBAGENT_TURN_TIMEOUT_MS, workspace_root) {
                Ok(outcome) => outcome.final_text,
                Err(err) => {
                    // 失败收口：状态回 Inactive + 错误文本进 Lead 收件箱（不含任何假成功）。
                    set_teammate_status(target, TeammateStatus::Inactive);
                    if target != "lead" {
                        let _ = send_message(SendMessageArgs {
                            target: "lead".to_string(),
                            message: format!("Teammate '{}' turn failed: {}", target, err),
                            mode: Some(MessageDeliveryMode::Queue),
                        });
                    }
                    return Err(err);
                }
            }
        }
        None => {
            // 无引擎：保持既有降级回执与状态口径（fail-closed，不驱动真实 turn）。
            // 注意：`Running → Inactive` 收口只适用于**引擎真正执行**的路径（C7.2）；
            // 无引擎时沿用既有 `Active` 口径，避免与冻结的 e2e 用例
            // （`tests/teammate_e2e_communication_test.rs` 第 5 步）冲突。
            set_teammate_status(target, TeammateStatus::Active);
            format!(
                "Processed {} message(s) for teammate {}",
                result.messages.len(),
                target
            )
        }
    };

    // 收口（C7.2）：引擎真正执行过 → `Running → Inactive`；无引擎的降级路径沿用既有
    // `Active` 口径（见上方 None 分支注释，避免与冻结 e2e 用例冲突）。
    set_teammate_status(
        target,
        if engine_was_wired {
            TeammateStatus::Inactive
        } else {
            TeammateStatus::Active
        },
    );

    // 将产出的答复作为回执发送给 lead（经既有 TEAMMATE_INBOXES 投递，`drain_inbox` 可取）。
    if target != "lead" {
        let _ = send_message(SendMessageArgs {
            target: "lead".to_string(),
            message: format!("Teammate '{}' finished turn:\n{}", target, final_reply),
            mode: Some(MessageDeliveryMode::Queue),
        });
    }

    Ok(Some(final_reply))
}

pub fn team_task_list() -> Result<TeamTaskListResult, String> {
    let lock = TEAM_TASKS.lock().unwrap();
    let mut tasks: Vec<TeamTaskItem> = if let Some(ref map) = *lock {
        map.values().cloned().collect()
    } else {
        Vec::new()
    };
    // 生成次序：created_at_ms 升序，同毫秒以 task_id 作次级键，保证确定性。
    tasks.sort_by(|a, b| {
        a.created_at_ms
            .cmp(&b.created_at_ms)
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    Ok(TeamTaskListResult { tasks })
}

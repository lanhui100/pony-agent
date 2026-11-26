use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::agent::config::{ProviderRegistryStore, ResolvedProviderSelection};

// ==========================================
// 1. Subagent (子智能体) 契约
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentArgs {
    pub prompt: String,
    pub description: String,
    pub run_in_background: Option<bool>,
    pub fork_context: Option<bool>,
    /// W2（additive）：可选 provider 覆盖；`None` = 继承宿主 provider。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// W2（additive）：可选 model 覆盖；`None` = 继承宿主 model。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
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
    /// 预留：异步创建（今天没有任何写入点，不得对外宣称已产生）。
    Provisioning,
    /// 空闲：已登记且有待驱动消息（或刚派生尚未开始跑）。
    Active,
    /// 有 turn 正在执行。
    Running,
    /// 空闲：上一轮 turn 正常收口。
    Inactive,
    /// 上一轮 turn 驱动返回错误（错误文本见 `TeammateMember::last_error`）。
    Failed,
    /// 被 `interrupt_agent` 在下一 drain 边界前停下。
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateMember {
    pub target: String,
    pub name: String,
    pub role: String,
    pub status: TeammateStatus,
    pub description: String,
    /// 读时快照：是否有后台驱动线程正在跑该 target（真值源是 `DRIVING` 单飞标记）。
    ///
    /// 只由 `list_agents` 在返回前填充；成员表里恒为 `false`，不参与持久化语义。
    #[serde(default)]
    pub is_driving: bool,
    /// 上一轮驱动失败的错误文本；成功收口或被重新驱动时清空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeammateArgs {
    pub name: String,
    pub role: Option<String>,
    pub description: String,
    pub prompt: String,
    pub context: Option<String>, // fresh | fork
    /// W2（additive）：可选 provider 覆盖；`None` = 继承宿主 provider。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// W2（additive）：可选 model 覆盖；`None` = 继承宿主 model。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeammateResult {
    /// 返回时**回读**的成员快照（不是登记快照）：驱动已派发则为 `running`，
    /// 空 prompt / 引擎未接线则为 `active`，派生即失败的极端竞态可为 `inactive`。
    pub member: TeammateMember,
    /// 初始 prompt 的派发阶段：`driving`（已派发后台驱动）/ `queued`（仅入队）/
    /// `not_started`（空 prompt 或引擎未接线，未投递任何消息）/ `failed`（投递报错）。
    #[serde(default)]
    pub delivery_state: String,
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
    /// 消息 id。
    pub message_id: String,
    /// 是否已成功**入队**（历史语义，保持不变；不等于"已被引擎处理"）。
    pub delivered: bool,
    pub mode: MessageDeliveryMode,
    pub pending_inbox_len: usize,
    /// Wave-61 C10.2（additive）：本条消息是否已被引擎消费完成。
    ///
    /// 后台驱动是异步的，入队返回时恒为 `false`；调用方不得把 `delivered=true` 当作
    /// "已处理"。消费失败时整批消息会被放回收件箱（不丢失），并另投一条错误回执给 Lead。
    #[serde(default)]
    pub processed: bool,
    /// Wave-61 C10.2（additive）：投递阶段，`queued`（仅入队）/ `driving`（已派发驱动）。
    #[serde(default)]
    pub delivery_state: String,
}

/// Wave-61 C10.2：投递阶段取值（字符串化，避免新增枚举破坏 `SendMessageResult` 的向后兼容）。
pub const DELIVERY_STATE_QUEUED: &str = "queued";
/// Wave-61 C10.2：已派发后台驱动。
pub const DELIVERY_STATE_DRIVING: &str = "driving";
/// 派生阶段未投递任何消息（空 prompt 或引擎未接线）：不得被读成"已经在跑"。
pub const DELIVERY_STATE_NOT_STARTED: &str = "not_started";
/// 派生阶段投递报错（成员已登记，但初始 prompt 没进收件箱）。
pub const DELIVERY_STATE_FAILED: &str = "failed";

/// Wave-61 C10.1：正在被驱动的 target 集合（进程级单飞标记）。
///
/// 语义：同一 target 同时最多只有**一个**驱动循环；后续 `send_message` 只入队，
/// 由在跑的循环消费（detached fan-out 会让同 target 并发多驱动、状态抖动）。
static DRIVING: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// 单飞标记的 RAII 句柄：**任何**退出路径（成功 / 引擎错误 / panic 展开）都会清除标记。
struct DrivingGuard(String);

impl Drop for DrivingGuard {
    fn drop(&mut self) {
        if let Ok(mut lock) = DRIVING.lock() {
            if let Some(set) = lock.as_mut() {
                set.remove(&self.0);
            }
        }
    }
}

/// 尝试占据 target 的单飞标记；返回 `Some(guard)` 表示本次调用负责驱动。
fn begin_driving(target: &str) -> Option<DrivingGuard> {
    let mut lock = DRIVING.lock().ok()?;
    let set = lock.get_or_insert_with(std::collections::HashSet::new);
    if !set.insert(target.to_string()) {
        return None;
    }
    Some(DrivingGuard(target.to_string()))
}

/// Wave-61 C10-A2 配套：被 `interrupt_agent` 标记为中断的 target（一次性消费）。
///
/// 中断语义不能用「status == Inactive」表达：收口（turn 正常结束）同样会把状态置回
/// `Inactive`，两者混用会让驱动循环把「本轮刚结束」误判成「被中断」而提前退出，
/// 导致 turn 在途期间到达的消息滞留（C10-A2 缺陷）。故用独立的一次性标记。
static INTERRUPTED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// 标记某队友被中断（由 `interrupt_agent` 调用）。
fn mark_interrupted(target: &str) {
    if let Ok(mut lock) = INTERRUPTED.lock() {
        lock.get_or_insert_with(std::collections::HashSet::new)
            .insert(target.to_string());
    }
}

/// 消费中断标记：返回 `true` 表示该 target 已被中断且标记 hereby 清除。
fn take_interrupted(target: &str) -> bool {
    INTERRUPTED
        .lock()
        .ok()
        .and_then(|mut lock| lock.as_mut().map(|set| set.remove(target)))
        .unwrap_or(false)
}

/// 该 target 是否已有驱动在跑（观测用）。
pub fn is_driving(target: &str) -> bool {
    DRIVING
        .lock()
        .ok()
        .and_then(|lock| {
            lock.as_ref()
                .map(|set| set.contains(target))
        })
        .unwrap_or(false)
}

/// Wave-61 C10.2：把整批消息按**原顺序**放回收件箱队首（失败补偿，绝不丢消息）。
fn requeue_messages(target: &str, messages: &[QueuedMessage]) {
    let mut inbox_lock = TEAMMATE_INBOXES.lock().unwrap();
    if inbox_lock.is_none() {
        *inbox_lock = Some(HashMap::new());
    }
    let inbox_map = inbox_lock.as_mut().unwrap();
    let deque = inbox_map
        .entry(target.to_string())
        .or_insert_with(VecDeque::new);
    // 逆序 push_front ⇒ 回填后队首顺序与 drain 前一致。
    for message in messages.iter().rev() {
        deque.push_front(message.clone());
    }
    drop(inbox_lock);
    if !messages.is_empty() {
        bump_team_activity();
    }
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

/// 团队活动边沿：成员状态、消息入队/出队、任务流转都会 bump 版本并唤醒 `wait_agent`。
///
/// 这是 `wait_agent` 的唯一事件来源：等待者记录基线版本，任一边沿发生即返回，
/// 超时返回 `timed_out=true`。绝不返回"没有事件却宣称有进展"的假信号。
struct TeamActivity {
    version: Mutex<u64>,
    changed: Condvar,
}

impl TeamActivity {
    fn new() -> Self {
        Self {
            version: Mutex::new(0),
            changed: Condvar::new(),
        }
    }
}

static TEAM_ACTIVITY: OnceLock<TeamActivity> = OnceLock::new();

fn team_activity() -> &'static TeamActivity {
    TEAM_ACTIVITY.get_or_init(TeamActivity::new)
}

/// 发布一次团队边沿（版本 +1 并唤醒全部等待者）。
fn bump_team_activity() {
    bump_activity_instance(team_activity());
}

/// 实例级 bump：产品路径作用于全局实例，单测作用于局部实例（互不干扰、无串扰）。
fn bump_activity_instance(activity: &TeamActivity) {
    let mut version = activity
        .version
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *version = version.wrapping_add(1);
    activity.changed.notify_all();
}

/// 读取当前团队活动版本（`wait_agent` 的基线）。
fn team_activity_version() -> u64 {
    *team_activity()
        .version
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 阻塞等待团队边沿：`baseline` 之后任一次 bump 返回 `true`，超时返回 `false`。
///
/// 抽成独立函数只为可测：`wait_agent` 的对外超时下界是 10s，单测不可能真等那么久，
/// 而「假等待 / 假成功」正是这条边沿契约要根治的缺陷。
fn wait_for_team_activity(baseline: u64, timeout: Duration) -> bool {
    wait_on_team_activity(team_activity(), baseline, timeout)
}

/// 实例级等待（单测用局部实例验证"真等待/真唤醒"，不碰进程级全局状态）。
fn wait_on_team_activity(activity: &TeamActivity, baseline: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut version = activity
        .version
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if *version != baseline {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let (next, _) = activity
            .changed
            .wait_timeout(version, deadline.saturating_duration_since(now))
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        version = next;
    }
}

static TEAM_TASKS: Mutex<Option<HashMap<String, TeamTaskItem>>> = Mutex::new(None);

/// 子代理 turn 超时（NFR 基线：subagent_turn_timeout_ms=120000）。
const SUBAGENT_TURN_TIMEOUT_MS: u64 = 120_000;
/// 名册里的 Lead 伪成员 target（与 `list_agents` / `spawn_teammate` 建队时同源）。
const LEAD_TARGET: &str = "lead";
static TEAMMATES: Mutex<Option<HashMap<String, TeammateMember>>> = Mutex::new(None);
/// 每个 Teammate 对应的优先级双端信箱 (双端队列支持普通排队 push_back 与插队 push_front)
static TEAMMATE_INBOXES: Mutex<Option<HashMap<String, VecDeque<QueuedMessage>>>> = Mutex::new(None);
/// W2：每个 teammate target 派生时解析出的 provider 选择（驱动其 turn 时消费）。
///
/// 与名册/信箱同源：派生即登记，重生（同名 target 再次派生）即覆盖；无条目 ⇒ 旧行为
/// （`run_turn_in`，由引擎自行解析 provider）。
static TEAMMATE_PROVIDERS: Mutex<Option<HashMap<String, ResolvedProviderSelection>>> =
    Mutex::new(None);

/// W2：登记某 teammate 实际生效的 provider 选择（派生阶段调用）。
fn set_teammate_provider(target: &str, selection: ResolvedProviderSelection) {
    let mut lock = TEAMMATE_PROVIDERS.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    lock.as_mut()
        .unwrap()
        .insert(target.to_string(), selection);
}

/// W2：读取某 teammate 的 provider 选择（驱动阶段调用）；无登记 ⇒ `None`（旧行为）。
pub(crate) fn teammate_provider_selection(target: &str) -> Option<ResolvedProviderSelection> {
    let lock = TEAMMATE_PROVIDERS.lock().unwrap();
    lock.as_ref()?.get(target).cloned()
}

/// W3：每个 teammate target **最近一次成功 turn** 产生的子会话 id。
///
/// 纯进程内可见性缓存（不改 session schema、不新建子会话）：`drive_teammate_turn_in` 在队友
/// turn 成功返回后写入；TUI `/agents` 面板据此展示会话尾号，并允许进入该队友子会话。
static TEAMMATE_SESSIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// W3：登记 teammate 最近成功 turn 的子会话 id（`lead` 与空 id 一律忽略）。
///
/// 为什么忽略空 id：`SubagentRunOutcome.session_id` 由引擎给出，空串不构成可恢复会话，
/// 写进去只会让 UI 打开一个"存在但空白"的会话。
fn set_teammate_session(target: &str, session_id: &str) {
    let session_id = session_id.trim();
    if target == LEAD_TARGET || session_id.is_empty() {
        return;
    }
    let mut lock = TEAMMATE_SESSIONS.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    lock.as_mut()
        .unwrap()
        .insert(target.to_string(), session_id.to_string());
}

/// W3：读取 teammate 最近成功 turn 的子会话 id；未成功运行过 ⇒ `None`。
///
/// `lead` 永无独立会话（`None`）—— 它本身就是宿主主会话。
pub fn teammate_session_id(target: &str) -> Option<String> {
    let target = normalize_target(target);
    if target == LEAD_TARGET {
        return None;
    }
    let lock = TEAMMATE_SESSIONS.lock().unwrap();
    lock.as_ref()?.get(&target).cloned()
}

/// D2 修复：由会话 id 反查其归属 teammate target（宿主会话 / 未登记 ⇒ `None`）。
///
/// 供工具层判定 `drain_inbox` 的调用者身份：队友成功 turn 的会话 id 由
/// [`set_teammate_session`] 登记；宿主会话从不登记 ⇒ 调用者回落 `lead`。
pub fn teammate_target_for_session(session_id: &str) -> Option<String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let lock = TEAMMATE_SESSIONS.lock().ok()?;
    let map = lock.as_ref()?;
    map.iter()
        .find(|(_, registered)| registered.as_str() == session_id)
        .map(|(target, _)| target.clone())
}

// ==========================================
// 1a. 会话隔离：teammate / team task 的宿主会话归属
// ==========================================

/// 每个 teammate target 派生于哪个宿主会话（`None` 值不写入）。
///
/// 为什么需要：`TEAMMATES` / `TEAM_TASKS` 是进程全局静态表，同一进程里的多个 TUI 会话
/// 会互相看到对方的队友与任务（跨会话串话）。本表记录归属，TUI 侧据此过滤。
static TEAMMATE_HOST_SESSIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// 每个 team task 由哪个宿主会话创建（`None` 值不写入）。
static TEAM_TASK_HOST_SESSIONS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// 登记 teammate 的宿主会话归属（派生阶段调用；空 host 忽略）。
pub fn set_teammate_host_session(target: &str, host_session_id: &str) {
    let target = normalize_target(target);
    let host_session_id = host_session_id.trim();
    if target.is_empty() || host_session_id.is_empty() || target == LEAD_TARGET {
        return;
    }
    let mut lock = TEAMMATE_HOST_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = lock.get_or_insert_with(HashMap::new);
    map.insert(target, host_session_id.to_string());
}

/// 读取 teammate 的宿主会话；无登记 ⇒ `None`（旧记录 / 库层直调）。
pub fn teammate_host_session(target: &str) -> Option<String> {
    let target = normalize_target(target);
    let lock = TEAMMATE_HOST_SESSIONS.lock().ok()?;
    lock.as_ref()?.get(&target).cloned()
}

/// 把**工具调用的会话 id**解析成「归属的宿主会话 id」。
///
/// - 宿主会话（TUI 主会话 / 桌面会话）⇒ 就是它自己；
/// - 队友子会话（`team-*`，`teammate_target_for_session` 可反查）⇒ 该队友的宿主会话；
/// - 反查不到且非空 ⇒ 自身（无归属的上游，保持可用）。
pub fn host_session_for_caller(session_id: Option<&str>) -> Option<String> {
    let session_id = session_id.map(str::trim).filter(|id| !id.is_empty())?;
    if let Some(target) = teammate_target_for_session(session_id) {
        return teammate_host_session(&target).or_else(|| Some(session_id.to_string()));
    }
    Some(session_id.to_string())
}

/// 登记 team task 的宿主会话归属（创建阶段调用）。
pub fn set_team_task_host_session(task_id: &str, host_session_id: &str) {
    let task_id = task_id.trim();
    let host_session_id = host_session_id.trim();
    if task_id.is_empty() || host_session_id.is_empty() {
        return;
    }
    let mut lock = TEAM_TASK_HOST_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = lock.get_or_insert_with(HashMap::new);
    map.insert(task_id.to_string(), host_session_id.to_string());
}

/// 某宿主会话名下的 teammate 名册（过滤掉伪成员 `lead` 与其他会话的队友）。
///
/// 与 [`list_agents`] 同源（同一 `TEAMMATES` 表与 `is_driving` 判定），只是多一道归属过滤。
pub fn list_agents_for_session(host_session_id: &str) -> Result<ListAgentsResult, String> {
    let host_session_id = host_session_id.trim();
    if host_session_id.is_empty() {
        return list_agents();
    }
    let mut agents = list_agents()?.agents;
    agents.retain(|member| {
        member.target != LEAD_TARGET
            && teammate_host_session(&member.target).as_deref() == Some(host_session_id)
    });
    Ok(ListAgentsResult { agents })
}

/// 某宿主会话名下的 team task 列表（过滤掉其他会话创建的任务）。
///
/// 与 [`team_task_list`] 同源（同一 `TEAM_TASKS` 表与 `deleted` 过滤），只多一道归属过滤。
pub fn team_task_list_for_session(host_session_id: &str) -> Result<TeamTaskListResult, String> {
    let host_session_id = host_session_id.trim();
    if host_session_id.is_empty() {
        return team_task_list();
    }
    let mut list = team_task_list()?;
    let lock = TEAM_TASK_HOST_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(map) = lock.as_ref() {
        list.tasks.retain(|task| {
            map.get(&task.task_id).map(String::as_str) == Some(host_session_id)
        });
    } else {
        // 没有任何归属登记：严格隔离 —— 不把无主任务串给任意会话。
        list.tasks.clear();
    }
    Ok(list)
}

/// W3 trace：每个 teammate target 派发时捕获的**宿主 turn traceparent**。
///
/// 用途：`drive_teammate_turn_in`（在后台线程/另一 turn 上执行）已经离开宿主 turn 的
/// 活动上下文，必须靠这份快照把宿主 trace 延续到队友 turn（否则队友 trace 会另起 root）。
static TEAMMATE_TRACEPARENTS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// W3 trace：登记 target 的宿主 traceparent（`lead`/空值/非法值忽略 —— fail-closed 保持 root）。
fn set_teammate_traceparent(target: &str, traceparent: Option<&str>) {
    let Some(traceparent) = traceparent.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if target == LEAD_TARGET {
        return;
    }
    let mut lock = TEAMMATE_TRACEPARENTS.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    lock.as_mut()
        .unwrap()
        .insert(target.to_string(), traceparent.to_string());
}

/// W3 trace：读取 target 的宿主 traceparent 快照；无登记 ⇒ `None`（队友 turn 自起 root）。
pub(crate) fn teammate_traceparent(target: &str) -> Option<String> {
    if target == LEAD_TARGET {
        return None;
    }
    let lock = TEAMMATE_TRACEPARENTS.lock().unwrap();
    lock.as_ref()?.get(target).cloned()
}

// ==========================================
// 1b. Subagent 运行注册表（TUI Agents 页签可见性）
// ==========================================

/// 子智能体一次运行的终态（`Running` 期间对用户可见为「● 运行中」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunStatus {
    Running,
    Completed,
    Failed,
}

impl SubagentRunStatus {
    /// 是否仍在运行（TUI 状态条与页签计数的唯一判据）。
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// 是否「需要人看一眼」（只有失败算；完成与运行中都不算）。
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// 一次子智能体运行的**可见性记录**（进程内，无正文、无输出）。
///
/// 为什么需要它：`subagent_in_workspace_with_context` 预分配了 `subagent-<uuid>` 会话 id，
/// 但此前没有登记到任何花名册 —— TUI 的 Agents 页签与状态条因此完全看不到子智能体
/// （见 ADR `2026-11-25-tui-drawer-ux-and-subagent-visibility`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentRunRecord {
    /// 子会话 id（`subagent-<uuid>`；与宿主 `SessionStore` 中的真实历史同一个 id）。
    pub session_id: String,
    /// 面向用户的中文描述（`subagent` 的 `description` 参数）。
    pub description: String,
    pub status: SubagentRunStatus,
    /// 发起子代理的宿主会话 id；`None` = 无会话归属（内部直调，不串到任何 TUI 会话）。
    pub parent_session_id: Option<String>,
    /// 开始时刻（Unix 毫秒）。
    pub started_at_ms: u64,
    /// 失败原因（仅 `Failed` 有值）。
    pub error: Option<String>,
}

/// 子智能体运行记录的保留上限：超出后淘汰**最早的非运行中**记录。
///
/// 运行中记录永不淘汰（否则「在跑的子代理」会从界面上凭空消失）。
const MAX_SUBAGENT_RUNS: usize = 64;

static SUBAGENT_RUNS: Mutex<Option<VecDeque<SubagentRunRecord>>> = Mutex::new(None);

/// 登记一次「开始运行」的子代理（入口预分配 id 之后立即调用）。
pub fn register_subagent_run(
    session_id: &str,
    description: &str,
    parent_session_id: Option<&str>,
) {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return;
    }
    let record = SubagentRunRecord {
        session_id: session_id.to_string(),
        description: description.trim().to_string(),
        status: SubagentRunStatus::Running,
        parent_session_id: parent_session_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        started_at_ms: now_unix_ms(),
        error: None,
    };
    let mut lock = SUBAGENT_RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let runs = lock.get_or_insert_with(VecDeque::new);
    runs.push_back(record);
    // 淘汰：只在超过上限时移除**最早的终态**记录；全是运行中则保留（宁可多留几行）。
    while runs.len() > MAX_SUBAGENT_RUNS {
        match runs.iter().position(|run| !run.status.is_running()) {
            Some(index) => {
                runs.remove(index);
            }
            None => break,
        }
    }
}

/// 更新一次子代理运行的终态（`Completed` / `Failed`）；未知 id 静默忽略。
pub fn finish_subagent_run(
    session_id: &str,
    status: SubagentRunStatus,
    error: Option<&str>,
) {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return;
    }
    let mut lock = SUBAGENT_RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(runs) = lock.as_mut() else {
        return;
    };
    if let Some(record) = runs.iter_mut().find(|run| run.session_id == session_id) {
        record.status = status;
        record.error = error.map(str::to_string);
    }
}

/// 把运行记录的会话 id 从预分配值迁到引擎回传的真实 id。
///
/// 为什么需要：Stub / 自定义引擎可能忽略预分配 id 而回传自己的会话 id；不迁移的话
/// 页签里会留一条**永不结束**的「运行中」记录（用户永远看不到它完成）。
pub fn rebind_subagent_run(from_session_id: &str, to_session_id: &str) {
    let from = from_session_id.trim();
    let to = to_session_id.trim();
    if from.is_empty() || to.is_empty() || from == to {
        return;
    }
    let mut lock = SUBAGENT_RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(runs) = lock.as_mut() else {
        return;
    };
    if let Some(record) = runs.iter_mut().find(|run| run.session_id == from) {
        record.session_id = to.to_string();
    }
}

/// 某宿主会话名下的子代理运行记录（按开始时刻升序，运行中排在后面也无妨——由调用方排序）。
///
/// 无 `parent_session_id` 的记录**不**返回给任何具体会话：内部直调的子代理不得串到
/// TUI 会话的页签里（与 ACP 委派/作业归属同一条纪律）。
pub fn list_subagent_runs_for_session(parent_session_id: &str) -> Vec<SubagentRunRecord> {
    let target = parent_session_id.trim();
    if target.is_empty() {
        return Vec::new();
    }
    let lock = SUBAGENT_RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    lock.as_ref()
        .map(|runs| {
            runs.iter()
                .filter(|run| run.parent_session_id.as_deref() == Some(target))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

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
    subagent_in_workspace_with_provider(args, workspace_root, None)
}

/// W2：带宿主 provider 继承/显式覆盖的子代理入口（兼容包装）。
///
/// 不携带会话归属上下文：workspace id / 父会话 id 缺失，由引擎回落既有解析链。
/// 工具层入口应改用 [`subagent_in_workspace_with_context`]。
pub fn subagent_in_workspace_with_provider(
    args: SubagentArgs,
    workspace_root: Option<&std::path::Path>,
    host_selection: Option<&ResolvedProviderSelection>,
) -> Result<SubagentResult, String> {
    subagent_in_workspace_with_context(args, workspace_root, host_selection, None, None)
}

/// 带**完整会话归属**的子代理入口（实现体）。
///
/// 在 [`subagent_in_workspace_with_provider`] 之上补齐三件事：
/// - `session_id` 由本层预分配并下传引擎，`SubagentResult.subagent_id` 与真实子会话同一个 id；
/// - `parent_session_id` / `workspace_id` 让引擎继承父会话的 workspace 绑定；
/// - `args.fork_context` 原样进入请求，由引擎决定是否 seed 父历史。
///
/// 许可门 / 前后台语义 / provider 解析与旧入口逐字一致。
pub fn subagent_in_workspace_with_context(
    args: SubagentArgs,
    workspace_root: Option<&std::path::Path>,
    host_selection: Option<&ResolvedProviderSelection>,
    parent_session_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<SubagentResult, String> {
    // W2 失败前置：**先解析**，解析失败立即返回 —— 绝不先起后台线程再异步失败。
    let selection = resolve_optional_teammate_provider_selection(
        host_selection,
        args.provider_id.as_deref(),
        args.model_id.as_deref(),
    )?;

    // W3 trace：**派发时**捕获宿主当前 turn 的 traceparent（同步与后台两条路径都继承同一 trace）。
    // P3：工具层若已预创建 `tool.call` span，这里拿到的是该 span —— 子 turn 因此成为工具的子节点。
    let parent_traceparent = crate::agent::otel::current_traceparent();

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
    // P2：会话 id 只在这里生成一次，引擎必须复用；后台路径无 outcome 时也返回同一个 id。
    let subagent_id = format!("subagent-{}", Uuid::new_v4());
    let is_bg = args.run_in_background.unwrap_or(false);
    let fork_context = args.fork_context.unwrap_or(false);
    // TUI Agents 页签的可见性来源：预分配 id 之后立即登记「运行中」，两条路径收尾时更新终态。
    let run_description = args.description.clone();
    register_subagent_run(&subagent_id, &run_description, parent_session_id);

    if is_bg {
        let bg_id = subagent_id.clone();
        let bg_root = workspace_root.map(std::path::Path::to_path_buf);
        let bg_prompt = args.prompt;
        let bg_parent_session_id = parent_session_id.map(str::to_string);
        let bg_workspace_id = workspace_id.map(str::to_string);
        std::thread::spawn(move || {
            // 后台 turn 全程占许可（仅非自持引擎会携带 Some）。
            let _gate = gate;
            let request = crate::agent::subagent_runtime::SubagentRunRequest {
                prompt: &bg_prompt,
                timeout_ms: SUBAGENT_TURN_TIMEOUT_MS,
                workspace_root: bg_root.as_deref(),
                provider_selection: selection.as_ref(),
                parent_traceparent: parent_traceparent.as_deref(),
                session_id: Some(&bg_id),
                parent_session_id: bg_parent_session_id.as_deref(),
                workspace_id: bg_workspace_id.as_deref(),
                fork_context,
            };
            match engine.run_turn_request(&request) {
                Ok(_) => finish_subagent_run(&bg_id, SubagentRunStatus::Completed, None),
                Err(err) => {
                    eprintln!("[subagent] background turn failed ({}): {}", bg_id, err);
                    finish_subagent_run(&bg_id, SubagentRunStatus::Failed, Some(err.as_str()));
                }
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
    let request = crate::agent::subagent_runtime::SubagentRunRequest {
        prompt: &args.prompt,
        timeout_ms: SUBAGENT_TURN_TIMEOUT_MS,
        workspace_root,
        provider_selection: selection.as_ref(),
        parent_traceparent: parent_traceparent.as_deref(),
        session_id: Some(&subagent_id),
        parent_session_id,
        workspace_id,
        fork_context,
    };
    match engine.run_turn_request(&request) {
        Ok(outcome) => {
            // P2：引擎回传真实会话 id 时以它为准（Stub 引擎可能忽略预分配 id）；
            // 为空才回落预分配值，保证返回值永远可检索。
            let resolved_id = if outcome.session_id.trim().is_empty() {
                subagent_id.clone()
            } else {
                outcome.session_id
            };
            // 引擎换 id 时把运行记录一并迁移（否则页签里会留一条永不结束的「运行中」）。
            rebind_subagent_run(&subagent_id, &resolved_id);
            finish_subagent_run(&resolved_id, SubagentRunStatus::Completed, None);
            Ok(SubagentResult {
                subagent_id: resolved_id,
                status: "completed".to_string(),
                output: Some(outcome.final_text),
            })
        }
        Err(err) => {
            finish_subagent_run(&subagent_id, SubagentRunStatus::Failed, Some(err.as_str()));
            Err(err)
        }
    }
}

/// W2：按宿主继承 + 显式覆盖解析子代理/队友实际使用的 provider 选择。
///
/// 语义（task-8 冻结契约）：
/// 1. 无显式 `provider_id`：`host` 必须存在，否则 `Err("missing_host_provider_selection")`；
///    存在则 clone 宿主（`model_id` 若给出再覆盖 model）。
/// 2. 显式 `provider_id` 且等于 `host.requested_name` / `host.provider_name`：用宿主 base
///    （同样只按 `model_id` 覆盖 model）。
/// 3. 显式 `provider_id` 指向其它 provider：在本机 provider 注册表
///    （[`ProviderRegistryStore::new`] 的 `load_view().providers`）里按 id 或 name 查找；
///    找不到 ⇒ `Err("unknown_provider")`；找到 ⇒ 用该 provider 的 id 走
///    [`ProviderRegistryStore::resolve_selection`]（`model_id` 一并作为入参）。
/// 4. 最后统一应用 `model_id` 覆盖（含宿主分支），保证“只换模型不换 provider”。
pub fn resolve_teammate_provider_selection(
    host: Option<&ResolvedProviderSelection>,
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<ResolvedProviderSelection, String> {
    let requested_provider = provider_id.map(str::trim).filter(|value| !value.is_empty());
    let requested_model = model_id.map(str::trim).filter(|value| !value.is_empty());

    let mut selection = match (host, requested_provider) {
        (_, None) => host.cloned().ok_or_else(|| {
            "missing_host_provider_selection: 未注入宿主 provider 选择，且未显式指定 provider"
                .to_string()
        })?,
        (Some(host), Some(requested))
            if host.requested_name == requested || host.provider_name == requested =>
        {
            host.clone()
        }
        (_, Some(requested)) => {
            let store = ProviderRegistryStore::new();
            let view = store.load_view();
            let provider_id = view
                .providers
                .iter()
                .find(|provider| provider.id == requested || provider.name == requested)
                .map(|provider| provider.id.clone())
                .ok_or_else(|| {
                    format!("unknown_provider: provider `{requested}` 不在本机 provider 注册表中")
                })?;
            store.resolve_selection(Some(provider_id.as_str()), requested_model)
        }
    };

    if let Some(model) = requested_model {
        selection.model = model.to_string();
    }
    Ok(selection)
}

/// W2：`host_selection=None` 且无显式覆盖 ⇒ legacy（`None`，引擎自行解析）；
/// 其余情况必须解析成功（失败即 `Err`，调用方据此**前置失败**）。
fn resolve_optional_teammate_provider_selection(
    host_selection: Option<&ResolvedProviderSelection>,
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<Option<ResolvedProviderSelection>, String> {
    let provider_id = provider_id.map(str::trim).filter(|value| !value.is_empty());
    let model_id = model_id.map(str::trim).filter(|value| !value.is_empty());
    if host_selection.is_none() && provider_id.is_none() && model_id.is_none() {
        return Ok(None);
    }
    resolve_teammate_provider_selection(host_selection, provider_id, model_id).map(Some)
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
    let (mut result, _target, _prompt) = register_teammate(args, &TeamCaller::lead())?;
    // 纯登记不做任何投递：显式声明「初始 prompt 未派发」，不靠默认值猜。
    result.delivery_state = DELIVERY_STATE_NOT_STARTED.to_string();
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
    spawn_teammate_in_workspace_with_provider(args, workspace_root, None)
}

/// W2：带宿主 provider 继承/显式覆盖的队友生成入口（实现体）。
///
/// 与 [`spawn_teammate_in_workspace`] 的唯一差别是**先解析 provider 选择**：
/// - 解析失败 ⇒ 直接 `Err`，**不登记成员、不投递、不起后台驱动**（失败前置）；
/// - 解析成功 ⇒ 登记成员后把选择写入 [`TEAMMATE_PROVIDERS`]，后续
///   [`drive_teammate_turn_in`] 驱动该队友时按同一选择建栈；
/// - `host_selection=None` 且 `args` 无覆盖 ⇒ legacy（不写选择，引擎自行解析）。
pub fn spawn_teammate_in_workspace_with_provider(
    args: SpawnTeammateArgs,
    workspace_root: Option<&std::path::Path>,
    host_selection: Option<&ResolvedProviderSelection>,
) -> Result<SpawnTeammateResult, String> {
    spawn_teammate_in_workspace_with_provider_as(
        args,
        workspace_root,
        host_selection,
        &TeamCaller::lead(),
    )
}

/// 以显式调用者身份派生队友（**工具层必须走本入口**：只有 Team Lead 可派生）。
///
/// 为什么需要 `_as`：旧入口按 Lead 代理，工具层若沿用它，队友调用 `spawn_teammate`
/// 也会被当作 Lead 放行 —— 授权形同虚设（审计 P0-3 的同类问题）。
pub fn spawn_teammate_in_workspace_with_provider_as(
    args: SpawnTeammateArgs,
    workspace_root: Option<&std::path::Path>,
    host_selection: Option<&ResolvedProviderSelection>,
    caller: &TeamCaller,
) -> Result<SpawnTeammateResult, String> {
    // 1) 失败前置：先解析（无 host 且无显式覆盖时允许 None，保持 legacy 语义）。
    let selection = resolve_optional_teammate_provider_selection(
        host_selection,
        args.provider_id.as_deref(),
        args.model_id.as_deref(),
    )?;
    // W3 trace：**派发时**捕获宿主当前 turn 的 traceparent（此刻仍在宿主 turn 的活动上下文内）。
    let parent_traceparent = crate::agent::otel::current_traceparent();

    let auto_drive = crate::agent::subagent_runtime::subagent_engine().is_some();
    let (result, target, initial_prompt) = register_teammate(args, caller)?;

    // 2) 登记成功后持久化该 target 的 provider 选择（驱动阶段唯一真值源）。
    if let Some(selection) = selection {
        set_teammate_provider(&target, selection);
    }
    // W3 trace：同名 target 再派生时覆盖（与 provider/会话登记同源语义）。
    if let Some(parent_traceparent) = parent_traceparent {
        set_teammate_traceparent(&target, Some(parent_traceparent.as_str()));
    }

    // Wave-61 C7：引擎在位时把初始 prompt 投递并驱动（复用 send_message 的入队 + 后台
    // drive 通路，不新造存储；无引擎时保持纯登记，行为与既有 `spawn_teammate` 一致）。
    //
    // B1795535505312：投递结果（`queued` / `driving`）必须回传模型，否则「派生成功」会被
    // 读成「已经开始干活」——这正是原事故里 Lead 误判队友在跑的直接成因。
    let mut result = result;
    if auto_drive && !initial_prompt.trim().is_empty() {
        match send_message_in_workspace(
            SendMessageArgs {
                target: target.clone(),
                message: initial_prompt,
                mode: Some(MessageDeliveryMode::Queue),
            },
            workspace_root,
        ) {
            Ok(sent) => result.delivery_state = sent.delivery_state,
            Err(err) => {
                // 成员已登记但初始 prompt 没进收件箱：如实标记，不伪造成已派发。
                result.delivery_state = DELIVERY_STATE_FAILED.to_string();
                set_teammate_error(&target, Some(err));
            }
        }
    }

    // 回读运行态：`result.member` 是登记快照（恒 `active`），模型据此无法判断驱动是否真的
    // 起来了。这里在返回前用名册真值覆盖一次，`status` / `is_driving` / `last_error` 同源。
    if let Some(snapshot) = teammate_snapshot(&target) {
        result.member = snapshot;
    }

    Ok(result)
}

/// D1 修复：teammate target 的**唯一归一化规则**（trim + 小写 + 空格转连字符）。
///
/// 登记（`register_teammate`）与所有查找（send / interrupt / drain）必须逐字共用本函数：
/// 此前只有登记侧归一化，`list_agents` 又同时暴露 `name="Lead"` 与 `target="lead"`，
/// 模型拿 name 当 target 发送即 `Teammate target not found`（2026-11-07 trace 实证）。
pub fn normalize_target(raw: &str) -> String {
    raw.trim().to_lowercase().replace(' ', "-")
}

/// 成员上限：防止递归派生把进程拖死。
const MAX_TEAM_MEMBERS: usize = 16;

/// 登记队友（成员表 + 独立收件箱）并返回 (结果, target, 初始 prompt)。
///
/// 仅 Lead 可派生；同名成员绝不静默覆盖；成员数量有硬上限。
fn register_teammate(
    args: SpawnTeammateArgs,
    caller: &TeamCaller,
) -> Result<(SpawnTeammateResult, String, String), String> {
    if !caller.is_lead {
        return Err(format!(
            "spawn_teammate requires the Team Lead; caller `{}` is a teammate",
            caller.name
        ));
    }
    let initial_prompt = args.prompt.clone();
    let target = normalize_target(&args.name);
    if target.is_empty() {
        return Err("teammate name must not be empty".to_string());
    }
    if target == LEAD_TARGET {
        return Err("teammate name `lead` is reserved".to_string());
    }
    let member = TeammateMember {
        target: target.clone(),
        name: args.name,
        role: args.role.unwrap_or_else(|| "teammate".to_string()),
        status: TeammateStatus::Active,
        description: args.description,
        is_driving: false,
        last_error: None,
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
                is_driving: false,
                last_error: None,
                },
            );
            *lock = Some(map);
        }
        let map = lock.as_mut().unwrap();
        if map.contains_key(&target) {
            return Err(format!(
                "teammate `{target}` already exists; names are not silently replaced"
            ));
        }
        let teammate_count = map
            .keys()
            .filter(|key| key.as_str() != LEAD_TARGET)
            .count();
        if teammate_count >= MAX_TEAM_MEMBERS {
            return Err(format!(
                "team member limit {MAX_TEAM_MEMBERS} reached; interrupt an existing teammate first"
            ));
        }
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

    // 预分配并登记该队友的固定会话 id：
    // - 驱动每轮复用同一会话 ⇒ 队友跨轮保留上下文（不再是每轮失忆的新会话）；
    // - 首轮 turn 内部即可由 session_id 反查成员身份 ⇒ 队友调用团队工具时不会被当成 Lead 放行。
    let teammate_session = format!(
        "team-{}-{}",
        target,
        &Uuid::new_v4().simple().to_string()[..8]
    );
    set_teammate_session(&target, &teammate_session);

    bump_team_activity();
    Ok((
        SpawnTeammateResult {
            member,
            delivery_state: DELIVERY_STATE_NOT_STARTED.to_string(),
        },
        target,
        initial_prompt,
    ))
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
            is_driving: false,
            last_error: None,
            },
        );
        *lock = Some(map);
    }
    let mut agents: Vec<TeammateMember> = lock.as_ref().unwrap().values().cloned().collect();
    // 锁已在此作用域结束后释放（下面 `drop`），避免与 `DRIVING` 形成嵌套。
    drop(lock);
    agents.sort_by(|left, right| left.target.cmp(&right.target));
    for member in &mut agents {
        member.is_driving = is_driving(&member.target);
    }
    Ok(ListAgentsResult { agents })
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
    let (mut result, drive_target) = enqueue_message(args)?;

    if drive_target != LEAD_TARGET && crate::agent::subagent_runtime::subagent_engine().is_some() {
        // Wave-61 C10.1：**每 target 单飞**。已有驱动在跑时只入队并返回，由在跑的循环消费；
        // 绝不为同一 target 派生第二个 detached 线程（会造成并发多驱动与状态抖动）。
        if let Some(guard) = begin_driving(&drive_target) {
            // Wave-61 C10-A2：新驱动启动前清掉**残留**的中断标记，防止历史标记把本次驱动
            // 在检查点 #1 直接打断（空转地把消息留在收件箱、状态卡 `Running`）。
            let _ = take_interrupted(&drive_target);
            let owner = workspace_root.map(std::path::Path::to_path_buf);
            // C10.3：进入驱动前先置 `Running`（`Active` 由入队/派生阶段给出，收口回 `Inactive`）。
            set_teammate_status(&drive_target, TeammateStatus::Running);
            std::thread::spawn(move || {
                // guard 随线程存活/Drop ⇒ 所有退出路径（含 panic 展开）都清除单飞标记。
                let _guard = guard;
                if let Err(err) = drive_loop_in(&drive_target, owner.as_deref()) {
                    eprintln!("[team] teammate drive failed ({}): {}", drive_target, err);
                }
            });
            result.delivery_state = DELIVERY_STATE_DRIVING.to_string();
        }
    }

    Ok(result)
}

/// Wave-61 C10.1：驱动循环（由后台线程执行，配合 [`DrivingGuard`] 单飞）。
///
/// `loop { drain → 空则退出 → run_turn → 投递回执 }`，直到收件箱为空才结束；失败由
/// [`drive_teammate_turn_in`] 做「放回队首 + 置 Inactive + 投递错误回执」补偿后返回 Err，
/// 本循环随即停止（把剩下的工作留给下一次 `send_message` 触发）。
fn drive_loop_in(
    target: &str,
    workspace_root: Option<&std::path::Path>,
) -> Result<(), String> {
    // Wave-61 C10-A2：以「收件箱为空」为**唯一**正常退出条件。
    // - 成功：处理完一批后继续 drain —— turn 在途期间新到的消息（因 `DRIVING` 单飞不会
    //   另起驱动）由此被同一循环消费，绝不滞留；
    // - 失败：`drive_teammate_turn_in` 已按 C10.2 把整批原序放回队首并置 `Inactive`，
    //   这里立即 `?` 退出（避免 `subagent_busy` / `prompt_too_large` 这类确定性失败自旋），
    //   消息留在收件箱等待下一次 `send_message` 或外部驱动；
    // - 中断：`take_interrupted` 命中时单批驱动返回 `Ok(None)`，循环随之结束；
    // - `DRIVING` 标记由调用方持有的 `DrivingGuard` 清除，覆盖成功/失败/panic 全部路径。
    loop {
        match drive_teammate_turn_in(target, workspace_root)? {
            Some(_reply) => continue,
            None => return Ok(()),
        }
    }
}

/// 纯入队 seam：只把消息写进目标收件箱，**不触发**任何驱动/状态机副作用。
///
/// 这是队列层入口：`send_message_in_workspace`（以及库层 `send_message`）在调用本函数
/// 拿到结果后，才按需把 `Running` 置位并派生后台 drive；因此：
/// - 需要「只验证排队 / 插队顺序」的调用方（含契约测试）用本 seam，观测面不含驱动竞态；
/// - 需要「入队即真实驱动」的调用方用 `send_message_in_workspace`。
/// 返回 `(入队结果, 目标 target)`，便于驱动方复用同一 target 文本。
pub fn enqueue_message(args: SendMessageArgs) -> Result<(SendMessageResult, String), String> {
    // D1 修复：查找前按登记侧同一规则归一化（"Lead"/"Probe Agent" 等 name 写法亦可命中）。
    let mut args = args;
    args.target = normalize_target(&args.target);
    if args.message.len() > MAX_TEAM_MESSAGE_BYTES {
        return Err(format!(
            "team message exceeds {MAX_TEAM_MESSAGE_BYTES} bytes"
        ));
    }
    let mode = args.mode.unwrap_or(MessageDeliveryMode::Queue);
    let msg_id = format!("msg-{}", Uuid::new_v4());

    // 1. 检验目标并更新状态，立即释放 TEAMMATES 锁
    {
        let mut lock = TEAMMATES.lock().unwrap();
        let map = lock.as_mut().ok_or_else(|| "No active team".to_string())?;
        let member = map
            .get_mut(&args.target)
            .ok_or_else(|| format!("Teammate target not found: {}", args.target))?;

        // 新消息一律把「无 turn 在跑」的成员拉回可驱动状态：`Inactive`（上一轮成功）、
        // `Failed`（上一轮报错）、`Interrupted`（被中断）都允许重试；只有 `Running`
        // 保持不动（在途 turn 由当前驱动的 drain 边界消费新消息）。
        match mode {
            MessageDeliveryMode::Steer => {
                if member.status != TeammateStatus::Running
                    && member.status != TeammateStatus::Provisioning
                {
                    member.status = TeammateStatus::Running;
                }
            }
            MessageDeliveryMode::Queue => {
                if member.status == TeammateStatus::Inactive
                    || member.status == TeammateStatus::Failed
                    || member.status == TeammateStatus::Interrupted
                {
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

    bump_team_activity();

    Ok((
        SendMessageResult {
            message_id: msg_id,
            delivered: true,
            mode,
            pending_inbox_len: len,
            processed: false,
            delivery_state: DELIVERY_STATE_QUEUED.to_string(),
        },
        args.target,
    ))
}

/// 队友状态写入（独立作用域持锁，绝不与收件箱锁嵌套，避免锁序反转）。
fn set_teammate_status(target: &str, status: TeammateStatus) {
    update_teammate(target, |member| {
        if member.status == status {
            return false;
        }
        member.status = status;
        true
    });
}

/// 记录（或清空）上一轮驱动的失败文本；与状态写入同源、同一把锁。
fn set_teammate_error(target: &str, error: Option<String>) {
    update_teammate(target, |member| {
        if member.last_error == error {
            return false;
        }
        member.last_error = error;
        true
    });
}

/// 单点成员变更入口：`mutate` 返回 `true` 才算「有变化」，并统一 bump 团队活动版本
/// （`wait_agent` 的边沿等待依赖它）。
fn update_teammate(target: &str, mutate: impl FnOnce(&mut TeammateMember) -> bool) {
    let mut changed = false;
    if let Ok(mut lock) = TEAMMATES.lock() {
        if let Some(map) = lock.as_mut() {
            if let Some(member) = map.get_mut(target) {
                changed = mutate(member);
            }
        }
    }
    if changed {
        bump_team_activity();
    }
}

/// 成员读时快照：克隆成员表里的登记项，并按 `DRIVING` 单飞标记填充 `is_driving`。
///
/// 锁序：先取 `TEAMMATES` 克隆后**释放**，再取 `DRIVING` —— 与
/// `interrupt_agent_as`「先 is_driving 再 TEAMMATES」的既有顺序一起保证永不嵌套。
fn teammate_snapshot(target: &str) -> Option<TeammateMember> {
    let mut member = TEAMMATES
        .lock()
        .ok()
        .and_then(|lock| lock.as_ref().and_then(|map| map.get(target).cloned()))?;
    member.is_driving = is_driving(target);
    Some(member)
}

/// 打断正在运行的智能体（保留其信箱队列）
pub fn interrupt_agent(args: InterruptAgentArgs) -> Result<InterruptAgentResult, String> {
    interrupt_agent_as(args, &TeamCaller::lead())
}

/// 以显式调用者身份中断队友：**只有 Team Lead 可中断**（队友不得中断 Lead 或兄弟成员）。
pub fn interrupt_agent_as(
    args: InterruptAgentArgs,
    caller: &TeamCaller,
) -> Result<InterruptAgentResult, String> {
    if !caller.is_lead {
        return Err(format!(
            "interrupt_agent requires the Team Lead; caller `{}` is a teammate",
            caller.name
        ));
    }
    // D1 修复：与登记侧同一归一化（is_driving / 成员表 / 中断标记三处共用同一 target）。
    let mut args = args;
    args.target = normalize_target(&args.target);
    // Wave-61 C10-A2 回归修正：先读「是否真有驱动在跑」。
    // - 在跑：置一次性中断标记，驱动循环会在下一次 drain 边界前停下（C7.4）；
    // - 空闲：**只**翻状态位，不写标记 —— 否则残留标记会被下一次 `send_message` 派生的
    //   驱动在检查点 #1 误消费，导致消息滞留、成员永久卡 `Running`。
    // 说明：`is_driving` 使用独立的 `DRIVING` 锁，且在取 `TEAMMATES` 锁**之前**调用，
    // 避免与驱动线程形成锁嵌套（无锁序反转）。
    let inflight_drive = is_driving(&args.target);

    let (prev, new_status) = {
        let mut lock = TEAMMATES.lock().unwrap();
        let map = lock.as_mut().ok_or_else(|| "No active team".to_string())?;
        let member = map
            .get_mut(&args.target)
            .ok_or_else(|| format!("Teammate target not found: {}", args.target))?;
        let prev = member.status.clone();
        // B1795535505312：中断是独立终态，不得与「正常收口」共用 `Inactive`。
        member.status = TeammateStatus::Interrupted;
        (prev, TeammateStatus::Interrupted)
    };

    if inflight_drive {
        mark_interrupted(&args.target);
    }

    Ok(InterruptAgentResult {
        success: true,
        previous_status: prev,
        new_status,
    })
}

/// 读取并消费目标智能体收件箱的消息
pub fn drain_inbox(args: ReadInboxArgs) -> Result<ReadInboxResult, String> {
    // D1 修复：与登记侧同一归一化（成员判定与信箱键共用同一 target）。
    let mut args = args;
    args.target = normalize_target(&args.target);
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
    drop(inbox_lock);
    if !messages.is_empty() {
        bump_team_activity();
    }
    Ok(ReadInboxResult {
        target: args.target,
        messages,
    })
}

/// Wave-61 C10-A2 配套（additive，只读）：某队友收件箱里**待消费**的消息条数。
///
/// 供外部观察者（测试 / UI / 观测）等待"收件箱已空"而**不**像 [`drain_inbox`] 那样
/// 破坏性消费消息 —— 后者会把驱动循环应消费的消息抢走（C7d 的竞态根因）。
pub fn inbox_pending_len(target: &str) -> usize {
    TEAMMATE_INBOXES
        .lock()
        .ok()
        .and_then(|lock| {
            lock.as_ref()
                .and_then(|map| map.get(target).map(|deque| deque.len()))
        })
        .unwrap_or(0)
}

/// 团队任务看板的容量与文本预算（拒绝无限增长与超长 prompt 注入）。
const MAX_TEAM_TASKS: usize = 256;
const MAX_TASK_SUBJECT_CHARS: usize = 200;
const MAX_TASK_DESCRIPTION_CHARS: usize = 16_384;
/// 单条团队消息的字节上限：避免一次消息把队友 prompt 撑爆。
const MAX_TEAM_MESSAGE_BYTES: usize = 64 * 1024;

/// 任务变更的调用者身份：`lead` 或具名 teammate。
#[derive(Debug, Clone)]
pub struct TeamCaller {
    pub name: String,
    pub is_lead: bool,
}

impl TeamCaller {
    /// 用户界面 / Tauri 命令 / 旧库调用：一律按 Lead 代理处理。
    pub fn lead() -> Self {
        Self {
            name: LEAD_TARGET.to_string(),
            is_lead: true,
        }
    }

    /// 由工具调用会话反查成员身份；未登记 ⇒ Lead（与 `drain_inbox` 的判定同源）。
    pub fn from_session(session_id: Option<&str>) -> Self {
        match session_id.and_then(teammate_target_for_session) {
            Some(target) => Self {
                name: target,
                is_lead: false,
            },
            None => Self::lead(),
        }
    }
}

/// 任务文本校验：非空 + 长度预算。
fn validate_task_text(subject: &str, description: &str) -> Result<(), String> {
    if subject.trim().is_empty() {
        return Err("task subject must not be empty".to_string());
    }
    if subject.chars().count() > MAX_TASK_SUBJECT_CHARS {
        return Err(format!(
            "task subject exceeds {MAX_TASK_SUBJECT_CHARS} characters"
        ));
    }
    if description.chars().count() > MAX_TASK_DESCRIPTION_CHARS {
        return Err(format!(
            "task description exceeds {MAX_TASK_DESCRIPTION_CHARS} characters"
        ));
    }
    Ok(())
}

/// 写域归一化：拒绝绝对路径与 `..`，去重保序；重叠仅作运行期诊断，不是锁。
fn normalize_write_scopes(scopes: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized: Vec<String> = Vec::new();
    for scope in scopes {
        let scope = scope.trim();
        if scope.is_empty() {
            return Err("write scope must not be empty".to_string());
        }
        if scope.starts_with('/') || scope.starts_with('\\') || scope.contains("..") {
            return Err(format!(
                "write scope `{scope}` must be workspace-relative without `..`"
            ));
        }
        if !normalized.iter().any(|existing| existing == scope) {
            normalized.push(scope.to_string());
        }
    }
    Ok(normalized)
}

/// 依赖校验：不得自引用、不得重复、不得引用不存在的任务。
fn validate_dependencies(
    map: &HashMap<String, TeamTaskItem>,
    task_id: &str,
    deps: &[String],
) -> Result<(), String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for dep in deps {
        if dep == task_id {
            return Err("a task cannot block itself".to_string());
        }
        if !seen.insert(dep.as_str()) {
            return Err(format!("duplicate dependency `{dep}`"));
        }
        if !map.contains_key(dep) {
            return Err(format!("dependency `{dep}` does not exist"));
        }
    }
    Ok(())
}

/// 依赖环检测：从候选任务出发，沿 blocked_by 深度优先，回到自身即为环。
fn has_dependency_cycle(map: &HashMap<String, TeamTaskItem>, task_id: &str, deps: &[String]) -> bool {
    fn visit(
        map: &HashMap<String, TeamTaskItem>,
        root: &str,
        current: &str,
        root_deps: &[String],
        visited: &mut std::collections::HashSet<String>,
    ) -> bool {
        let deps: &[String] = if current == root {
            root_deps
        } else {
            match map.get(current) {
                Some(task) => &task.blocked_by,
                None => return false,
            }
        };
        for dep in deps {
            if dep == root {
                return true;
            }
            if !visited.insert(dep.clone()) {
                continue;
            }
            if visit(map, root, dep, root_deps, visited) {
                return true;
            }
        }
        false
    }

    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visit(map, task_id, task_id, deps, &mut visited)
}

/// 就绪判定：全部依赖已完成（缺失依赖视为未就绪，绝不默认放行）。
fn task_ready(map: &HashMap<String, TeamTaskItem>, task: &TeamTaskItem) -> bool {
    task.blocked_by.iter().all(|dep| {
        map.get(dep)
            .map(|blocker| blocker.status == "completed")
            .unwrap_or(false)
    })
}

/// 授权：任务当前 owner 或 Team Lead 才能变更。
fn authorize_task(caller: &TeamCaller, task: &TeamTaskItem) -> Result<(), String> {
    if caller.is_lead {
        return Ok(());
    }
    match task.owner.as_deref() {
        Some(owner) if owner == caller.name => Ok(()),
        Some(owner) => Err(format!(
            "task `{}` is owned by `{owner}`; caller `{}` is not authorized",
            task.task_id, caller.name
        )),
        None => Err(format!(
            "task `{}` has no owner; only the Team Lead can mutate it",
            task.task_id
        )),
    }
}

pub fn team_task_create(args: TeamTaskCreateArgs) -> Result<TeamTaskItem, String> {
    team_task_create_as(args, &TeamCaller::lead())
}

/// 以显式调用者身份创建任务（工具层必须走本入口）。
pub fn team_task_create_as(
    args: TeamTaskCreateArgs,
    _caller: &TeamCaller,
) -> Result<TeamTaskItem, String> {
    validate_task_text(&args.subject, &args.description)?;
    let write_scopes = normalize_write_scopes(args.write_scopes.unwrap_or_default())?;
    let blocked_by = args.blocked_by.unwrap_or_default();

    let mut lock = TEAM_TASKS.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    let map = lock.as_mut().unwrap();

    let active = map.values().filter(|task| task.status != "deleted").count();
    if active >= MAX_TEAM_TASKS {
        return Err(format!("Team task limit {MAX_TEAM_TASKS} reached"));
    }

    let task_id = format!("task-{}", Uuid::new_v4());
    validate_dependencies(map, &task_id, &blocked_by)?;
    if has_dependency_cycle(map, &task_id, &blocked_by) {
        return Err("task dependencies must not form a cycle".to_string());
    }
    let task = TeamTaskItem {
        task_id: task_id.clone(),
        subject: args.subject,
        description: args.description,
        status: "pending".to_string(),
        owner: None,
        revision: 1,
        write_scopes,
        blocked_by,
        created_at_ms: now_unix_ms(),
    };
    map.insert(task_id, task.clone());
    drop(lock);
    bump_team_activity();

    Ok(task)
}

pub fn team_task_get(args: TeamTaskGetArgs) -> Result<TeamTaskItem, String> {
    let lock = TEAM_TASKS.lock().unwrap();
    let map = lock.as_ref().ok_or_else(|| "No tasks exist".to_string())?;
    let task = map.get(&args.task_id).ok_or_else(|| format!("Task not found: {}", args.task_id))?;
    Ok(task.clone())
}

pub fn team_task_update(args: TeamTaskUpdateArgs) -> Result<TeamTaskItem, String> {
    team_task_update_as(args, &TeamCaller::lead())
}

/// 以显式调用者身份执行 CAS 任务流转（授权 + 状态机 + 依赖就绪的唯一收口）。
pub fn team_task_update_as(
    args: TeamTaskUpdateArgs,
    caller: &TeamCaller,
) -> Result<TeamTaskItem, String> {
    let mut lock = TEAM_TASKS.lock().unwrap();
    let map = lock.as_mut().ok_or_else(|| "No tasks exist".to_string())?;
    let snapshot = map
        .get(&args.task_id)
        .ok_or_else(|| "Task not found".to_string())?
        .clone();

    if snapshot.revision != args.expected_revision {
        return Err(format!(
            "CAS revision mismatch: expected {}, got {}",
            args.expected_revision, snapshot.revision
        ));
    }
    if snapshot.status == "deleted" {
        return Err(format!("Task {} is deleted", snapshot.task_id));
    }

    let mut next = snapshot.clone();
    match args.action {
        TeamTaskAction::Claim => {
            if snapshot.status != "pending" {
                return Err(format!(
                    "only a pending task can be claimed (task {} is {})",
                    snapshot.task_id, snapshot.status
                ));
            }
            if let Some(owner) = snapshot.owner.as_deref() {
                if owner != caller.name && !caller.is_lead {
                    return Err(format!("task `{}` is owned by `{owner}`", snapshot.task_id));
                }
            }
            if let Some(requested) = args.owner.as_deref() {
                if !caller.is_lead && requested != caller.name {
                    return Err("a teammate can only claim a task for itself".to_string());
                }
            }
            if !task_ready(map, &snapshot) {
                return Err(format!(
                    "task `{}` is blocked by unfinished dependencies",
                    snapshot.task_id
                ));
            }
            next.status = "in_progress".to_string();
            next.owner = Some(if caller.is_lead {
                args.owner.unwrap_or_else(|| caller.name.clone())
            } else {
                caller.name.clone()
            });
        }
        TeamTaskAction::Complete => {
            authorize_task(caller, &snapshot)?;
            if snapshot.status != "in_progress" {
                return Err(format!(
                    "only an in-progress task can complete (task {} is {})",
                    snapshot.task_id, snapshot.status
                ));
            }
            next.status = "completed".to_string();
        }
        TeamTaskAction::Release => {
            authorize_task(caller, &snapshot)?;
            if snapshot.status != "in_progress" {
                return Err(format!(
                    "only an in-progress task can be released (task {} is {})",
                    snapshot.task_id, snapshot.status
                ));
            }
            next.status = "pending".to_string();
            next.owner = None;
        }
        TeamTaskAction::Delete => {
            authorize_task(caller, &snapshot)?;
            let dependent = map.values().find(|task| {
                task.status != "deleted"
                    && task.task_id != snapshot.task_id
                    && task.blocked_by.iter().any(|dep| dep == &snapshot.task_id)
            });
            if let Some(dependent) = dependent {
                return Err(format!(
                    "task `{}` still blocks `{}`",
                    snapshot.task_id, dependent.task_id
                ));
            }
            next.status = "deleted".to_string();
        }
        TeamTaskAction::Edit => {
            authorize_task(caller, &snapshot)?;
            if args.subject.is_none() && args.description.is_none() && args.write_scopes.is_none() {
                return Err("task edit requires subject, description, or write_scopes".to_string());
            }
            if let Some(subject) = args.subject {
                next.subject = subject;
            }
            if let Some(description) = args.description {
                next.description = description;
            }
            if let Some(scopes) = args.write_scopes {
                next.write_scopes = normalize_write_scopes(scopes)?;
            }
            validate_task_text(&next.subject, &next.description)?;
        }
        TeamTaskAction::SetDependencies => {
            authorize_task(caller, &snapshot)?;
            let deps = args
                .blocked_by
                .ok_or_else(|| "set_dependencies requires blocked_by".to_string())?;
            validate_dependencies(map, &snapshot.task_id, &deps)?;
            if has_dependency_cycle(map, &snapshot.task_id, &deps) {
                return Err("task dependencies must not form a cycle".to_string());
            }
            next.blocked_by = deps;
        }
    }

    next.revision = snapshot.revision + 1;
    map.insert(next.task_id.clone(), next.clone());
    drop(lock);
    bump_team_activity();
    Ok(next)
}

/// 等待队友活动（对齐 DSH wait_agent 边沿机制）
pub fn wait_agent(args: WaitAgentArgs) -> Result<WaitAgentResult, String> {
    wait_agent_as(args, &TeamCaller::lead())
}

/// 以显式调用者身份等待团队边沿：调用者自己不计入「活跃同伴」。
pub fn wait_agent_as(
    args: WaitAgentArgs,
    caller: &TeamCaller,
) -> Result<WaitAgentResult, String> {
    let has_active_peer = {
        let lock = TEAMMATES.lock().unwrap();
        if let Some(ref map) = *lock {
            map.iter().any(|(target, member)| {
                target != "lead"
                    && target != &caller.name
                    && (member.status == TeammateStatus::Running
                        || member.status == TeammateStatus::Provisioning)
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

    // 真等待：记录基线版本后阻塞，任一边沿（成员状态/消息入队出队/任务流转）即返回；
    // 超时返回 `timed_out=true`。绝不返回"未超时且无进展"的假信号。
    const DEFAULT_WAIT_MS: u64 = 30_000;
    const MIN_WAIT_MS: u64 = 10_000;
    const MAX_WAIT_MS: u64 = 3_600_000;

    let timeout_ms = args
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(MIN_WAIT_MS, MAX_WAIT_MS);
    let baseline = team_activity_version();
    let resumed = wait_for_team_activity(baseline, Duration::from_millis(timeout_ms));
    Ok(WaitAgentResult {
        timed_out: !resumed,
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
    // 检查点 #1（drain 之前）：已被 `interrupt_agent` 标记 → 消费标记并停止本轮驱动
    // （C7.4：中断在下一次 drain 边界前生效；引擎无回合内 stop API）。
    //
    // 真·在途中断的收口口径与 C10.2 一致：本检查点尚未 drain，故无消息需要回填；但必须
    // 显式把成员状态收到 `Inactive`，绝不留在 `Running`（否则名单里会永久显示"运行中"）。
    // 若本检查点被挪到 drain 之后调用，则同样先 `requeue_messages` 再收口（见检查点 #2）。
    if take_interrupted(target) {
        set_teammate_status(target, TeammateStatus::Interrupted);
        return Ok(None);
    }

    let result = drain_inbox(ReadInboxArgs {
        target: target.to_string(),
    })?;
    if result.messages.is_empty() {
        // 收件箱为空＝唯一退出条件（Wave-61 C10-A2）。循环由 `drive_loop_in` 负责。
        return Ok(None);
    }

    // 检查点 #2（引擎调用之前）：drain 期间到来的中断 → 整批原序放回队首后停止，
    // 不丢弃任何消息、也不跑一个"已被要求停下"的 turn。
    if take_interrupted(target) {
        requeue_messages(target, &result.messages);
        set_teammate_status(target, TeammateStatus::Interrupted);
        return Ok(None);
    }

    // 启动前置：成员置为 Running 并清掉上一轮失败文本（对 `list_agents` 可见；
    // C10.3 迁移序 Active → Running → Inactive/Failed/Interrupted）。
    set_teammate_status(target, TeammateStatus::Running);
    set_teammate_error(target, None);

    // 整合收件箱内的消息内容
    let mut combined_prompt = format!("You are teammate '{}'. Incoming messages:\n", target);
    for msg in &result.messages {
        combined_prompt.push_str(&format!("- {}\n", msg.content));
    }
    combined_prompt.push_str("\nProcess these messages and provide your response or action report.");

    // 如果挂载了真实模型引擎，则执行真实 Turn（C8：以父会话工作区根建栈）。
    let engine = crate::agent::subagent_runtime::subagent_engine();
    let engine_was_wired = engine.is_some();
    // W2：派生阶段登记的 provider 选择在此消费；无登记 ⇒ `None`（引擎走 `run_turn_in` 旧链）。
    let selection = teammate_provider_selection(target);
    // W3 trace：派发时捕获的宿主 traceparent 在此消费；无登记 ⇒ `None`（队友 turn 自起 root）。
    let parent_traceparent = teammate_traceparent(target);
    let final_reply = match engine {
        Some(engine) => {
            // 会话延续：复用 register_teammate 预分配的固定 session_id。引擎以同一 id 建栈/续写
            // ⇒ 队友跨轮保留上下文；该 id 也已登记，turn 内工具调用可按 session 反查成员身份。
            let teammate_session = teammate_session_id(target);
            let request = crate::agent::subagent_runtime::SubagentRunRequest {
                prompt: &combined_prompt,
                timeout_ms: SUBAGENT_TURN_TIMEOUT_MS,
                workspace_root,
                provider_selection: selection.as_ref(),
                parent_traceparent: parent_traceparent.as_deref(),
                session_id: teammate_session.as_deref(),
                parent_session_id: None,
                workspace_id: None,
                fork_context: false,
            };
            match engine.run_turn_request(&request) {
                Ok(outcome) => {
                    // W3：成功 turn 的会话 id 登记进进程内可见性缓存（失败路径不写：避免
                    // `/agents` 把一次失败的孤儿会话当成可进入的子会话）。
                    set_teammate_session(target, &outcome.session_id);
                    outcome.final_text
                }
                Err(err) => {
                    // Wave-61 C10.2 失败补偿：把本轮 drain 的整批消息**按原顺序放回队首**
                    // （消息绝不丢失：引擎侧 busy/timeout/prompt_too_large/任意 Err 都不吞消息），
                    // 状态回 Inactive，并把错误文本投递 Lead 收件箱（不含任何假成功）。
                    // 说明：`prompt_too_large` 这类确定性失败会再次失败，直到调用方拆分消息；
                    // 这里不做静默丢弃，也不伪造成成功回执。
                    requeue_messages(target, &result.messages);
                    // B1795535505312：失败不再伪装成「空闲」——`Failed` + 错误文本让
                    // Lead 能区分「跑完了」与「跑挂了」，且不丢原来的回滚/回执语义。
                    set_teammate_status(target, TeammateStatus::Failed);
                    set_teammate_error(target, Some(err.clone()));
                    if target != LEAD_TARGET {
                        let _ = send_message(SendMessageArgs {
                            target: LEAD_TARGET.to_string(),
                            message: format!(
                                "Teammate '{}' turn failed ({} message(s) requeued): {}",
                                target,
                                result.messages.len(),
                                err
                            ),
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
    set_teammate_error(target, None);

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
    // D3 修复：软删除（delete ⇒ status="deleted"）任务不得再出现在看板 ——
    // TUI 投影对未知状态一律降级 Pending，泄漏的 deleted 任务会伪装成待办
    //（2026-11-08 live 实证：两条已删除探针任务仍列在 /tasks）。
    let mut tasks: Vec<TeamTaskItem> = if let Some(ref map) = *lock {
        map.values()
            .filter(|task| task.status != "deleted")
            .cloned()
            .collect()
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

#[cfg(test)]
mod team_activity_and_task_tests {
    use super::*;

    fn task(id: &str, status: &str, owner: Option<&str>, blocked_by: &[&str]) -> TeamTaskItem {
        TeamTaskItem {
            task_id: id.to_string(),
            subject: format!("task {id}"),
            description: "d".to_string(),
            status: status.to_string(),
            owner: owner.map(str::to_string),
            revision: 1,
            write_scopes: Vec::new(),
            blocked_by: blocked_by.iter().map(|dep| dep.to_string()).collect(),
            created_at_ms: 1,
        }
    }

    #[test]
    fn wait_on_activity_times_out_without_edges() {
        let activity = TeamActivity::new();
        let started = Instant::now();
        let changed = wait_on_team_activity(&activity, 0, Duration::from_millis(80));
        assert!(!changed, "无团队边沿时必须超时返回 false（禁止假成功）");
        assert!(
            started.elapsed() >= Duration::from_millis(60),
            "必须真的等待，而不是立即返回"
        );
    }

    #[test]
    fn wait_on_activity_wakes_on_edge() {
        let activity = std::sync::Arc::new(TeamActivity::new());
        let bumper = std::sync::Arc::clone(&activity);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            bump_activity_instance(&bumper);
        });
        let started = Instant::now();
        let changed = wait_on_team_activity(&activity, 0, Duration::from_secs(5));
        handle.join().expect("bumper thread");
        assert!(changed, "边沿发生后等待者必须被唤醒");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "唤醒不得落到超时分支"
        );
    }

    #[test]
    fn wait_on_activity_returns_immediately_when_baseline_is_stale() {
        let activity = TeamActivity::new();
        bump_activity_instance(&activity);
        let started = Instant::now();
        assert!(wait_on_team_activity(&activity, 0, Duration::from_secs(5)));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "基线之后已有边沿时必须立即返回"
        );
    }

    #[test]
    fn task_text_budget_and_write_scopes_fail_closed() {
        assert!(validate_task_text("", "d").is_err());
        assert!(validate_task_text(&"x".repeat(MAX_TASK_SUBJECT_CHARS + 1), "d").is_err());
        assert!(validate_task_text(&"x".repeat(MAX_TASK_DESCRIPTION_CHARS + 1), "d").is_err());
        assert!(validate_task_text("ok", "d").is_ok());

        assert!(normalize_write_scopes(vec!["/etc".to_string()]).is_err());
        assert!(normalize_write_scopes(vec!["../x".to_string()]).is_err());
        assert!(normalize_write_scopes(vec![" ".to_string()]).is_err());
        assert_eq!(
            normalize_write_scopes(vec![
                "src/a".to_string(),
                "src/a".to_string(),
                "src/b".to_string()
            ])
            .expect("normalize"),
            vec!["src/a".to_string(), "src/b".to_string()]
        );
    }

    #[test]
    fn dependencies_are_validated_for_self_duplicates_cycles_and_readiness() {
        let mut map: HashMap<String, TeamTaskItem> = HashMap::new();
        map.insert("task-a".to_string(), task("task-a", "pending", None, &[]));
        map.insert(
            "task-b".to_string(),
            task("task-b", "pending", None, &["task-a"]),
        );

        assert!(validate_dependencies(&map, "task-c", &["task-a".to_string()]).is_ok());
        assert!(validate_dependencies(&map, "task-c", &["task-c".to_string()]).is_err());
        assert!(validate_dependencies(
            &map,
            "task-c",
            &["task-a".to_string(), "task-a".to_string()]
        )
        .is_err());
        assert!(validate_dependencies(&map, "task-c", &["task-missing".to_string()]).is_err());

        // task-b → task-a 无环；task-a → task-b 成环。
        assert!(!has_dependency_cycle(&map, "task-c", &["task-b".to_string()]));
        assert!(has_dependency_cycle(&map, "task-a", &["task-b".to_string()]));

        let mut ready_map: HashMap<String, TeamTaskItem> = HashMap::new();
        ready_map.insert(
            "task-a".to_string(),
            task("task-a", "completed", None, &[]),
        );
        assert!(task_ready(
            &ready_map,
            &task("task-b", "pending", None, &["task-a"])
        ));
        assert!(!task_ready(
            &ready_map,
            &task("task-b", "pending", None, &["task-a", "task-x"])
        ));
        ready_map.insert(
            "task-a".to_string(),
            task("task-a", "in_progress", None, &[]),
        );
        assert!(!task_ready(
            &ready_map,
            &task("task-b", "pending", None, &["task-a"])
        ));
    }

    #[test]
    fn task_mutation_authorization_is_owner_or_lead_only() {
        let lead = TeamCaller::lead();
        let teammate = TeamCaller {
            name: "reviewer".to_string(),
            is_lead: false,
        };
        let owned = task("task-a", "in_progress", Some("reviewer"), &[]);
        let foreign = task("task-b", "in_progress", Some("executor"), &[]);
        let unowned = task("task-c", "pending", None, &[]);
        assert!(authorize_task(&lead, &owned).is_ok());
        assert!(authorize_task(&teammate, &owned).is_ok());
        assert!(authorize_task(&teammate, &foreign).is_err());
        assert!(authorize_task(&teammate, &unowned).is_err());
    }
}

/// 2026-11-25：子智能体运行注册表与「宿主会话归属」的隔离语义。
///
/// 这些注册表只承载 TUI 可见性，不参与权限/生命周期裁决；用例断言的是**隔离与终态迁移**
/// 两条不变量（跨会话不得串话、引擎换 id 不得留幽灵「运行中」）。
#[cfg(test)]
mod subagent_and_host_session_tests {
    use super::*;

    #[test]
    fn subagent_runs_are_isolated_per_parent_session() {
        let mine = "host-subagent-mine";
        let other = "host-subagent-other";
        register_subagent_run("subagent-mine-1", "梳理调用链", Some(mine));
        register_subagent_run("subagent-other-1", "别人的子代理", Some(other));
        // 无宿主归属（内部直调）不得串到任何会话。
        register_subagent_run("subagent-orphan", "无归属", None);

        let listed = list_subagent_runs_for_session(mine);
        let ids: Vec<&str> = listed.iter().map(|run| run.session_id.as_str()).collect();
        assert!(ids.contains(&"subagent-mine-1"), "本会话子代理必须可见：{ids:?}");
        assert!(
            !ids.contains(&"subagent-other-1"),
            "别的会话的子代理不得串入：{ids:?}"
        );
        assert!(
            !ids.contains(&"subagent-orphan"),
            "无宿主归属的子代理不归任何会话：{ids:?}"
        );
        assert_eq!(
            listed[0].status,
            SubagentRunStatus::Running,
            "登记即运行中"
        );

        // 空宿主会话 ⇒ 空列表（不把「无归属」读成「全量」）。
        assert!(list_subagent_runs_for_session("  ").is_empty());
    }

    #[test]
    fn finishing_a_subagent_run_updates_status_and_error() {
        let host = "host-subagent-finish";
        register_subagent_run("subagent-finish-1", "跑测试", Some(host));
        finish_subagent_run("subagent-finish-1", SubagentRunStatus::Failed, Some("boom"));
        let run = list_subagent_runs_for_session(host)
            .into_iter()
            .find(|run| run.session_id == "subagent-finish-1")
            .expect("记录必须存在");
        assert_eq!(run.status, SubagentRunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some("boom"));
        assert!(run.status.needs_attention(), "失败必须计入需关注");
        assert!(!run.status.is_running(), "终态不再算运行中");
    }

    #[test]
    fn rebind_moves_run_to_engine_resolved_session_id() {
        let host = "host-subagent-rebind";
        register_subagent_run("subagent-pre-allocated", "迁移用例", Some(host));
        rebind_subagent_run("subagent-pre-allocated", "subagent-engine-real");
        let runs = list_subagent_runs_for_session(host);
        assert!(
            runs.iter()
                .any(|run| run.session_id == "subagent-engine-real"),
            "必须迁移到引擎真实 id：{runs:?}"
        );
        assert!(
            !runs
                .iter()
                .any(|run| run.session_id == "subagent-pre-allocated"),
            "旧 id 不得残留（否则页签留一条永不结束的运行中）：{runs:?}"
        );
    }

    #[test]
    fn teammate_host_session_round_trips_and_resolves_team_caller() {
        let host = "host-team-owner";
        // 队友子会话 → 宿主会话的反查依赖 TEAMMATE_SESSIONS（登记 teammate 时写入）。
        set_teammate_session("worker-host-test", "team-worker-host-test-0001");
        set_teammate_host_session("worker-host-test", host);

        assert_eq!(
            teammate_host_session("worker-host-test").as_deref(),
            Some(host),
            "队友宿主归属必须可回读"
        );
        assert_eq!(
            host_session_for_caller(Some("team-worker-host-test-0001")).as_deref(),
            Some(host),
            "队友会话必须反查到宿主会话"
        );
        assert_eq!(
            host_session_for_caller(Some(host)).as_deref(),
            Some(host),
            "宿主会话归属就是它自己"
        );
        assert!(host_session_for_caller(None).is_none(), "无上下文 ⇒ 无归属");
    }

    #[test]
    fn list_agents_for_session_drops_lead_and_foreign_members() {
        let host = "host-agents-owner";
        let other = "host-agents-other";
        let mut member = TeammateMember {
            target: "agent-host-test".to_string(),
            name: "Ada".to_string(),
            role: "executor".to_string(),
            status: TeammateStatus::Active,
            description: String::new(),
            is_driving: false,
            last_error: None,
        };
        {
            let mut lock = TEAMMATES.lock().unwrap();
            let map = lock.get_or_insert_with(HashMap::new);
            map.insert(member.target.clone(), member.clone());
            member.target = "agent-host-other".to_string();
            member.name = "Bob".to_string();
            map.insert(member.target.clone(), member);
        }
        set_teammate_host_session("agent-host-test", host);
        set_teammate_host_session("agent-host-other", other);

        let mine = list_agents_for_session(host).expect("名册读取必须成功");
        let targets: Vec<&str> = mine.agents.iter().map(|m| m.target.as_str()).collect();
        assert!(targets.contains(&"agent-host-test"), "本会话队友必须可见：{targets:?}");
        assert!(
            !targets.contains(&"agent-host-other"),
            "别的会话队友不得串入：{targets:?}"
        );
        assert!(
            !targets.contains(&"lead"),
            "伪成员 lead 不得进入会话视图：{targets:?}"
        );
    }

    #[test]
    fn team_task_list_for_session_filters_by_creator_host() {
        let host = "host-task-owner";
        let other = "host-task-other";
        let create = |subject: &str| {
            team_task_create(TeamTaskCreateArgs {
                subject: subject.to_string(),
                description: "d".to_string(),
                write_scopes: None,
                blocked_by: None,
            })
            .expect("任务创建必须成功")
        };
        let mine = create("本会话任务");
        let foreign = create("别的会话任务");
        set_team_task_host_session(&mine.task_id, host);
        set_team_task_host_session(&foreign.task_id, other);

        let listed = team_task_list_for_session(host).expect("任务列表必须成功");
        let ids: Vec<&str> = listed.tasks.iter().map(|t| t.task_id.as_str()).collect();
        assert!(ids.contains(&mine.task_id.as_str()), "本会话任务必须可见：{ids:?}");
        assert!(
            !ids.contains(&foreign.task_id.as_str()),
            "别的会话任务不得串入：{ids:?}"
        );
    }
}

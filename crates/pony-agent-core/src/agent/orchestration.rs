use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
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
    /// W2（additive）：可选 provider 覆盖；`None` = 继承宿主 provider。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// W2（additive）：可选 model 覆盖；`None` = 继承宿主 model。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
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
    if target == LEAD_TARGET {
        return None;
    }
    let lock = TEAMMATE_SESSIONS.lock().unwrap();
    lock.as_ref()?.get(target).cloned()
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

/// W2：带宿主 provider 继承/显式覆盖的子代理入口（实现体）。
///
/// 复刻 [`subagent_in_workspace`] 的许可门 / 前后台语义，只把引擎调用换成
/// [`SubagentTurnEngine::run_turn_with_selection`]：
/// - `host_selection=Some` ⇒ 子代理继承宿主 provider（再按 `args` 覆盖 provider/model）；
/// - `host_selection=None` 且 `args` 无覆盖 ⇒ 保持 legacy（`selection=None` ⇒ 引擎走 `run_turn_in`）。
pub fn subagent_in_workspace_with_provider(
    args: SubagentArgs,
    workspace_root: Option<&std::path::Path>,
    host_selection: Option<&ResolvedProviderSelection>,
) -> Result<SubagentResult, String> {
    // W2 失败前置：**先解析**，解析失败立即返回 —— 绝不先起后台线程再异步失败。
    let selection = resolve_optional_teammate_provider_selection(
        host_selection,
        args.provider_id.as_deref(),
        args.model_id.as_deref(),
    )?;

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
            if let Err(err) = engine.run_turn_with_selection(
                &args.prompt,
                SUBAGENT_TURN_TIMEOUT_MS,
                bg_root.as_deref(),
                selection.as_ref(),
            ) {
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
    match engine.run_turn_with_selection(
        &args.prompt,
        SUBAGENT_TURN_TIMEOUT_MS,
        workspace_root,
        selection.as_ref(),
    ) {
        Ok(outcome) => Ok(SubagentResult {
            subagent_id,
            status: "completed".to_string(),
            output: Some(outcome.final_text),
        }),
        Err(err) => Err(err),
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
    // 1) 失败前置：先解析（无 host 且无显式覆盖时允许 None，保持 legacy 语义）。
    let selection = resolve_optional_teammate_provider_selection(
        host_selection,
        args.provider_id.as_deref(),
        args.model_id.as_deref(),
    )?;

    let auto_drive = crate::agent::subagent_runtime::subagent_engine().is_some();
    let (result, target, initial_prompt) = register_teammate(args)?;

    // 2) 登记成功后持久化该 target 的 provider 选择（驱动阶段唯一真值源）。
    if let Some(selection) = selection {
        set_teammate_provider(&target, selection);
    }

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
            processed: false,
            delivery_state: DELIVERY_STATE_QUEUED.to_string(),
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

/// 打断正在运行的智能体（保留其信箱队列）
pub fn interrupt_agent(args: InterruptAgentArgs) -> Result<InterruptAgentResult, String> {
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
        member.status = TeammateStatus::Inactive;
        (prev, TeammateStatus::Inactive)
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
    // 检查点 #1（drain 之前）：已被 `interrupt_agent` 标记 → 消费标记并停止本轮驱动
    // （C7.4：中断在下一次 drain 边界前生效；引擎无回合内 stop API）。
    //
    // 真·在途中断的收口口径与 C10.2 一致：本检查点尚未 drain，故无消息需要回填；但必须
    // 显式把成员状态收到 `Inactive`，绝不留在 `Running`（否则名单里会永久显示"运行中"）。
    // 若本检查点被挪到 drain 之后调用，则同样先 `requeue_messages` 再收口（见检查点 #2）。
    if take_interrupted(target) {
        set_teammate_status(target, TeammateStatus::Inactive);
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
        set_teammate_status(target, TeammateStatus::Inactive);
        return Ok(None);
    }

    // 启动前置：成员置为 Running（对 `list_agents` 可见；C10.3 迁移序 Active → Running → Inactive）。
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
    // W2：派生阶段登记的 provider 选择在此消费；无登记 ⇒ `None`（引擎走 `run_turn_in` 旧链）。
    let selection = teammate_provider_selection(target);
    let final_reply = match engine {
        Some(engine) => {
            match engine.run_turn_with_selection(
                &combined_prompt,
                SUBAGENT_TURN_TIMEOUT_MS,
                workspace_root,
                selection.as_ref(),
            ) {
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
                    set_teammate_status(target, TeammateStatus::Inactive);
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

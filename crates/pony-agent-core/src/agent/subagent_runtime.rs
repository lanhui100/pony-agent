//! wave-37 C3 子代理真实执行引擎。
//!
//! 契约见 `.dev-team/contracts/wave-37-tool-surface-subagent.md`（C3 节）：
//! - `SubagentTurnEngine` trait / `SubagentRunOutcome` / OnceLock 全局槽；
//! - `HostTurnEngine` 真实现：独立 `AgentRuntimeBuilder` 建栈（独立
//!   `SessionStore::new` / `ProviderRegistryStore::new`，与宿主完全隔离），
//!   把 prompt 包为 `TurnInput` 运行一个 turn，从 `TurnResult` 提取
//!   final 文本与 session_id；
//! - 错误契约：`subagent_busy`（并发 >2）、`prompt_too_large`（>200000 字节）、
//!   `subagent_timeout`（>timeout_ms，检测粒度 ≤1s，超时会话标注为孤儿）。

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::agent::config::{ProviderSelectionResolver, ResolvedProviderSelection};
use crate::agent::runtime::{AgentRuntimeBuilder, TurnInput};

/// W2：把**已解析**的 provider 选择原样喂给本次子代理栈的解析器。
///
/// 子代理不再走 `ProviderRegistryStore` 的默认解析链（那会丢掉宿主的 model/密钥/推理档位），
/// 而是固定返回宿主继承（或显式覆盖）后的快照 —— `resolve_provider_selection` 的
/// `provider_id`/`model_id` 入参因此被忽略（快照即最终选择，`TurnInput` 也不再传覆盖）。
struct FixedSelectionResolver {
    selection: ResolvedProviderSelection,
}

impl FixedSelectionResolver {
    fn new(selection: ResolvedProviderSelection) -> Self {
        Self { selection }
    }
}

impl ProviderSelectionResolver for FixedSelectionResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

/// 单个字代理 turn 的执行结果。
pub struct SubagentRunOutcome {
    /// 子代理 turn 的最终文本输出。
    pub final_text: String,
    /// 子代理会话 id（与编排层 `SubagentResult.subagent_id` 同一个值）。
    pub session_id: String,
}

/// 子代理 turn 的完整派发请求（会话归属 / id 对齐 / 上下文继承的唯一载体）。
///
/// 新增字段全部可选，既有调用方经 [`SubagentRunRequest::new`] 构造后行为与旧
/// `run_turn_with_selection_and_parent` 逐字一致。
pub struct SubagentRunRequest<'a> {
    pub prompt: &'a str,
    pub timeout_ms: u64,
    pub workspace_root: Option<&'a Path>,
    pub provider_selection: Option<&'a ResolvedProviderSelection>,
    pub parent_traceparent: Option<&'a str>,
    /// 编排层预分配的子会话 id；`Some` ⇒ 引擎必须复用，保证工具返回值可检索。
    pub session_id: Option<&'a str>,
    /// 父会话 id：用于继承 workspace 绑定与（可选）fork 历史。
    pub parent_session_id: Option<&'a str>,
    /// 父会话 workspace id；`None` ⇒ 引擎从父会话快照 / workspace root 反查。
    pub workspace_id: Option<&'a str>,
    /// 是否把父会话已完成历史带进子 turn 作为 seed。
    pub fork_context: bool,
}

impl<'a> SubagentRunRequest<'a> {
    /// 最小请求：仅 prompt + 预算，其余字段缺省（= 旧 `run_turn` 语义）。
    pub fn new(prompt: &'a str, timeout_ms: u64) -> Self {
        Self {
            prompt,
            timeout_ms,
            workspace_root: None,
            provider_selection: None,
            parent_traceparent: None,
            session_id: None,
            parent_session_id: None,
            workspace_id: None,
            fork_context: false,
        }
    }
}

/// 子代理 turn 执行引擎契约（真实实现，禁止返回伪字符串）。
pub trait SubagentTurnEngine: Send + Sync {
    /// 以给定 prompt 运行一个完整子代理 turn。
    fn run_turn(&self, prompt: &str, timeout_ms: u64) -> Result<SubagentRunOutcome, String>;

    /// 引擎是否在 `run_turn` 内部**自行**获取并发/体积许可（许可唯一真源）。
    ///
    /// 默认 `false`：注入式引擎（测试 Stub、宿主自定义引擎）不自持，由
    /// [`crate::agent::orchestration::subagent`] 入口门兜住 —— wave-37 C3.2 契约要求
    /// 「Stub 引擎注入场景下 busy/oversize 契约同样生效」。
    ///
    /// [`HostTurnEngine`] 返回 `true`：它在 `run_turn` 内调用 [`subagent_gate_enter`]，
    /// 因此入口门必须**跳过**，否则一次调用会扣两个许可（有效并发被压成 1）。
    fn gates_concurrency_internally(&self) -> bool {
        false
    }

    /// 以**显式工作区根**运行一个子代理 turn（Wave-61 C8，additive）。
    ///
    /// 默认实现原样委托 [`SubagentTurnEngine::run_turn`]，因此既有 Stub/宿主引擎零改动，
    /// 也不会因新增方法而破坏 `impl SubagentTurnEngine for X`（默认方法不要求覆写）。
    /// [`HostTurnEngine`] 覆写它：`Some(root)` 用该 root 建栈，`None` 回落既有解析链。
    fn run_turn_in(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
    ) -> Result<SubagentRunOutcome, String> {
        let _ = workspace_root;
        self.run_turn(prompt, timeout_ms)
    }

    /// 以**显式 provider 选择**运行一个子代理 turn（W2，additive 接口桩）。
    ///
    /// 默认实现忽略 `selection` 并原样委托 [`SubagentTurnEngine::run_turn_in`]：
    /// 既有 Stub/宿主引擎零改动、既有 `run_turn`/`run_turn_in` 语义不变；
    /// 覆写（真实 provider 继承/自定义）由 W2 GREEN 补齐。
    fn run_turn_with_selection(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
        selection: Option<&crate::agent::config::ResolvedProviderSelection>,
    ) -> Result<SubagentRunOutcome, String> {
        let _ = selection;
        self.run_turn_in(prompt, timeout_ms, workspace_root)
    }

    /// W3 trace：在显式 provider 选择之外，再继承**宿主 turn 的 traceparent**。
    ///
    /// 默认实现忽略 `parent_traceparent` 并原样委托
    /// [`SubagentTurnEngine::run_turn_with_selection`]（既有 Stub/宿主引擎零改动）；
    /// [`HostTurnEngine`] 覆写它：建栈后把父 traceparent 注入 runtime，使本队友 turn 的
    /// `agent.turn` span 挂在宿主 turn span 之下（同一 trace id，而非另起 root trace）。
    fn run_turn_with_selection_and_parent(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
        selection: Option<&crate::agent::config::ResolvedProviderSelection>,
        parent_traceparent: Option<&str>,
    ) -> Result<SubagentRunOutcome, String> {
        let _ = parent_traceparent;
        self.run_turn_with_selection(prompt, timeout_ms, workspace_root, selection)
    }

    /// 以完整派发请求运行一个子代理 turn（additive）。
    ///
    /// 默认实现忽略新增的会话归属字段，原样委托
    /// [`SubagentTurnEngine::run_turn_with_selection_and_parent`]：既有 Stub / 宿主引擎
    /// 零改动；[`HostTurnEngine`] 覆写它，落实 workspace 继承、id 对齐与 `fork_context`。
    fn run_turn_request(
        &self,
        request: &SubagentRunRequest<'_>,
    ) -> Result<SubagentRunOutcome, String> {
        self.run_turn_with_selection_and_parent(
            request.prompt,
            request.timeout_ms,
            request.workspace_root,
            request.provider_selection,
            request.parent_traceparent,
        )
    }
}

static SUBAGENT_ENGINE: OnceLock<Arc<dyn SubagentTurnEngine>> = OnceLock::new();

/// 注册全局子代理引擎；重复注册返回 Err。
pub fn set_subagent_engine(eng: Arc<dyn SubagentTurnEngine>) -> Result<(), String> {
    SUBAGENT_ENGINE.set(eng).map_err(|_| {
        "subagent engine already set".to_string()
    })
}

/// 读取全局子代理引擎槽。
pub fn subagent_engine() -> Option<Arc<dyn SubagentTurnEngine>> {
    SUBAGENT_ENGINE.get().cloned()
}

// ==========================================
// C3 错误契约常量（NFR 基线对齐）
// ==========================================

/// prompt 字节上限（NFR: subagent_max_prompt_bytes=200000）。
pub const SUBAGENT_MAX_PROMPT_BYTES: usize = 200_000;

/// 默认并发子代理上限（设置项 `subagentMaxConcurrent` 未配置时生效）。
///
/// 评估依据（2026-11-13）：单个 subagent turn 的峰值占用 ≈{完整会话上下文 + 工具输出 +
/// 图片解码}，多并发实测单进程 RSS 可达 7.7GB（4 个子代理会话 + 主会话）。取 4 作为默认：
/// 16GB 机器上 4 路峰值约 6–8GB，仍为主会话与系统留出余量；3 在「主会话 + 2 队友」场景
/// 已无并行余量（B125 反馈「3 太少」）。更大机器可由设置页上调，上限见硬顶。
pub const DEFAULT_SUBAGENT_MAX_CONCURRENT: usize = 4;

/// 并发上限硬顶：设置值再大也不得突破（32GB 级机器的可用上限），防止误填导致 OOM。
pub const SUBAGENT_MAX_CONCURRENT_CEILING: usize = 8;

/// 当前生效的并发上限；**唯一写入口**是 [`set_subagent_max_concurrent`]。
static SUBAGENT_CONCURRENT_LIMIT: AtomicUsize = AtomicUsize::new(DEFAULT_SUBAGENT_MAX_CONCURRENT);

static SUBAGENT_INFLIGHT: AtomicUsize = AtomicUsize::new(0);

/// 当前生效的并发上限（恒在 `1..=SUBAGENT_MAX_CONCURRENT_CEILING` 内）。
pub fn subagent_max_concurrent() -> usize {
    SUBAGENT_CONCURRENT_LIMIT
        .load(Ordering::SeqCst)
        .clamp(1, SUBAGENT_MAX_CONCURRENT_CEILING)
}

/// 把配置值规范化为落盘/生效口径：`None` 保持未配置（引擎回落默认值），
/// `Some` 一律 clamp 到 `1..=SUBAGENT_MAX_CONCURRENT_CEILING`。
///
/// 单一真源：设置层（`app_settings`）落盘前与运行时门都走本函数，杜绝两处各自 clamp。
pub fn normalize_subagent_max_concurrent(configured: Option<usize>) -> Option<usize> {
    configured.map(|value| value.clamp(1, SUBAGENT_MAX_CONCURRENT_CEILING))
}

/// 注入并发上限（`None` ⇒ 回到默认值），返回最终生效值。
///
/// 生效时机：**下一次** `acquire` 起读新上限；已在途的 turn 不受影响（引擎无回合内
/// stop API，下调上限只阻止新许可，不会取消正在跑的 turn）。
pub fn set_subagent_max_concurrent(configured: Option<usize>) -> usize {
    let effective =
        normalize_subagent_max_concurrent(configured).unwrap_or(DEFAULT_SUBAGENT_MAX_CONCURRENT);
    SUBAGENT_CONCURRENT_LIMIT.store(effective, Ordering::SeqCst);
    effective
}

/// scope guard：持有期间占一个并发许可，Drop 时无论成败（含 panic 展开）
/// 必然归还，杜绝许可泄漏。
struct InflightGuard;

impl InflightGuard {
    fn acquire() -> Result<Self, String> {
        loop {
            // 每次重试都重读上限：设置变更对后续 acquire 立即生效，无需重启。
            let limit = subagent_max_concurrent();
            let current = SUBAGENT_INFLIGHT.load(Ordering::SeqCst);
            if current >= limit {
                return Err("subagent_busy".to_string());
            }
            if SUBAGENT_INFLIGHT
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(InflightGuard);
            }
        }
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        SUBAGENT_INFLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 并发许可 guard（对外导出：orchestration::subagent 入口在引擎调用前
/// 统一过门，保证 Stub 引擎注入场景下 busy/oversize 契约同样生效）。
pub struct SubagentInflightGuard(InflightGuard);

/// 子代理入口统一门：prompt 字节 >200000 → `prompt_too_large`；
/// 并发运行数已达生效上限（[`subagent_max_concurrent`]）→ `subagent_busy`。
/// 返回的 guard Drop 时必然归还许可。
pub fn subagent_gate_enter(prompt: &str) -> Result<SubagentInflightGuard, String> {
    if prompt.as_bytes().len() > SUBAGENT_MAX_PROMPT_BYTES {
        return Err("prompt_too_large".to_string());
    }
    Ok(SubagentInflightGuard(InflightGuard::acquire()?))
}

/// 宿主默认引擎：独立 `AgentRuntimeBuilder` 建栈运行子代理 turn。
pub struct HostTurnEngine {
    /// 父会话工作区根（wave-58 契约 F1-5）。
    ///
    /// 子代理必须以**父会话工作区根**构建 runtime（继承项目指令与项目技能），
    /// 不得再固定 `compute_default_workspace_root()`；`None` 时才回落默认根。
    pub workspace_root: Option<PathBuf>,
    /// W3b：宿主共享的会话存储（与宿主 control plane 同一 `Arc`）。
    ///
    /// `Some` ⇒ 子代理建栈时 `.shared_session_store(...)`，于是队友 turn 的会话直接落进宿主
    /// `SessionStore`（`/agents` 的 Enter 才可 `hydrate`）；`None` ⇒ 维持旧行为（独立 store）。
    pub shared_sessions: Option<Arc<std::sync::RwLock<crate::agent::session::SessionStore>>>,
}

impl Default for HostTurnEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl HostTurnEngine {
    /// 无父上下文的引擎：`workspace_root = None`，`run_turn` 回落默认工作区根。
    pub fn new() -> Self {
        Self {
            workspace_root: None,
            shared_sessions: None,
        }
    }

    /// 注入父会话工作区根，返回自身（契约 F1-5）。
    pub fn with_workspace_root(mut self, root: Option<PathBuf>) -> Self {
        self.workspace_root = root;
        self
    }

    /// W3b：注入宿主共享的会话存储（`None` = 保持独立 store 旧行为）。
    pub fn with_shared_sessions(
        mut self,
        shared_sessions: Option<Arc<std::sync::RwLock<crate::agent::session::SessionStore>>>,
    ) -> Self {
        self.shared_sessions = shared_sessions;
        self
    }

    /// 子代理实际建栈根（Wave-58 F1-5）。
    ///
    /// 父会话工作区根优先（子代理由此继承父会话的项目指令与项目技能）；
    /// 父根缺失或非绝对路径时回落 `compute_default_workspace_root()`；
    /// 两者都不可得才用进程 cwd 兜底，任何情况都不 panic。
    fn resolved_workspace_root(&self) -> PathBuf {
        if let Some(root) = self
            .workspace_root
            .as_ref()
            .filter(|root| root.is_absolute())
        {
            return root.clone();
        }
        crate::agent::workspace::compute_default_workspace_root()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl HostTurnEngine {
    /// 从共享会话存储读取父会话的 workspace 绑定与（可选）已完成历史。
    ///
    /// 只读快照：不创建父会话、不落盘、不占写锁。共享 store 缺失或父会话不存在时
    /// 返回 `(None, [])` —— 绝不伪造归属或上下文。
    fn inherited_parent_context(
        &self,
        parent_session_id: Option<&str>,
        fork_context: bool,
    ) -> (
        Option<String>,
        Vec<crate::agent::session::TurnHistoryMessage>,
    ) {
        let Some(parent_session_id) = parent_session_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return (None, Vec::new());
        };
        let Some(shared) = self.shared_sessions.as_ref() else {
            return (None, Vec::new());
        };
        let sessions = shared.read().unwrap_or_else(|poisoned| {
            tracing::warn!(target: "control_plane", error = %poisoned, "[pony-agent] sessions rwlock poisoned");
            poisoned.into_inner()
        });
        let snapshot = sessions.snapshot_at_readonly(Some(parent_session_id), None, &[]);
        let history = if fork_context {
            seed_completed_history(snapshot.history)
        } else {
            Vec::new()
        };
        (snapshot.workspace_id, history)
    }

    /// 子会话 workspace 归属解析：显式 id → 父会话快照 → 按 root 反查已注册 workspace。
    ///
    /// 最后一步覆盖「父会话尚未盖章 workspace id，但调用方给了真实 root」的场景；
    /// 三步都失败时返回 `None`（保持可诊断的缺失，不静默落默认 workspace 之外的值）。
    fn resolve_child_workspace_id(
        &self,
        explicit: Option<&str>,
        inherited: Option<String>,
        workspace_root: &Path,
    ) -> Option<String> {
        if let Some(explicit) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
            return Some(explicit.to_string());
        }
        if inherited.is_some() {
            return inherited;
        }
        let shared = self.shared_sessions.as_ref()?;
        let sessions = shared.read().unwrap_or_else(|poisoned| {
            tracing::warn!(target: "control_plane", error = %poisoned, "[pony-agent] sessions rwlock poisoned");
            poisoned.into_inner()
        });
        sessions
            .list_workspaces()
            .into_iter()
            .find(|workspace| workspace_root_matches(&workspace.root_path, workspace_root))
            .map(|workspace| workspace.id)
    }

    /// 完整派发请求的建栈实现（workspace / id / fork_context 的唯一落点）。
    fn run_turn_with_request(
        &self,
        request: &SubagentRunRequest<'_>,
        workspace_root: PathBuf,
    ) -> Result<SubagentRunOutcome, String> {
        // 1) prompt 体积门 + 2) 并发门：引擎是许可的唯一真源（Guard Drop 归还许可，
        //    panic 展开路径同样安全）。
        let inflight = subagent_gate_enter(request.prompt)?;

        // 3) 会话 id：编排层预分配时**必须**复用（工具返回值与真实子会话同一个 id）；
        //    仅旧调用方未预分配时才自生成。
        let session_id = request
            .session_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("subagent-{}", Uuid::new_v4()));
        let thread_session_id = session_id.clone();
        let prompt_owned = request.prompt.to_string();
        let selection = request.provider_selection.cloned();
        let parent_traceparent = request
            .parent_traceparent
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        // 父上下文在**父线程**读取（`self` / store 句柄不跨线程），worker 只收快照数据。
        let (inherited_workspace_id, history) =
            self.inherited_parent_context(request.parent_session_id, request.fork_context);
        let workspace_id =
            self.resolve_child_workspace_id(request.workspace_id, inherited_workspace_id, &workspace_root);
        // W3b：共享会话存储句柄在父线程克隆后 move 进 worker（`self` 不跨线程）。
        let shared_sessions = self.shared_sessions.clone();

        // AgentRuntime 的 Send+Sync 不作为对外契约假设：runtime 在工作线程
        // 内部构建并持有，跨线程只传 outcome（TurnResult: Clone + Send 数据）。
        let (tx, rx) = mpsc::channel::<Result<crate::agent::runtime::TurnResult, String>>();
        let worker = std::thread::spawn(move || {
            // Wave-61 C.2：并发许可必须**随工作线程**存活。父线程在超时分支提前 return 时
            // worker 仍在跑（孤儿会话），若许可留在父线程会被提前归还，实际并发会突破
            // 生效并发上限。move 进本线程后，只有线程真正结束才归还。
            let _inflight = inflight;
            let result = catch_unwind(AssertUnwindSafe(move || {
                // Wave-61 C12.2：本 worker 是裸 `std::thread::spawn`（**无** reactor），
                // 之所以安全，是因为 provider 侧所有 reqwest/tokio future（`send()` /
                // `text()` / `stream.next()` / `tokio::time::timeout(...)`）都已在 **async
                // 块内**构造，future 自身的创建不再触碰 `Handle::current()`；
                // 实际轮询统一走 `runtime_helper::block_on`（无 handle 时回落到进程级静态
                // Runtime）。因此这里**不需要**再 `rt.enter()`，保持裸线程即可。

                // 独立建栈：默认不共享宿主 SessionStore / ProviderRegistryStore。
                // W3b：宿主注入了共享会话存储时复用它 —— 队友会话由此进入宿主 `list_sessions()`。
                let mut builder = AgentRuntimeBuilder::new().workspace_root(workspace_root);
                if let Some(shared_sessions) = shared_sessions {
                    builder = builder.shared_session_store(shared_sessions);
                }
                // W2：有显式选择时用 FixedSelectionResolver 固定本栈 provider（宿主继承/覆盖）。
                if let Some(selection) = selection {
                    builder = builder.provider_resolver(Box::new(
                        FixedSelectionResolver::new(selection),
                    ));
                }
                let mut runtime = builder.build();
                // W3 trace：父 traceparent 在建栈后、跑 turn 前注入（本栈后续 turn 一律继承）。
                if let Some(parent_traceparent) = parent_traceparent {
                    runtime.set_parent_traceparent(Some(parent_traceparent));
                }
                let input = TurnInput {
                    message: prompt_owned,
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some(thread_session_id),
                    node_id: None,
                    history,
                    images: Vec::new(),
                    workspace_id,
                    plan_execution_approval: None,
                    tool_authorization_mode: None,
                };
                runtime.run_turn(input)
            }));
            let mapped = match result {
                Ok(turn_result) => Ok(turn_result),
                Err(_) => Err("subagent_engine_panicked".to_string()),
            };
            let _ = tx.send(mapped);
            // 发送后仍有 TurnResult 持有工作线程栈上数据， join 前不退出：
            // 超时路径下工作线程被有意遗留（孤儿），TurnResult 随线程存活。
        });

        // 4) 超时检测：≤1s 粒度 recv_timeout 自旋。
        let deadline = Duration::from_millis(request.timeout_ms);
        let started = Instant::now();
        loop {
            if started.elapsed() >= deadline {
                // 超时：引擎无 stop API，工作线程遗留为孤儿会话，
                // session_id 写入错误消息供运维排查。
                return Err(format!(
                    "subagent_timeout: orphan session_id={} (turn exceeded {}ms; no stop API, session left for inspection)",
                    session_id, request.timeout_ms
                ));
            }
            let wait = deadline
                .checked_sub(started.elapsed())
                .unwrap_or(Duration::ZERO)
                .min(Duration::from_secs(1));
            match rx.recv_timeout(wait) {
                Ok(Ok(turn_result)) => {
                    let _ = worker.join();
                    let final_text = if turn_result.assistant_message.trim().is_empty() {
                        turn_result.session_summary.clone()
                    } else {
                        turn_result.assistant_message.clone()
                    };
                    return Ok(SubagentRunOutcome {
                        final_text,
                        session_id,
                    });
                }
                Ok(Err(panic_msg)) => {
                    let _ = worker.join();
                    return Err(panic_msg);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // 发送端已 drop 且未送达结果：若仍在预算内属异常崩溃。
                    return Err("subagent_engine_worker_disconnected".to_string());
                }
            }
        }
    }
}

/// `fork_context` 的 seed 边界：只保留最后一个 assistant 消息（含）之前的历史，
/// 截断尚未完成的父 turn（其 user 消息已落库但 assistant 回复未产出），避免把半轮
/// 上下文带进子代理。
fn seed_completed_history(
    history: Vec<crate::agent::session::TurnHistoryMessage>,
) -> Vec<crate::agent::session::TurnHistoryMessage> {
    let Some(last_assistant) = history.iter().rposition(|message| message.role == "assistant")
    else {
        return Vec::new();
    };
    history.into_iter().take(last_assistant + 1).collect()
}

/// workspace root 比较（忽略尾部路径分隔符；不引入 canonicalize 的 IO 与 symlink 语义）。
fn workspace_root_matches(registered: &str, root: &Path) -> bool {
    let registered = registered.trim_end_matches(['/', '\\']);
    let target = root.display().to_string();
    let target = target.trim_end_matches(['/', '\\']);
    !registered.is_empty() && registered == target
}

impl SubagentTurnEngine for HostTurnEngine {
    /// 本引擎在 `run_turn` 内持门（见 `subagent_gate_enter`），故 orchestration 入口门必须跳过，
    /// 避免一次调用扣两个许可（Wave-61 C.1 双重扣减修正）。
    fn gates_concurrency_internally(&self) -> bool {
        true
    }

    fn run_turn(&self, prompt: &str, timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        self.run_turn_request(&SubagentRunRequest::new(prompt, timeout_ms))
    }

    /// Wave-61 C8：显式工作区根优先（桌面宿主的真实工作区在 `ToolRouter.workspace_root`，
    /// 不等同于 `compute_default_workspace_root()`）；`None` 时回落既有解析链。
    fn run_turn_in(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
    ) -> Result<SubagentRunOutcome, String> {
        let mut request = SubagentRunRequest::new(prompt, timeout_ms);
        request.workspace_root = workspace_root;
        self.run_turn_request(&request)
    }

    /// W2：显式 provider 选择优先（经完整请求转发，语义与旧实现逐字一致）。
    fn run_turn_with_selection(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
        selection: Option<&ResolvedProviderSelection>,
    ) -> Result<SubagentRunOutcome, String> {
        let mut request = SubagentRunRequest::new(prompt, timeout_ms);
        request.workspace_root = workspace_root;
        request.provider_selection = selection;
        self.run_turn_request(&request)
    }

    /// W3 trace：继承宿主 turn 的 traceparent（经完整请求转发）。
    fn run_turn_with_selection_and_parent(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
        selection: Option<&ResolvedProviderSelection>,
        parent_traceparent: Option<&str>,
    ) -> Result<SubagentRunOutcome, String> {
        let mut request = SubagentRunRequest::new(prompt, timeout_ms);
        request.workspace_root = workspace_root;
        request.provider_selection = selection;
        request.parent_traceparent = parent_traceparent;
        self.run_turn_request(&request)
    }

    /// 完整请求入口：解析建栈根后复用同一实现（workspace 绑定 / id 对齐 / fork_context）。
    fn run_turn_request(
        &self,
        request: &SubagentRunRequest<'_>,
    ) -> Result<SubagentRunOutcome, String> {
        let root = request
            .workspace_root
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| self.resolved_workspace_root());
        self.run_turn_with_request(request, root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 并发上限是进程级静态：本模块用例串行执行，且退出时恢复默认值（含 panic 展开）。
    static LIMIT_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct LimitTestGuard {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for LimitTestGuard {
        fn drop(&mut self) {
            set_subagent_max_concurrent(None);
        }
    }

    fn limit_test_guard() -> LimitTestGuard {
        LimitTestGuard {
            _guard: LIMIT_TEST_GUARD
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        }
    }

    #[test]
    fn normalize_subagent_max_concurrent_clamps_to_range() {
        assert_eq!(normalize_subagent_max_concurrent(None), None);
        assert_eq!(normalize_subagent_max_concurrent(Some(0)), Some(1));
        assert_eq!(normalize_subagent_max_concurrent(Some(3)), Some(3));
        assert_eq!(
            normalize_subagent_max_concurrent(Some(usize::MAX)),
            Some(SUBAGENT_MAX_CONCURRENT_CEILING)
        );
    }

    #[test]
    fn set_subagent_max_concurrent_applies_default_and_clamps() {
        let _guard = limit_test_guard();

        assert_eq!(
            set_subagent_max_concurrent(None),
            DEFAULT_SUBAGENT_MAX_CONCURRENT
        );
        assert_eq!(subagent_max_concurrent(), DEFAULT_SUBAGENT_MAX_CONCURRENT);

        assert_eq!(set_subagent_max_concurrent(Some(6)), 6);
        assert_eq!(subagent_max_concurrent(), 6);

        assert_eq!(set_subagent_max_concurrent(Some(0)), 1);
        assert_eq!(subagent_max_concurrent(), 1);

        assert_eq!(
            set_subagent_max_concurrent(Some(usize::MAX)),
            SUBAGENT_MAX_CONCURRENT_CEILING
        );
        assert_eq!(
            subagent_max_concurrent(),
            SUBAGENT_MAX_CONCURRENT_CEILING
        );
    }

    #[test]
    fn subagent_gate_enforces_effective_limit_and_releases_on_drop() {
        let _guard = limit_test_guard();
        set_subagent_max_concurrent(Some(3));

        let permit_a = subagent_gate_enter("a").expect("第 1 个许可可用");
        let permit_b = subagent_gate_enter("b").expect("第 2 个许可可用");
        let permit_c = subagent_gate_enter("c").expect("第 3 个许可可用");

        let busy = match subagent_gate_enter("d") {
            Ok(_) => panic!("第 4 个必须 busy"),
            Err(error) => error,
        };
        assert_eq!(busy, "subagent_busy");

        drop(permit_a);
        let permit_d = subagent_gate_enter("d").expect("释放后必须可重新获取");
        drop((permit_b, permit_c, permit_d));
    }

    #[test]
    fn subagent_gate_rejects_prompt_over_byte_limit_before_taking_permit() {
        let _guard = limit_test_guard();
        set_subagent_max_concurrent(Some(1));

        // 超长 prompt 必须先命中 oversized，而不是先占许可/返回 busy。
        let oversized = "x".repeat(SUBAGENT_MAX_PROMPT_BYTES + 1);
        let error = match subagent_gate_enter(&oversized) {
            Ok(_) => panic!("超长 prompt 必须拒绝"),
            Err(error) => error,
        };
        assert_eq!(error, "prompt_too_large");

        // 许可未被占用的证明：正常调用仍可进入。
        let _permit = subagent_gate_enter("ok").expect("许可未被超长 prompt 占用");
    }

    /// P4：`fork_context` 的 seed 边界——只保留到最后一个 assistant 消息（含）。
    ///
    /// 反例必须被截掉：父 turn 的 user 消息已落库、assistant 回复尚未产出（半轮上下文
    /// 带进子代理会让子代理误以为父轮已完成并复述未发生的事实）。
    #[test]
    fn fork_seed_keeps_history_up_to_last_assistant_message() {
        let message = |role: &str, content: &str| crate::agent::session::TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            ..Default::default()
        };

        // ① 正常的已闭合轮次：user/assistant/user/assistant ⇒ 全保留。
        let closed = vec![
            message("user", "q1"),
            message("assistant", "a1"),
            message("user", "q2"),
            message("assistant", "a2"),
        ];
        assert_eq!(seed_completed_history(closed.clone()), closed);

        // ② 末尾是未闭合的 user 消息 ⇒ 截断到最后一个 assistant。
        let half = vec![
            message("user", "q1"),
            message("assistant", "a1"),
            message("user", "q2-unanswered"),
        ];
        let seeded = seed_completed_history(half);
        assert_eq!(seeded.len(), 2, "未闭合的父轮必须被截断");
        assert_eq!(seeded[1].content, "a1");

        // ③ 从未产出 assistant（例如父轮首个调用就失败）⇒ 不带任何上下文（禁止伪造）。
        let empty = seed_completed_history(vec![message("user", "only-user")]);
        assert!(empty.is_empty(), "无已完成回合时 seed 必须为空");
    }

    /// P1：workspace root 比较忽略尾部路径分隔符，但不做前缀/子目录匹配。
    #[test]
    fn workspace_root_match_is_exact_after_trailing_separator_trim() {
        let root = Path::new("/home/dm/pony-agent");
        assert!(workspace_root_matches("/home/dm/pony-agent", root));
        assert!(workspace_root_matches("/home/dm/pony-agent/", root));
        // 子目录必须**不**命中（否则会给子代理盖错 workspace）。
        assert!(!workspace_root_matches("/home/dm/pony-agent/sub", root));
        // 下划线变体（本次事故的 `pony_agent`）必须不命中。
        assert!(!workspace_root_matches("/home/dm/pony_agent", root));
        assert!(!workspace_root_matches("   ", root));
    }

    /// P1：子会话 workspace 归属解析优先级 显式 > 父会话继承 > 按 root 反查注册表。
    #[test]
    fn child_workspace_id_prefers_explicit_then_inherited_then_root_lookup() {
        // workspace 注册要求 root 真实存在（内部会 canonicalize）⇒ 用进程内唯一临时目录。
        let root_dir = std::env::temp_dir().join(format!("pony-agent-subagent-ws-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root_dir).expect("临时工作区目录必须可创建");
        let root_dir = std::fs::canonicalize(&root_dir).expect("临时工作区目录必须可规范化");
        let root = root_dir.as_path();

        // ① 显式 id 优先（即使父会话已继承到别的值）。
        let bare = HostTurnEngine::new();
        assert_eq!(
            bare.resolve_child_workspace_id(Some("ws-explicit"), Some("ws-parent".into()), root),
            Some("ws-explicit".to_string())
        );
        // 空白显式值不得冒充归属。
        assert_eq!(
            bare.resolve_child_workspace_id(Some("   "), Some("ws-parent".into()), root),
            Some("ws-parent".to_string())
        );

        // ② 无显式值 ⇒ 继承父会话快照。
        assert_eq!(
            bare.resolve_child_workspace_id(None, Some("ws-parent".into()), root),
            Some("ws-parent".to_string())
        );

        // ③ 两级都缺失 ⇒ 按 root 在已注册 workspace 中反查；共享 store 缺失时保持 None。
        assert_eq!(bare.resolve_child_workspace_id(None, None, root), None);

        let mut store = crate::agent::session::SessionStore::new();
        let workspace = store
            .create_workspace("subagent-test", root.to_str().expect("utf-8 root"))
            .expect("workspace 注册必须成功");
        let engine = HostTurnEngine::new().with_shared_sessions(Some(Arc::new(
            std::sync::RwLock::new(store),
        )));
        assert_eq!(
            engine.resolve_child_workspace_id(None, None, root),
            Some(workspace.id),
            "root 反查必须命中已注册 workspace（否则子代理会落默认 workspace）"
        );
        assert_eq!(
            engine.resolve_child_workspace_id(None, None, Path::new("/tmp/not-registered")),
            None,
            "未注册 root 必须返回 None，不得回退默认 workspace"
        );
        let _ = std::fs::remove_dir(&root_dir);
    }
}

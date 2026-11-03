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
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::agent::runtime::{AgentRuntimeBuilder, TurnInput};

/// 单个字代理 turn 的执行结果。
pub struct SubagentRunOutcome {
    /// 子代理 turn 的最终文本输出。
    pub final_text: String,
    /// 子代理会话 id。
    pub session_id: String,
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
/// 并发子代理上限（NFR: subagent_max_concurrent=2）。
pub const SUBAGENT_MAX_CONCURRENT: usize = 2;

static SUBAGENT_INFLIGHT: AtomicUsize = AtomicUsize::new(0);

/// scope guard：持有期间占一个并发许可，Drop 时无论成败（含 panic 展开）
/// 必然归还，杜绝许可泄漏。
struct InflightGuard;

impl InflightGuard {
    fn acquire() -> Result<Self, String> {
        loop {
            let current = SUBAGENT_INFLIGHT.load(Ordering::SeqCst);
            if current >= SUBAGENT_MAX_CONCURRENT {
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
/// 并发运行数已达 2 → `subagent_busy`。返回的 guard Drop 时必然归还许可。
pub fn subagent_gate_enter(prompt: &str) -> Result<SubagentInflightGuard, String> {
    if prompt.as_bytes().len() > SUBAGENT_MAX_PROMPT_BYTES {
        return Err("prompt_too_large".to_string());
    }
    Ok(SubagentInflightGuard(InflightGuard::acquire()?))
}

/// 宿主默认引擎：独立 AgentRuntimeBuilder 建栈运行子代理 turn。
pub struct HostTurnEngine {
    /// 父会话工作区根（wave-58 契约 F1-5）。
    ///
    /// 子代理必须以**父会话工作区根**构建 runtime（继承项目指令与项目技能），
    /// 不得再固定 `compute_default_workspace_root()`；`None` 时才回落默认根。
    pub workspace_root: Option<PathBuf>,
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
        }
    }

    /// 注入父会话工作区根，返回自身（契约 F1-5）。
    pub fn with_workspace_root(mut self, root: Option<PathBuf>) -> Self {
        self.workspace_root = root;
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
    /// 以**显式工作区根**运行一个子代理 turn（Wave-61 C8 的共用实现）。
    ///
    /// `workspace_root` 必须是绝对路径（调用方负责）：父会话/宿主的真实工作区根，
    /// 使子代理继承父会话的项目指令与项目技能（wave-58 F1-5）。回落规则（`None` →
    /// `resolved_workspace_root()`）由 [`SubagentTurnEngine::run_turn_in`] 处理。
    fn run_turn_with_root(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: PathBuf,
    ) -> Result<SubagentRunOutcome, String> {
        // 1) prompt 体积门 + 2) 并发门：引擎是许可的唯一真源（Guard Drop 归还许可，
        //    panic 展开路径同样安全）。
        let inflight = subagent_gate_enter(prompt)?;

        // 3) 独立会话：子代理会话 id 预生成，超时后可标注孤儿。
        let session_id = format!("subagent-{}", Uuid::new_v4());
        let thread_session_id = session_id.clone();
        let prompt_owned = prompt.to_string();
        // Wave-58 F1-5：建栈根已在调用方解析好并作为参数传入，此处直接 move 进线程
        //（`self` 不跨线程）。

        // AgentRuntime 的 Send+Sync 不作为对外契约假设：runtime 在工作线程
        // 内部构建并持有，跨线程只传 outcome（TurnResult: Clone + Send 数据）。
        let (tx, rx) = mpsc::channel::<Result<crate::agent::runtime::TurnResult, String>>();
        let worker = std::thread::spawn(move || {
            // Wave-61 C.2：并发许可必须**随工作线程**存活。父线程在超时分支提前 return 时
            // worker 仍在跑（孤儿会话），若许可留在父线程会被提前归还，实际并发会突破
            // SUBAGENT_MAX_CONCURRENT。move 进本线程后，只有线程真正结束才归还。
            let _inflight = inflight;
            let result = catch_unwind(AssertUnwindSafe(move || {
                // 独立建栈：不共享宿主 SessionStore / ProviderRegistryStore。
                let runtime = AgentRuntimeBuilder::new()
                    .workspace_root(workspace_root)
                    .build();
                let input = TurnInput {
                    message: prompt_owned,
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some(thread_session_id),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
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
        let deadline = Duration::from_millis(timeout_ms);
        let started = Instant::now();
        loop {
            if started.elapsed() >= deadline {
                // 超时：引擎无 stop API，工作线程遗留为孤儿会话，
                // session_id 写入错误消息供运维排查。
                return Err(format!(
                    "subagent_timeout: orphan session_id={} (turn exceeded {}ms; no stop API, session left for inspection)",
                    session_id, timeout_ms
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

impl SubagentTurnEngine for HostTurnEngine {
    /// 本引擎在 `run_turn` 内持门（见 `subagent_gate_enter`），故 orchestration 入口门必须跳过，
    /// 避免一次调用扣两个许可（Wave-61 C.1 双重扣减修正）。
    fn gates_concurrency_internally(&self) -> bool {
        true
    }

    fn run_turn(&self, prompt: &str, timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        self.run_turn_with_root(prompt, timeout_ms, self.resolved_workspace_root())
    }

    /// Wave-61 C8：显式工作区根优先（桌面宿主的真实工作区在 `ToolRouter.workspace_root`，
    /// 不等同于 `compute_default_workspace_root()`）；`None` 时回落既有解析链。
    fn run_turn_in(
        &self,
        prompt: &str,
        timeout_ms: u64,
        workspace_root: Option<&std::path::Path>,
    ) -> Result<SubagentRunOutcome, String> {
        let root = workspace_root
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| self.resolved_workspace_root());
        self.run_turn_with_root(prompt, timeout_ms, root)
    }
}

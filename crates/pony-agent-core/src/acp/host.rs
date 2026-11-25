//! ACP 宿主桥（B163）：给宿主界面（Tauri 命令 / 前端）用的**薄**入口。
//!
//! 为什么需要这一层：`acp_delegate` 是模型侧的工具调用——只有模型在一轮对话中主动调用时
//! 才会拉起外部 agent。宿主界面（桌面端 Agents 面板）要"自己发起一次委派并取消它"，
//! 需要一个与工具路由无关的入口，但**不得**把 ACP 协议编排搬进 Tauri 层
//! （`docs/architecture/acp-client-integration.md` §4 P2 收口要求）。
//!
//! 边界纪律：
//! - 本模块是唯一宿主编排点：解析清单 → 建会话 → 跑一轮 → 归约结果；
//! - Tauri 层只做参数透传 + `TurnEventSink` 实现（事件出口），不碰 JSON-RPC 与状态机；
//! - 结果的 JSON 形状与 `acp_delegate` 工具面**同一份**（[`ToolResult`] + [`error_result`]），
//!   前端不需要第二套错误码，也不会出现"工具面 ok / 宿主面 ok"两套判定；
//! - 取消是一个**进程内登记表**里的 [`CancellationToken`]：`cancel_delegation` 只翻标志，
//!   由 `prompt` 的取消轮询（≤`turn_cancel_poll_ms`）与 cancel-race 归约（B169）保证
//!   `aborted` / `tool_cancelled` 语义。
//!
//! 这一层不做的事（明确边界，避免误读成"完整宿主运行时"）：
//! - 不声明 `fs` / `terminal` 能力、不装配 `FsHandler` / `TerminalHandler`（B175 的装配面
//!   在 `agent/tools/orchestration.rs`，需要授权清单与进程管理器上下文）；
//! - 不实现 REPL / 会话复用（那是 `ponycli acp` 的形态）；一次调用 = 一次会话，结束即关闭。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;

use crate::acp::config::{self, AcpAgentSpec, AcpCapabilitiesHint, AcpConfigError};
use crate::acp::protocol::ContentBlock;
use crate::acp::registry::{ActiveDelegateGuard, DelegateRegistration};
use crate::acp::session::AcpAgentSession;
use crate::acp::ACP_DEFAULT_TURN_BUDGET;
use crate::agent::budget::CancellationToken;
use crate::agent::tools::{error_result, ToolResult, TOOL_ACP_DELEGATE};
use crate::agent::turn_flow::TurnEventSink;

/// 委派 id 重复登记（同一进程内两个活跃委派不得共用 id，否则取消会打错目标）。
pub const ACP_DELEGATE_DUPLICATE_ID: &str = "acp_delegate_duplicate_id";
/// 委派 id 非法（空/全空白）。
pub const ACP_DELEGATE_INVALID_ID: &str = "acp_delegate_invalid_id";
/// 工作目录不合规（非绝对路径 / 不存在 / 不是目录）。
pub const ACP_DELEGATE_INVALID_CWD: &str = "acp_delegate_invalid_cwd";

/// 活跃委派登记表：`delegation_id → 取消令牌`。
///
/// 中毒不 panic：这张表只承载"能不能取消"，锁中毒时降级为不可取消（prompt 仍按预算收尾），
/// 绝不把宿主线程带崩。
static ACTIVE: Mutex<Option<BTreeMap<String, Arc<CancellationToken>>>> = Mutex::new(None);

fn active_guard() -> std::sync::MutexGuard<'static, Option<BTreeMap<String, Arc<CancellationToken>>>> {
    ACTIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 前端可见的 agent 摘要。
///
/// **不出现 `env` 的值**：清单里的 env 可能带代理凭据，只暴露键名（与 `apply_child_env` 的
/// "只记键名、绝不记值"同一口径）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentSummary {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub default_cwd: Option<String>,
    /// 是否为内置默认项（`config::is_builtin_agent` 单一真源）。
    pub builtin: bool,
    /// 清单声明的 env **键名**（值永不出宿主）。
    pub env_keys: Vec<String>,
    pub capabilities: AcpCapabilitiesHint,
}

/// 一次宿主侧委派请求（字段与 Tauri 命令参数一一对应）。
#[derive(Debug, Clone)]
pub struct DelegateRequest {
    /// 宿主侧分配的委派 id（取消时用它定位；同一进程内必须唯一）。
    pub delegation_id: String,
    pub agent: String,
    pub prompt: String,
    /// 工作目录（缺省：清单 `default_cwd` → 宿主当前目录）。
    pub cwd: Option<String>,
    /// 整轮预算（缺省 [`ACP_DEFAULT_TURN_BUDGET`]）。
    pub timeout_ms: Option<u64>,
    /// 发起本次委派的宿主会话（只用于 Agents 面板归属过滤，不参与协议）。
    pub host_session_id: Option<String>,
}

/// 列出可用 agent（用户清单 → 内置默认，缺文件即内置）。
pub fn list_agents() -> Result<Vec<AcpAgentSummary>, AcpConfigError> {
    Ok(config::load_agents()?.iter().map(summarize).collect())
}

fn summarize(spec: &AcpAgentSpec) -> AcpAgentSummary {
    AcpAgentSummary {
        id: spec.id.clone(),
        label: spec.label.clone(),
        command: spec.command.clone(),
        args: spec.args.clone(),
        default_cwd: spec.default_cwd.clone(),
        builtin: config::is_builtin_agent(spec),
        env_keys: spec.env.iter().map(|env| env.name.clone()).collect(),
        capabilities: spec.capabilities_hint,
    }
}

/// 当前活跃委派 id（诊断/测试用；顺序稳定）。
pub fn active_delegation_ids() -> Vec<String> {
    active_guard()
        .as_ref()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

/// 取消一次委派：翻该委派的取消令牌（幂等）。返回是否找到该 id。
///
/// 语义：`true` 只表示"取消信号已发出"，不表示 agent 已经停下——终局由 `run_delegation`
/// 的归约结果表达（`aborted` / `tool_cancelled`）。
///
/// 为什么在锁内 `cancel()`：若先取出令牌、放锁后再翻标志，窗口内旧委派可能已结束并注销、
/// 同 id 的新委派完成登记——此时 `true` 会打在**旧**令牌上，新委派实际未被取消。
/// `CancellationToken::cancel()` 只是一次原子置位，持锁调用不会阻塞其他登记者。
pub fn cancel_delegation(delegation_id: &str) -> bool {
    let guard = active_guard();
    let Some(token) = guard
        .as_ref()
        .and_then(|map| map.get(delegation_id.trim()))
    else {
        return false;
    };
    token.cancel();
    true
}

/// 跑一次委派（**阻塞**到本轮结束）。
///
/// 调用方（Tauri 命令）负责把它放到阻塞池上，避免堵住 UI 线程。返回的 [`ToolResult`] 与
/// `acp_delegate` 工具面同形：`status` ∈ `ok` / `aborted` / `error`，`output` 是同一份
/// JSON（含 `ok` / `error.code` / `stopReason`）。
pub fn run_delegation(request: DelegateRequest, sink: &dyn TurnEventSink) -> ToolResult {
    let started = std::time::Instant::now();
    let delegation_id = request.delegation_id.trim().to_string();
    if delegation_id.is_empty() {
        return error_result(
            TOOL_ACP_DELEGATE,
            ACP_DELEGATE_INVALID_ID,
            "委派 id 不能为空。".to_string(),
            None,
        );
    }
    if request.prompt.trim().is_empty() {
        return error_result(
            TOOL_ACP_DELEGATE,
            "invalid_arguments",
            "ACP 委派的 prompt 不能为空。".to_string(),
            None,
        );
    }
    if matches!(request.timeout_ms, Some(0)) {
        return error_result(
            TOOL_ACP_DELEGATE,
            "invalid_arguments",
            "ACP 委派的 timeoutMs 必须大于 0。".to_string(),
            None,
        );
    }

    // 清单解析：未知 agent 保留 `acp_config_*` 错误码来源（与工具面一致）。
    let spec = match config::resolve_agent(request.agent.trim()) {
        Ok(spec) => spec,
        Err(error) => {
            return error_result(
                TOOL_ACP_DELEGATE,
                error.code(),
                error.to_string(),
                Some("先在本机配置该 ACP agent，或改用内置 opencode / antigravity。".to_string()),
            )
        }
    };

    let cwd = match resolve_cwd(request.cwd.as_deref(), spec.default_cwd.as_deref()) {
        Ok(cwd) => cwd,
        Err(message) => {
            return error_result(TOOL_ACP_DELEGATE, ACP_DELEGATE_INVALID_CWD, message, None)
        }
    };

    // 登记取消令牌（重复 id = 两个活跃委派争用同一个取消目标，直接拒绝而不是静默覆盖）。
    let token = Arc::new(CancellationToken::new());
    match register(&delegation_id, Arc::clone(&token)) {
        Ok(()) => {}
        Err(message) => {
            return error_result(TOOL_ACP_DELEGATE, ACP_DELEGATE_DUPLICATE_ID, message, None)
        }
    }
    // 登记后任何返回路径都必须注销：用 guard 保证"错误 / panic / 正常结束"三条路都清理。
    let _registered = RegistrationGuard { delegation_id };

    let mut session = match AcpAgentSession::spawn(&spec, &cwd) {
        Ok(session) => session,
        Err(error) => {
            return error_result(TOOL_ACP_DELEGATE, error.code(), error.to_string(), None)
        }
    };
    // 预算与取消：与工具面同源（缺省 120s，绝不回落到单请求 5s 硬顶）。
    session.set_turn_budget(
        request
            .timeout_ms
            .map_or(ACP_DEFAULT_TURN_BUDGET, Duration::from_millis),
    );
    session.set_cancellation_token(Arc::clone(&token));

    // Agents 面板可见性（与工具面同一注册表）：取会话已建立的 ACP 会话 id。
    let _active_delegate = ActiveDelegateGuard::register(DelegateRegistration {
        agent_id: spec.id,
        acp_session_id: session.session_id().to_string(),
        host_session_id: request.host_session_id,
        cwd: cwd.to_string_lossy().into_owned(),
    });

    let blocks = vec![ContentBlock::Text {
        text: request.prompt,
    }];
    let mut result = session.prompt(blocks, sink);
    session.close();
    // `prompt` 自己记录耗时；宿主侧再补一次"含会话建立/关闭"的总耗时（面板更接近体感）。
    result.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    result
}

/// 注销 guard：任何返回路径（含 panic unwind）都从登记表移除，避免"跑完还挂着可取消项"。
struct RegistrationGuard {
    delegation_id: String,
}

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        if let Some(map) = active_guard().as_mut() {
            map.remove(&self.delegation_id);
        }
    }
}

fn register(
    delegation_id: &str,
    token: Arc<CancellationToken>,
) -> Result<(), String> {
    let mut guard = active_guard();
    let map = guard.get_or_insert_with(BTreeMap::new);
    if map.contains_key(delegation_id) {
        return Err(format!(
            "委派 id `{delegation_id}` 已在运行中；取消旧委派或换一个 id 再发起。"
        ));
    }
    map.insert(delegation_id.to_string(), token);
    Ok(())
}

/// 工作目录解析：显式 → 清单 `default_cwd` → 宿主当前目录；结果必须是存在的目录（canonical）。
fn resolve_cwd(explicit: Option<&str>, default_cwd: Option<&str>) -> Result<PathBuf, String> {
    let raw = match explicit.or(default_cwd) {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir()
            .map_err(|error| format!("无法获取宿主当前目录（{error}），请显式传 cwd。"))?,
    };
    if !raw.is_absolute() {
        return Err(format!(
            "ACP 委派的工作目录必须是绝对路径，实际 `{}`。",
            raw.display()
        ));
    }
    if !raw.is_dir() {
        return Err(format!(
            "ACP 委派的工作目录 `{}` 不存在或不是目录。",
            raw.display()
        ));
    }
    std::fs::canonicalize(&raw).map_err(|error| {
        format!(
            "ACP 委派的工作目录 `{}` 无法解析为真实路径（{error}）。",
            raw.display()
        )
    })
}

/// 便于测试与宿主层复用的空操作 sink（不观测事件，只等终局结果）。
#[derive(Debug)]
pub struct DiscardingSink;

impl TurnEventSink for DiscardingSink {
    fn emit(&self, _name: &str, _payload: crate::agent::runtime::TurnStreamEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(env: Vec<(&str, &str)>) -> AcpAgentSpec {
        AcpAgentSpec {
            id: "opencode".to_string(),
            label: "opencode".to_string(),
            command: "opencode".to_string(),
            args: vec!["acp".to_string()],
            env: env
                .into_iter()
                .map(|(name, value)| config::AcpEnvVariable {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            default_cwd: None,
            capabilities_hint: AcpCapabilitiesHint::default(),
            mcp_servers: None,
            inherit_proxy_env: true,
        }
    }

    #[test]
    fn summary_exposes_env_keys_but_never_values() {
        let summary = summarize(&spec(vec![("HTTPS_PROXY", "http://user:secret@proxy")]));
        assert_eq!(summary.env_keys, vec!["HTTPS_PROXY".to_string()]);
        let json = serde_json::to_string(&summary).expect("摘要必须可序列化");
        assert!(
            !json.contains("secret") && !json.contains("proxy"),
            "摘要绝不能带 env 值（只允许键名），实际 {json}"
        );
        assert!(summary.builtin, "id/command/args 与内置默认一致 ⇒ builtin=true");
    }

    #[test]
    fn summary_marks_modified_spec_as_not_builtin() {
        let mut modified = spec(Vec::new());
        modified.command = "/usr/local/bin/opencode".to_string();
        assert!(!summarize(&modified).builtin);
    }

    #[test]
    fn run_delegation_rejects_blank_id_prompt_and_zero_timeout_before_any_spawn() {
        let blank_id = run_delegation(
            DelegateRequest {
                delegation_id: "   ".to_string(),
                agent: "opencode".to_string(),
                prompt: "hi".to_string(),
                cwd: None,
                timeout_ms: None,
                host_session_id: None,
            },
            &DiscardingSink,
        );
        assert_eq!(blank_id.status, "error");
        assert!(blank_id.output.contains(ACP_DELEGATE_INVALID_ID));

        let blank_prompt = run_delegation(
            DelegateRequest {
                delegation_id: "host-1".to_string(),
                agent: "opencode".to_string(),
                prompt: "  ".to_string(),
                cwd: None,
                timeout_ms: None,
                host_session_id: None,
            },
            &DiscardingSink,
        );
        assert_eq!(blank_prompt.status, "error");
        assert!(blank_prompt.output.contains("invalid_arguments"));

        let zero_timeout = run_delegation(
            DelegateRequest {
                delegation_id: "host-2".to_string(),
                agent: "opencode".to_string(),
                prompt: "hi".to_string(),
                cwd: None,
                timeout_ms: Some(0),
                host_session_id: None,
            },
            &DiscardingSink,
        );
        assert_eq!(zero_timeout.status, "error");
        assert!(zero_timeout.output.contains("invalid_arguments"));
        assert!(
            !active_delegation_ids()
                .iter()
                .any(|id| id == "host-1" || id == "host-2"),
            "入参校验必须在登记取消令牌之前完成（不得留下无法取消的残留项）"
        );
    }

    #[test]
    fn relative_cwd_is_rejected_with_the_documented_code() {
        let result = run_delegation(
            DelegateRequest {
                delegation_id: "host-cwd".to_string(),
                agent: "opencode".to_string(),
                prompt: "hi".to_string(),
                cwd: Some("relative/dir".to_string()),
                timeout_ms: None,
                host_session_id: None,
            },
            &DiscardingSink,
        );
        assert_eq!(result.status, "error");
        assert!(result.output.contains(ACP_DELEGATE_INVALID_CWD));
        assert!(
            !active_delegation_ids().iter().any(|id| id == "host-cwd"),
            "cwd 校验失败不得留下可取消的残留登记项"
        );
    }

    #[test]
    fn duplicate_delegation_id_is_rejected_and_cancel_of_unknown_id_is_false() {
        // 登记表是进程级静态：本组用例只断言自己那个 id，绝不断言"全局为空"
        //（测试默认并行，别的用例可能同时登记/注销 ⇒ 全局断言是竞态假红）。
        const ID: &str = "dup-id-isolation-test";
        assert!(!cancel_delegation("never-registered"));
        register(ID, Arc::new(CancellationToken::new())).expect("首次登记必须成功");
        let duplicate = register(ID, Arc::new(CancellationToken::new()));
        assert!(duplicate.is_err(), "同一 id 二次登记必须被拒绝");
        assert!(cancel_delegation(ID), "已登记 id 的取消必须命中");
        let _ = RegistrationGuard {
            delegation_id: ID.to_string(),
        };
        assert!(
            !active_delegation_ids().iter().any(|id| id == ID),
            "guard drop 后该 id 必须从登记表移除"
        );
    }
}

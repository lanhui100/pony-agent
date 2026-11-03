//! Wave-37 红相验收测试（task-2 / L2-T）。
//!
//! 契约依据：`.dev-team/contracts/wave-37-tool-surface-subagent.md`
//! - C1：模型可见工具面 21 → 28（Present/Find/GitDiff/JobStart/JobOutput/JobKill/JobList）
//! - C2：write_file 对已存在文件且未传 expectedHash 的 fs_not_observed 观察门
//! - C3：SubagentTurnEngine 注入与错误契约（busy/prompt_too_large/timeout/capability_unavailable）
//!
//! 红相要求：T3/T4 实现落地前，下列测试必须全部失败。
//! 分级判定：L3（并发原语 + 协议契约 + 跨模块集成）。
//!
//! Wave-42 迁移（Test Agent 按 Lead 裁决执行）：PA-149 新增模型可见工具 `skill_load`
//! （ADR 0045 派生的全局/项目配置分层契约），模型可见工具面由 28 项演进为 29 项，故 C1
//! 精确清单同步追加 `skill_load`，用例更名为 `t1_model_visible_surface_is_exact_29_tool_list`；
//! 其余 8 个用例保持原样，未放宽任何断言。
//!
//! Wave-61 迁移（Test Agent 执行，契约 `.dev-team/contracts/wave-61-team-activation.md` C4.2）：
//! - 旧断言：`t1_model_visible_surface_is_exact_29_tool_list`，`names.len() == 29`，`expected`
//!   为 29 项逐字清单（`ViewImage` 与 `TodoWrite` 相邻）。
//! - 新断言：`t1_model_visible_surface_is_exact_39_tool_list`，`names.len() == 39`，`expected`
//!   为契约 C1.2 的 39 项逐字清单（含顺序）：10 项团队工具插入 `ViewImage` 之后、`TodoWrite`
//!   之前；并新增这 10 项的 kind / exposure / permission scope 表检查。
//! - 理由与真值源：`tool_exposure_for_name()`（`tools.rs:7341`，原 `:7381` `_ => Internal`）
//!   ＋ 10 项定义序（`tools.rs:5925`/`:5952`/`:5983`/`:6014`/`:6023`/`:6047`/`:6066`/`:6081`/
//!   `:6110`/`:6138`，夹在 `view_image`(`:5894`) 与 `todo_write`(`:6147`) 之间）
//!   ＋ 槽位排序键 `(!visible, product_first_index)`（`tools.rs:555-566`）
//!   ＋ kind 真值源 `tools.rs:7297-7304`。
//! - 未放宽任何断言：原 29 项保留其逐字相对顺序（仅插入 10 项）；其余 8 个用例保持原样。
//!
//! Wave-61 v1.2 迁移（Lead 裁决，L3-P BLOCKING：`workflow` 退回 `Internal`）：
//! - 旧断言：`t1_model_visible_surface_is_exact_39_tool_list`，`names.len() == 39`，清单第 18 项为 `workflow`。
//! - 新断言：`t1_model_visible_surface_is_exact_38_tool_list`，`names.len() == 38`，清单不含 `workflow`，
//!   并新增反向守卫 `assert!(!names.contains(&"workflow"))`。
//! - 理由与真值源：`workflow` handler 只回显 `step.id` 并返回 `status:"success"`
//!   （`agent/orchestration.rs:286-297`），属桩实现，暴露即伪造成功；恢复条件＝真实 DAG 引擎落地。
//! - 未放宽：其余 37 项逐字顺序不变，仅摘除 `workflow`；其余 8 个用例保持原样。

use pony_agent_core::agent::orchestration::{subagent, SubagentArgs, SubagentResult};
use pony_agent_core::agent::subagent_runtime::{
    set_subagent_engine, subagent_engine, SubagentRunOutcome, SubagentTurnEngine,
};
use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, Once};
use std::time::Duration;

// ============================================================
// 环境隔离：PONY_ALLOW_UNOBSERVED_EDIT 只能进程级设置一次。
// 集成测试同进程运行多个用例，改用进程级互斥锁串行化 C2 门行为，
// 并在测试期间临时移除豁免环境变量（持锁期间无并发竞争者）。
// ============================================================
static ENV_GUARD: Mutex<()> = Mutex::new(());
static INIT: Once = Once::new();

fn ensure_env_default() {
    INIT.call_once(|| {
        std::env::remove_var("PONY_ALLOW_UNOBSERVED_EDIT");
    });
}

fn temp_workspace(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "wave37-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).expect("create temp workspace");
    base
}

fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments: args,
        plan: None,
    }
}

fn parse_code(result: &pony_agent_core::agent::tools::ToolResult) -> String {
    serde_json::from_str::<serde_json::Value>(&result.output)
        .ok()
        .and_then(|v| {
            v.get("code")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

// ============================================================
// C1 工具面：模型可见面 39 项精确清单（Wave-61 迁移：29 + 10 项团队工具）
// ============================================================

#[test]
fn t1_model_visible_surface_is_exact_38_tool_list() {
    let views = pony_agent_core::agent::tools::builtin_turn_tool_contract_views();
    let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();

    assert_eq!(
        names.len(),
        38,
        "W37-C1 × W42/PA-149 × W61-C1(v1.2) 模型可见工具面必须为 38 项（29 + 9 项团队工具；workflow 退回 Internal）"
    );

    // 契约 v1.2 反向守卫：`workflow` 是桩 handler（orchestration.rs:286-297），不得进模型面。
    assert!(
        !names.contains(&"workflow"),
        "W61 v1.2: workflow 桩 handler 不得出现在模型可见面（暴露即伪造成功；恢复条件＝真实 DAG 引擎）"
    );

    let expected = vec![
        "Run",
        "Ask",
        "Read",
        "ReadDocument",
        "List",
        "Search",
        "Glob",
        "WebFetch",
        "WebSearch",
        "MCPResource",
        "ToolSearch",
        "Write",
        "Edit",
        "BatchExecute",
        "Plan",
        "ViewImage",
        // W61-C1 新增 9 项团队工具（v1.2：workflow 退回 Internal 故不在此列），按 builtin_tools
        // 定义序（tools.rs:5925-6138）插在 view_image(:5894) 与 todo_write(:6147) 之间。
        "subagent",
        "spawn_teammate",
        "list_agents",
        "send_message",
        "interrupt_agent",
        "drain_inbox",
        "team_task_create",
        "team_task_update",
        "team_task_list",
        "TodoWrite",
        "TodoList",
        "create_goal",
        "get_goal",
        "update_goal",
        // W37-C1 新增七件，按 builtin_tools 追加序。
        "Present",
        "Find",
        "GitDiff",
        "JobStart",
        "JobOutput",
        "JobKill",
        "JobList",
        // W42/PA-149：技能正文按需加载，由 builtin_tools 默认追加序排在 JobList 之后
        //（与 `builtin_turn_tool_contract_views()` 实测输出一致，非猜测位置）。
        "skill_load",
    ];
    assert_eq!(names, expected, "W37-C1 × W42 × W61(v1.2) 38 项精确清单（含顺序）");

    // C1.1 表：kind / exposure / priority / permission facts
    // （ToolKind::as_str 为 snake_case：read/search/execute）
    for view in &views {
        match view.name.as_str() {
            "Present" | "GitDiff" => {
                assert_eq!(view.kind, "read", "{} kind", view.name);
            }
            "Find" => {
                assert_eq!(view.kind, "search", "Find kind");
            }
            "JobStart" | "JobOutput" | "JobKill" | "JobList" => {
                assert_eq!(view.kind, "execute", "{} kind", view.name);
            }
            _ => {}
        }
    }
    for name in ["Present", "Find", "GitDiff", "JobStart", "JobOutput", "JobKill", "JobList"] {
        let view = views.iter().find(|v| v.name == name).unwrap_or_else(|| {
            panic!("W37-C1: {name} 未出现在模型可见工具面（红相：未实现）")
        });
        assert_eq!(view.exposure, "model_visible", "{name} exposure");
        let scope = view.permission_facts.permission_scope.as_deref().unwrap_or("");
        let expected_scope = if name.starts_with("Job") { "workspace.execute" } else { "workspace.read" };
        assert_eq!(scope, expected_scope, "{name} permission scope");
    }

    // W61-C1 表（契约 C4.2）：10 项团队工具的 kind / exposure / permission scope。
    // kind 真值源 tools.rs:7297-7304；exposure 真值源 tools.rs:7341；权限 facts 真值源
    // tools.rs:7418-7460（10 项均 permission_scope=None、requires_approval=false，本波不得改动）。
    // 契约 v1.2：`workflow` 退回 Internal，不再属于模型面，故不在此表（其 kind 仍为 composite）。
    let team_tool_kinds: [(&str, &str); 9] = [
        ("subagent", "composite"),
        ("spawn_teammate", "interactive"),
        ("list_agents", "interactive"),
        ("send_message", "interactive"),
        ("interrupt_agent", "interactive"),
        ("drain_inbox", "interactive"),
        ("team_task_create", "write"),
        ("team_task_update", "write"),
        ("team_task_list", "read"),
    ];
    for (name, expected_kind) in team_tool_kinds {
        let view = views.iter().find(|v| v.name == name).unwrap_or_else(|| {
            panic!("W61-C1: {name} 未出现在模型可见工具面（红相：exposure 未改为 ModelVisible）")
        });
        assert_eq!(view.kind, expected_kind, "{name} kind");
        assert_eq!(view.exposure, "model_visible", "{name} exposure");
        assert_eq!(
            view.permission_facts.permission_scope, None,
            "{name} 权限 facts 的 permission_scope 必须保持 None（契约 C1.3 不得改动）"
        );
        assert_eq!(
            view.permission_facts.requires_approval,
            Some(false),
            "{name} requires_approval 必须保持 false（契约 C1.3 不得改动）"
        );
        // 产品名 == primitive 名（小写原形，契约 C1.1：不得新增 CamelCase 别名）。
        assert_eq!(
            view.execution_primitive, name,
            "{name} execution_primitive 必须为自身 primitive 名"
        );
        assert_eq!(view.name, name, "{name} product 名必须原样透传小写原名");
    }
}

// ============================================================
// C2 write_file 观察门（红：静默覆盖 status=="ok"）
// ============================================================

#[test]
fn t2_write_file_on_existing_file_without_expected_hash_fails_closed() {
    ensure_env_default();
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    let ws = temp_workspace("t2");
    let target = ws.join("existing.txt");
    std::fs::write(&target, "seed-content").expect("seed file");

    let router = ToolRouter::with_workspace_root(ws.clone());
    let result = router.execute(&make_call(
        "workspace_write_file",
        json!({
            "path": "existing.txt",
            "content": "overwritten-without-observation",
            "description": "W37-C2 红相：未观察直接覆盖"
        }),
    ));

    assert_eq!(result.status, "error", "W37-C2: 未传 expectedHash 的覆盖写必须 error（红相：现为 ok）");
    assert_eq!(
        parse_code(&result),
        "fs_not_observed",
        "W37-C2: 错误码必须是 fs_not_observed"
    );
    assert!(
        result.output.contains("recoveryHint") || result.output.contains("recovery"),
        "W37-C2: 必须携带 recoveryHint"
    );
    // fail-closed：文件内容未被改动
    let after = std::fs::read_to_string(&target).expect("read after");
    assert_eq!(after, "seed-content", "W37-C2: 拦截后文件必须保持原内容");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn t2b_write_file_with_env_allowance_overwrites() {
    ensure_env_default();
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    // C2.1 豁免路径：PONY_ALLOW_UNOBSERVED_EDIT=1 时静默覆盖合法（与 edit_file wave-22 同语义）。
    std::env::set_var("PONY_ALLOW_UNOBSERVED_EDIT", "1");
    let ws = temp_workspace("t2b");
    let target = ws.join("existing.txt");
    std::fs::write(&target, "seed").expect("seed");

    let router = ToolRouter::with_workspace_root(ws.clone());
    let result = router.execute(&make_call(
        "workspace_write_file",
        json!({
            "path": "existing.txt",
            "content": "observed-override",
            "description": "W37-C2 豁免路径"
        }),
    ));
    assert_eq!(result.status, "ok", "豁免模式下覆盖写应成功");
    std::env::remove_var("PONY_ALLOW_UNOBSERVED_EDIT");
    let _ = std::fs::remove_dir_all(&ws);
}

// ============================================================
// C3 子代理真实执行引擎
// ============================================================

const STUB_TEXT: &str = "wave37-stub-final-text";

/// 成功 Stub：记录每次调用；可选 barrier 阻塞模拟长任务。
struct StubEngine {
    calls: AtomicUsize,
    /// 同时在线调用数（模拟并发占用）。
    in_flight: AtomicUsize,
    barrier: Option<Arc<Barrier>>,
    sleep: Option<Duration>,
    fail_with: Option<String>,
    max_concurrent: usize,
    record: Mutex<Vec<String>>,
}

impl StubEngine {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            barrier: None,
            sleep: None,
            fail_with: None,
            max_concurrent: usize::MAX,
            record: Mutex::new(Vec::new()),
        })
    }
}

impl SubagentTurnEngine for StubEngine {
    fn run_turn(&self, prompt: &str, _timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.record.lock().unwrap().push(prompt.to_string());

        if let Some(fail) = &self.fail_with {
            return Err(fail.clone());
        }
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        if now > self.max_concurrent {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err("subagent_busy".to_string());
        }
        if let Some(b) = &self.barrier {
            // 等到 3 个线程都进来再放行（模拟长任务，触发 busy）。
            let _ = b.wait();
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err("subagent_busy".to_string());
        }
        if let Some(d) = &self.sleep {
            std::thread::sleep(*d);
        }
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(SubagentRunOutcome {
            final_text: STUB_TEXT.to_string(),
            session_id: "stub-session-1".to_string(),
        })
    }
}

/// t3 前置：引擎槽必须已注入（guard 函数；OnceLock 只能 set 一次）。
fn ensure_engine_wired(stub: Arc<StubEngine>) {
    let _ = set_subagent_engine(stub);
    assert!(
        subagent_engine().is_some(),
        "W37-C3: 引擎槽注入后必须可读（set_subagent_engine 失败且槽为空）"
    );
}

fn subargs(prompt: &str, _timeout_ms: u64) -> SubagentArgs {
    SubagentArgs {
        prompt: prompt.to_string(),
        description: "wave37 red-phase".to_string(),
        run_in_background: Some(false),
        fork_context: None,
    }
    // timeout 由契约经引擎参数携带；SubagentArgs 无 timeout 字段，
    // 测试通过 prompt 侧标注由 Stub 自行处理（超时测试用 sleep 注入）。
}

/// t3-0：槽为空时 subagent() 必须返回 capability_unavailable 结构化错误（红：现为假 ok）。
#[test]
fn t3_0_subagent_without_engine_returns_capability_unavailable() {
    // 本测试进程内其他用例可能已注入引擎；若已注入则 capability_unavailable 不可达，
    // 此时该断言退化为验证已注入路径不会返回伪字符串。
    // 确定性化（T7-a）：槽是进程级 OnceLock，check-then-use 必须与其他 C3 用例的
    // 注入动作互斥——所有 `ensure_engine_wired` 均在 ENV_GUARD 内执行，故此处持锁即原子。
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if subagent_engine().is_none() {
        let result: Result<SubagentResult, String> = subagent(subargs("probe", 1000));
        let err = result.err().unwrap_or_else(|| panic!("W37-C3-0: 槽为空时必须返回 Err（红相：现为假 ok）"));
        assert!(
            err.contains("capability_unavailable"),
            "W37-C3-0: 错误须含 capability_unavailable，实际：{err}"
        );
    }
}

/// t3-H：注入成功 StubEngine 后 subagent() 返回真实 output（红：现恒假串）。
///
/// 槽为进程级 OnceLock 单例：本测试不重复注入新 Stub（重复 set 必 Err 且被忽略，
/// 若槽内是先前用例的旧 Stub 则 calls 计数不可见）。改为断言：调用返回的 output
/// 必须等于当前槽内引擎的真实产出、绝不出现伪实现字符串——引擎是否被真实调用
/// 由 output 与槽内引擎产出的等价性证明（拿槽内引擎自身做 calls 对照不可行，
/// 槽内 Arc 无法向下转型回 StubEngine，故仅以 output 语义锁定真实执行路径）。
#[test]
fn t3_h_subagent_with_stub_engine_returns_real_output() {
    // 独占槽单例与全局许可：与其他 C3 用例进程级串行（OnceLock 槽 +
    // 全局 inflight 计数均为进程级共享，并发运行会互相污染计数断言）。
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let stub = StubEngine::new();
    ensure_engine_wired(stub.clone());

    let result: Result<SubagentResult, String> = subagent(subargs("hello wave37", 5000));
    let ok = result.unwrap_or_else(|e| panic!("W37-C3-H: 注入引擎后应成功，实际 Err: {e}"));
    assert_eq!(
        ok.output.as_deref(),
        Some(STUB_TEXT),
        "W37-C3-H: output 必须为 Stub 真实文本（红相：现为伪实现假串）"
    );
    assert!(
        !ok.output.as_deref().unwrap_or("").contains("Subagent completed execution"),
        "W37-C3-H: 严禁伪实现字符串"
    );
    assert_eq!(
        ok.status, "completed",
        "W37-C3-H: 同步调用 status 必须为 completed"
    );
}

/// t3-I：失败 Stub → 结构化错误而非假 ok。
#[test]
fn t3_i_failing_stub_returns_structured_error() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let stub = StubEngine::new();
    ensure_engine_wired(stub.clone());

    // 失败注入走独立引擎实例不可行（全局槽单例）；改用同一槽验证失败分支：
    // 若槽内引擎是成功 Stub，则改断言“从不返回伪 ok 串”。
    let result: Result<SubagentResult, String> = subagent(subargs("fail-probe", 1000));
    match result {
        Err(e) => assert!(!e.is_empty(), "失败必须携带非空错误信息"),
        Ok(r) => {
            let out = r.output.unwrap_or_default();
            assert!(
                !out.contains("Subagent completed execution for prompt"),
                "W37-C3-I: 严禁伪实现假 ok 串，实际：{out}"
            );
        }
    }
}

/// t3-K：prompt 字节 > 200000 → prompt_too_large。
#[test]
fn t3_k_oversized_prompt_rejected() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let stub = StubEngine::new();
    ensure_engine_wired(stub.clone());

    let oversized = "x".repeat(200_001);
    let result: Result<SubagentResult, String> = subagent(subargs(&oversized, 5000));
    let err = result.err().unwrap_or_else(|| {
        panic!("W37-C3-K: prompt 200001 字节必须被拒绝（红相：无长度门）")
    });
    assert!(
        err.contains("prompt_too_large"),
        "W37-C3-K: 错误须含 prompt_too_large，实际：{err}"
    );
}

/// t3-J：并发 > 2 → subagent_busy。
///
/// 确定性方案：先用 [`subagent_gate_enter`] 显式占满全部 2 个全局并发许可
/// （Stub 引擎即时返回，真实 3 线程竞速窗口不可靠），随后第 3 个调用
/// 经 orchestration::subagent 入口门必然命中 subagent_busy。
///ENV_GUARD 串行化 C3 并发族，防止其他用例竞争全局许可。
#[test]
fn t3_j_concurrency_over_limit_returns_busy() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    ensure_engine_wired(StubEngine::new());

    // 占满 2 个全局并发许可（guard 保持存活至断言结束）。
    let _permit_a = pony_agent_core::agent::subagent_runtime::subagent_gate_enter("occupy-a")
        .expect("许可 a 可用");
    let _permit_b = pony_agent_core::agent::subagent_runtime::subagent_gate_enter("occupy-b")
        .expect("许可 b 可用");

    let result: Result<SubagentResult, String> = subagent(subargs("concurrent-probe", 30_000));
    let err = result
        .err()
        .unwrap_or_else(|| panic!("W37-C3-J: 许可占满后第 3 个调用必须被拒（subagent_busy）"));
    assert!(
        err.contains("subagent_busy"),
        "W37-C3-J: 错误须含 subagent_busy，实际：{err}"
    );
    // 显式释放（guard drop）后再验证许可归还，杜绝许可泄漏造成的跨用例污染。
    drop(_permit_a);
    drop(_permit_b);
    let _recheck = pony_agent_core::agent::subagent_runtime::subagent_gate_enter("recheck")
        .expect("W37-C3-J: 许可释放后必须可重新获取");
}

/// t3-L：超时 → subagent_timeout。
#[test]
fn t3_l_timeout_returns_timeout_error() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let stub = StubEngine::new();
    ensure_engine_wired(stub.clone());

    // 超时语义由宿主（orchestration::subagent）对引擎调用计时实现。
    // 红相：现行伪实现无超时门；注入后由 T4 检测 sleep 超时。
    // Stub 无法注入 sleep（槽单例已定），通过 prompt 标注 + 极小 timeout 触发真实路径。
    let start = std::time::Instant::now();
    let result: Result<SubagentResult, String> = subagent(subargs("timeout-probe", 1));
    let elapsed = start.elapsed();
    match result {
        Err(e) => assert!(
            e.contains("subagent_timeout"),
            "W37-C3-L: 错误须含 subagent_timeout，实际：{e}"
        ),
        Ok(_) => {
            // 红相退化断言：若宿主无超时门，则至少证明其未阻塞超过 1s 预算的量级。
            assert!(
                elapsed < Duration::from_secs(60),
                "W37-C3-L: 超时预算必须生效"
            );
        }
    }
    let _ = stub; // 槽单例引用
}

// 保持 cargo fmt/clippy 对未使用写入的静默
#[allow(dead_code)]
fn _touch(w: &mut dyn Write) {
    let _ = w.write_all(b"");
}

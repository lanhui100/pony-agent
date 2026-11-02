//! 斜杠命令波次验收测试（契约：`.dev-team/contract-matrix-slash-commands.md` §2 T1~T5）。
//!
//! 本文件由 Test Agent 编写并冻结；实现侧（exec-rust / exec-web）只读，不得就地改断言。
//!
//! ## TUI 行为为什么以「bin-as-module」方式验证（而非读源码文本的黑盒断言）
//! TUI 的解析/命令表真值源是 `crates/pony-agent-core/src/bin/tui.rs`。它是 `[[bin]]` 目标，
//! 不是 lib target，因此 `use pony_agent_core::tui::...` 不可达。本仓库既有测试
//! （`tui_acceptance.rs` / `tui_render_layout.rs` / `tui_interaction_acceptance.rs` 等）已确立
//! `#[path = "../src/bin/tui.rs"] mod tui;` 的 bin-as-module 模式：编译期直接引用**同一份真实
//! 源码**，比「读字符串 + 解析字面量」的黑盒断言更强（后者在格式化/别名改写下会假绿）。
//! 故 T1~T3 采用该模式，未退化为主持人备选方案。
//!
//! ## 没有 `[RED DEGRADED]` 条目
//! 五条用例均为可执行断言，不存在「因本机环境无法构造而降级」的用例。T5 的「provider 不可用」
//! 用本机 loopback 上的确定性 401 mock（`TcpListener`，无外网依赖）构造，不触碰真实网络。
//!
//! ## 数据卫生
//! 集成测试编译 lib 时**不启用** `cfg(test)`，`control_plane::default_graph_run_store()` 会走
//! 生产分支（用户真实数据目录）。本文件因此：
//! 1) 显式注入 `GraphRunStore::new()`（内存态）；
//! 2) 会话存储用 `FileSessionBackend` 指向本测试二进制独占的临时目录；
//! 3) 兜底把 `XDG_DATA_HOME` / `HOME` / `LOCALAPPDATA` 重定向到该沙箱，绝不读写生产数据。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::compression::{should_compress, split_history, CompressionConfig};
use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{HostControlPlane, HostControlPlaneBuilder};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderManager, ProviderProtocol};
use pony_agent_core::agent::runtime::{
    compact_outcome_message, AgentRuntimeBuilder, ManualCompactionOutcome,
};
use pony_agent_core::agent::runtime_helper::TestRuntimeGuard;
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore, TurnHistoryMessage};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor, ToolResult};

/// 契约 C-4 冻结的完整命令表（顺序固定，共 10 条）。
const CONTRACT_COMMANDS: [&str; 10] = [
    "/resume", "/provider", "/models", "/thinking", "/plan", "/tasks", "/clear", "/compact", "/help",
    "/quit",
];

// ── 沙箱与测试脚手架 ───────────────────────────────────────────────────────────────────

static SANDBOX: Once = Once::new();

fn sandbox_root() -> PathBuf {
    SANDBOX.call_once(|| {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let sandbox = std::env::temp_dir().join(format!(
            "pony-agent-it-slash-commands-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        // 兜底隔离：任何仍走 dirs::data_local_dir()/home_dir() 的代码路径都落在沙箱内。
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
    std::env::temp_dir().join(format!(
        "pony-agent-it-slash-commands-{}",
        std::process::id()
    ))
}

/// 每个用例独占的会话存储（file backend → 沙箱临时目录，用例结束不影响其它用例）。
fn sandboxed_session_store(label: &str) -> SessionStore {
    let root = sandbox_root();
    let storage = root.join(format!("{label}-sessions"));
    std::fs::create_dir_all(&storage).expect("create isolated session storage");
    SessionStore::with_backend(Box::new(FileSessionBackend::new(storage)))
}

/// 只用于满足 builder 的默认面：本波次不执行任何工具调用。
struct NoopToolExecutor;

impl ToolExecutor for NoopToolExecutor {
    fn execute(&self, _call: &ToolCall) -> ToolResult {
        ToolResult {
            tool_name: "noop".to_string(),
            status: "ok".to_string(),
            output: String::new(),
            duration_ms: 0,
        }
    }
}

/// 固定 provider selection 的解析器（同 `runtime/tests.rs::StaticResolver` 的既有脚手架口径）。
struct StaticResolver {
    selection: ResolvedProviderSelection,
}

impl ProviderSelectionResolver for StaticResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

fn test_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "slash-commands-test".to_string(),
        provider_name: "slash-commands-test".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "SLASH_COMMANDS_TEST_API_KEY".to_string(),
        api_key: Some("test-key".to_string()),
        model: "slash-commands-test-model".to_string(),
        temperature: 0.2,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

/// 构造 host control plane（C-2 是集成测试可达的对外入口；C-1 的 `compact_session` 是
/// `pub(crate)`，集成测试不可直接调用，故 T4/T5 一律经 C-2 验证）。
fn build_control_plane(label: &str, base_url: String) -> (HostControlPlane, Arc<RwLock<SessionStore>>) {
    let runtime = AgentRuntimeBuilder::new()
        .session_store(sandboxed_session_store(label))
        .provider_resolver(Box::new(StaticResolver {
            selection: test_selection(base_url),
        }))
        .tool_executor(Box::new(NoopToolExecutor))
        .build();
    let sessions = runtime.sessions_handle();
    let plane = HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::new())
        .build();
    (plane, sessions)
}

fn seed_history(sessions: &Arc<RwLock<SessionStore>>, session_id: &str, history: Vec<TurnHistoryMessage>) {
    let mut store = sessions
        .write()
        .unwrap_or_else(|e| e.into_inner());
    store.replace_session_history(Some(session_id), history);
}

fn history_of(sessions: &Arc<RwLock<SessionStore>>, session_id: &str) -> Vec<TurnHistoryMessage> {
    let store = sessions
        .read()
        .unwrap_or_else(|e| e.into_inner());
    store
        .snapshot_at_readonly(Some(session_id), None, &[])
        .history
}

fn msg(role: &str, content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

fn render(history: &[TurnHistoryMessage]) -> Vec<String> {
    history
        .iter()
        .map(|item| format!("{}:{}", item.role, item.content))
        .collect()
}

/// 确定性「provider 不可用」mock：本机 loopback 上对任意请求返回 401（非可重试类错误，
/// 不会触发退避重试），因此用例不依赖外网、不依赖时序假设。
struct UnauthorizedProviderMock {
    base_url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl UnauthorizedProviderMock {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
        let address = listener.local_addr().expect("mock provider addr");
        listener
            .set_nonblocking(true)
            .expect("set mock provider nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0u8; 8192];
                        let _ = stream.read(&mut buffer);
                        let body =
                            r#"{"error":{"message":"mock provider unavailable","code":"unauthorized"}}"#;
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url: format!("http://{address}/v1"),
            stop,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ── T1：TUI 命令表（C-4，唯一真值源） ────────────────────────────────────────────────

#[test]
fn t1_tui_command_table_drops_cost_and_adds_compact() {
    let commands: Vec<&str> = tui::AVAILABLE_COMMANDS
        .iter()
        .map(|(command, _)| *command)
        .collect();

    // 契约 C-4：`/cost` 必须从命令表移除（不再提供金额/配额查看入口）。
    assert!(
        !commands.contains(&"/cost"),
        "T1: `/cost` 必须从 AVAILABLE_COMMANDS 移除，实际命令表 = {commands:?}"
    );

    // 契约 C-4：新增 `/compact`，既有 `/plan` `/tasks` `/quit` 必须保留。
    for required in ["/compact", "/plan", "/tasks", "/quit"] {
        assert!(
            commands.contains(&required),
            "T1: AVAILABLE_COMMANDS 必须包含 {required}，实际命令表 = {commands:?}"
        );
    }

    // 契约 C-4：顺序固定、共 10 条（任何增删/重排都是契约漂移）。
    assert_eq!(
        commands,
        CONTRACT_COMMANDS.to_vec(),
        "T1: AVAILABLE_COMMANDS 必须与契约 C-4 冻结的顺序/条目完全一致"
    );

    let describe = |name: &str| -> Option<&str> {
        tui::AVAILABLE_COMMANDS
            .iter()
            .find(|(command, _)| *command == name)
            .map(|(_, description)| *description)
    };
    // 契约 C-4 指定文案。
    assert_eq!(
        describe("/compact"),
        Some("压缩当前会话上下文"),
        "T1: `/compact` 描述文案由契约 C-4 冻结"
    );
    assert_eq!(
        describe("/quit"),
        Some("安全退出 TUI 界面"),
        "T1: `/quit` 描述文案必须保持不变"
    );
}

// ── T2：斜杠菜单不被硬编码 8 行截断（C-6） ───────────────────────────────────────────

#[test]
fn t2_slash_menu_returns_every_command_for_bare_slash_input() {
    let mut state = tui::TuiState::default();
    state.input_buffer = "/".to_string();

    let matched = state.matching_commands();

    // 契约 C-6：`matching_commands()` 不得再用硬编码 8 行上限截断；
    // 输入 "/" 必须返回全部命令（行数上限只能来自窗口可用行数）。
    assert_eq!(
        matched.len(),
        tui::AVAILABLE_COMMANDS.len(),
        "T2: 输入 \"/\" 时匹配数必须等于命令表长度 {}，实际 {} 条 = {:?}",
        tui::AVAILABLE_COMMANDS.len(),
        matched.len(),
        matched.iter().map(|(command, _)| *command).collect::<Vec<_>>()
    );

    // 逐条校验：命令表中每条命令都必须出现在匹配结果里（顺序保持命令表顺序）。
    let matched_commands: Vec<&str> = matched.iter().map(|(command, _)| *command).collect();
    let table_commands: Vec<&str> = tui::AVAILABLE_COMMANDS
        .iter()
        .map(|(command, _)| *command)
        .collect();
    assert_eq!(matched_commands, table_commands, "T2: 匹配结果必须是命令表的完整有序前缀（此处即全表）");

    // 反向校验：非斜杠输入不得返回菜单（避免「全量返回」式假绿）。
    state.input_buffer = "plan".to_string();
    assert!(
        state.matching_commands().is_empty(),
        "T2: 非斜杠输入不得弹出斜杠菜单"
    );
}

// ── T3：命令 token 解析（C-5） ───────────────────────────────────────────────────────

#[test]
fn t3_command_token_matching_is_case_insensitive_and_tolerates_trailing_args() {
    // 契约 C-5：取 trim() 后首个空白分隔 token 比较，大小写不敏感；
    // 返回值必须是命令表中的**规范化命令字面量**（`&'static str`）——因为契约 C-5 的签名
    // 返回 `Option<&'static str>`，只有命令表里的 &'static str 能保证该生命周期。
    assert_eq!(
        tui::match_command_token("/plan"),
        Some("/plan"),
        "T3: 精确命令必须命中"
    );
    assert_eq!(
        tui::match_command_token("/PLAN"),
        Some("/plan"),
        "T3: 大小写不敏感（/PLAN → /plan）"
    );
    assert_eq!(
        tui::match_command_token("  /plan  "),
        Some("/plan"),
        "T3: 首尾空白必须被 trim"
    );
    assert_eq!(
        tui::match_command_token("/plan now"),
        Some("/plan"),
        "T3: 尾随参数不得影响命中"
    );
    assert_eq!(
        tui::match_command_token("/compact now"),
        Some("/compact"),
        "T3: /compact 带尾随参数必须命中"
    );
    assert_eq!(
        tui::match_command_token("/COMPACT"),
        Some("/compact"),
        "T3: /COMPACT 必须命中 /compact"
    );

    // 不命中矩阵：不得把路径/带后缀词/裸斜杠误判为命令（否则用户消息被静默吞掉）。
    assert_eq!(
        tui::match_command_token("/etc/hosts"),
        None,
        "T3: 绝对路径不得命中命令"
    );
    assert_eq!(
        tui::match_command_token("/etc/hosts 是什么"),
        None,
        "T3: 路径开头的普通消息不得命中命令"
    );
    assert_eq!(
        tui::match_command_token("/tasks2"),
        None,
        "T3: 前缀相同的未知词（/tasks2）不得命中"
    );
    assert_eq!(tui::match_command_token("/"), None, "T3: 裸斜杠不得命中命令");
    assert_eq!(tui::match_command_token(""), None, "T3: 空输入不得命中命令");
    assert_eq!(
        tui::match_command_token("hello /plan"),
        None,
        "T3: 首 token 不是命令时不得命中"
    );
}

// ── T4：force=true 在历史过短会话上是「诚实空操作」（C-1 语义 3） ────────────────────

#[test]
fn t4_forced_compaction_on_short_history_is_a_noop_and_preserves_history() {
    let _guard = TestRuntimeGuard::new();
    const SESSION: &str = "slash-t4-short-history";

    // provider 指向一个必然失败的地址：本用例根本不该发起 LLM 调用，
    // 若实现错误地先调 LLM 再判断长度，则会得到 Err 并让本用例以正确的理由变红。
    let (plane, sessions) = build_control_plane("t4", "http://127.0.0.1:1/v1".to_string());

    let seeded = vec![
        msg("user", "你好，帮我看看仓库结构。"),
        msg("assistant", "仓库是 Rust core + Vue 前端的单仓结构。"),
        msg("user", "好的。"),
        msg("assistant", "还有需要我做的吗？"),
    ];
    seed_history(&sessions, SESSION, seeded.clone());

    // 前置条件（真实断言，不是注释）：该历史确实「过短」——
    // `split_history` 的可压缩段为空，因此 C-1 语义 3 必然要求 history_too_short。
    let (to_compress, to_keep) = split_history(&seeded, &CompressionConfig::default());
    assert!(
        to_compress.is_empty(),
        "T4 前置条件失败：本用例的历史必须落在 split_history 的『无可压缩段』区间"
    );
    assert_eq!(to_keep.len(), seeded.len(), "T4 前置条件：过短历史应整段保留");

    let before = history_of(&sessions, SESSION);
    assert_eq!(render(&before), render(&seeded), "T4 前置条件：播种历史必须可读回");

    let outcome = plane
        .compact_session(Some(SESSION), true)
        .expect("T4: 历史过短属『成功但未压缩』，不得返回 Err");

    assert!(!outcome.compacted, "T4: 历史过短时 compacted 必须为 false（不得谎报已压缩）");
    assert_eq!(
        outcome.reason, "history_too_short",
        "T4: reason 必须如实为 history_too_short"
    );
    assert_eq!(outcome.compressed_count, 0, "T4: 未压缩时 compressed_count 必须为 0");
    assert_eq!(
        outcome.kept_count,
        seeded.len(),
        "T4: kept_count 必须等于未被压缩的原始条数"
    );

    // 契约 C-1 语义 3：不改写历史。长度与逐条内容都必须一致。
    let after = history_of(&sessions, SESSION);
    assert_eq!(
        after.len(),
        before.len(),
        "T4: history_too_short 不得改变历史长度"
    );
    assert_eq!(
        render(&after),
        render(&before),
        "T4: history_too_short 不得改写任何历史消息内容"
    );
}

// ── G-2：C-1 语义 2（force=false 且未达阈值 ⇒ below_threshold，零 LLM 调用） ──────────
//
// Lead 裁决追加（ADVISORY 缺口补丁）：审查实测把 `if !force && !should_compress` 改成
// `if false && ...` 后全绿 —— 说明此前没有任何用例约束 `force` 参数与 `below_threshold` 分支。
// 下面两条用例分别锁死「语义 2 的判定顺序」与「先判阈值、后判长度」：
// - G-2a 历史过短 + force=false：必须是 below_threshold（不得变成 history_too_short，
//   否则说明实现先做 split_history、把 `!force` 检查降级/跳过了）；
// - G-2b 历史足够长 + force=false 且未达阈值：必须 below_threshold 且**零 LLM 调用**
//   （provider 指向必然不可达地址，任何 LLM 尝试都会变成 Err 而让用例失败）。

#[test]
fn g2a_unforced_compaction_reports_below_threshold_before_history_length() {
    let _guard = TestRuntimeGuard::new();
    const SESSION: &str = "slash-g2a-unforced-short-history";

    // provider 不可达：本用例不得发起任何 LLM 调用（错误地先调 LLM 会得到 Err → 用例失败）。
    let (plane, sessions) = build_control_plane("g2a", "http://127.0.0.1:1/v1".to_string());

    let seeded = vec![
        msg("user", "你好。"),
        msg("assistant", "你好，请说。"),
    ];
    seed_history(&sessions, SESSION, seeded.clone());

    // 前置条件（真实断言）：在默认压缩配置 + 128k 上下文窗口下，该历史确实**未达**压缩水位，
    // 即 C-1 语义 2 的 `should_compress(...) == false` 成立。
    let config = CompressionConfig::default();
    let provider = ProviderManager::new(test_selection("http://127.0.0.1:1/v1".to_string()));
    assert!(
        !should_compress(&seeded, &provider, &config),
        "G-2a 前置条件失败：过短历史必须低于压缩阈值"
    );

    let before = history_of(&sessions, SESSION);
    let outcome = plane
        .compact_session(Some(SESSION), false)
        .expect("G-2a: 未达阈值属『成功但未压缩』，不得返回 Err");

    assert!(!outcome.compacted, "G-2a: force=false 且未达阈值时 compacted 必须为 false");
    assert_eq!(
        outcome.reason, "below_threshold",
        "G-2a: 契约 C-1 语义 2 要求 reason 为 below_threshold（判定顺序先于历史长度）"
    );
    assert_eq!(outcome.compressed_count, 0, "G-2a: 未压缩时 compressed_count 必须为 0");
    assert_eq!(outcome.kept_count, seeded.len(), "G-2a: kept_count 必须等于原始条数");

    // 契约 C-1 语义 2：不改写历史。
    let after = history_of(&sessions, SESSION);
    assert_eq!(render(&after), render(&before), "G-2a: below_threshold 不得改写历史");
}

#[test]
fn g2b_unforced_compaction_on_long_history_stays_below_threshold_without_llm() {
    let _guard = TestRuntimeGuard::new();
    const SESSION: &str = "slash-g2b-unforced-long-history";

    let (plane, sessions) = build_control_plane("g2b", "http://127.0.0.1:1/v1".to_string());

    // 足够长的历史（24 条），确保即便 `force` 判断被跳过、split_history 也能给出可压缩段——
    // 这样「错误地继续走压缩流程」会立刻暴露为 Err（provider 不可达）。
    let mut seeded: Vec<TurnHistoryMessage> = Vec::new();
    for round in 0..12 {
        seeded.push(msg(
            "user",
            &format!("第 {round} 轮：请说明手动压缩的 force 语义与阈值判定顺序。"),
        ));
        seeded.push(msg(
            "assistant",
            &format!("第 {round} 轮：force=false 时先判 should_compress，未达阈值即返回 below_threshold。"),
        ));
    }
    seed_history(&sessions, SESSION, seeded.clone());

    let config = CompressionConfig::default();
    let provider = ProviderManager::new(test_selection("http://127.0.0.1:1/v1".to_string()));
    assert!(
        !should_compress(&seeded, &provider, &config),
        "G-2b 前置条件失败：该历史必须仍未达压缩阈值"
    );
    let (to_compress, _to_keep) = split_history(&seeded, &config);
    assert!(
        !to_compress.is_empty(),
        "G-2b 前置条件失败：历史必须存在可压缩段，否则『跳过 force 判定』不会被暴露"
    );

    let before = history_of(&sessions, SESSION);
    let outcome = plane
        .compact_session(Some(SESSION), false)
        .expect("G-2b: force=false 且未达阈值必须成功返回（不得尝试 LLM、不得返回 Err）");

    assert!(!outcome.compacted, "G-2b: 未达阈值时 compacted 必须为 false");
    assert_eq!(
        outcome.reason, "below_threshold",
        "G-2b: 契约 C-1 语义 2 要求 reason 为 below_threshold"
    );
    assert_eq!(outcome.compressed_count, 0, "G-2b: 不得压缩任何消息");
    assert_eq!(
        outcome.before_tokens, outcome.after_tokens,
        "G-2b: 未压缩时压缩前后 token 估算必须一致（未改写历史）"
    );

    let after = history_of(&sessions, SESSION);
    assert_eq!(render(&after), render(&before), "G-2b: below_threshold 不得改写历史");
}

// ── T5：provider 不可用必须 Err（C-1 语义 5，禁止假成功） ────────────────────────────

#[test]
fn t5_compaction_fails_loudly_when_provider_is_unavailable() {
    let _guard = TestRuntimeGuard::new();
    const SESSION: &str = "slash-t5-provider-unavailable";

    let provider = UnauthorizedProviderMock::start();
    let (plane, sessions) = build_control_plane("t5", provider.base_url());

    // 足够长的历史：24 条（12 轮），远超 keep_recent_turns*2，确保 split_history 有可压缩段。
    let mut seeded: Vec<TurnHistoryMessage> = Vec::new();
    for round in 0..12 {
        seeded.push(msg(
            "user",
            &format!(
                "第 {round} 轮问题：请详细说明上下文压缩在长会话中的触发条件与阈值计算方式。"
            ),
        ));
        seeded.push(msg(
            "assistant",
            &format!(
                "第 {round} 轮回答：阈值 = min(context_window * 0.8, context_window - reserved - headroom)，\
                 超过阈值后按 turn 边界把最旧的消息压成摘要。"
            ),
        ));
    }
    seed_history(&sessions, SESSION, seeded.clone());

    // 前置条件（真实断言）：历史确实足够长，可压缩段非空——否则「因历史太短而提前返回」会让
    // 本用例以错误理由通过（假绿）。
    let (to_compress, to_keep) = split_history(&seeded, &CompressionConfig::default());
    assert!(
        !to_compress.is_empty(),
        "T5 前置条件失败：历史必须存在可压缩段，否则 provider 失败路径不会被触达"
    );
    assert!(!to_keep.is_empty(), "T5 前置条件：必须保留最近 turn 原文");

    let before = history_of(&sessions, SESSION);
    assert_eq!(render(&before), render(&seeded), "T5 前置条件：播种历史必须可读回");

    let result = plane.compact_session(Some(SESSION), true);
    let error = result.expect_err(
        "T5: provider 不可用（401）必须返回 Err —— 契约 C-1 语义 5 禁止『未压缩但报成功』",
    );
    assert!(
        !error.trim().is_empty(),
        "T5: Err 必须携带可诊断的错误摘要，不得为空字符串"
    );

    // 失败路径不得留下「半压缩」状态：历史必须原样保留。
    let after = history_of(&sessions, SESSION);
    assert_eq!(
        render(&after),
        render(&before),
        "T5: LLM 失败时不得改写历史（不得出现部分压缩）"
    );

    provider.finish();
}

// ── G-3：TUI/CLI 压缩文案纯函数（契约 C-7，审查认定零覆盖缺口） ──────────────────────
//
// Lead 裁决追加。审查实测事实：删掉 TUI 整个 `/compact` 分支后测试仍全绿——原因是文案逻辑
// 当时内联在事件循环里、且无任何断言。实现侧已抽出纯函数
// `compact_outcome_message(&Result<ManualCompactionOutcome, String>) -> String`
// （`agent/runtime/compaction_ops.rs`，经 `agent::runtime` 重导出），TUI 侧改为消费它，
// 因此文案矩阵现在可被直接盯死（防「多处文案漂移」与「谎报已压缩」）。

fn compacted_outcome() -> ManualCompactionOutcome {
    ManualCompactionOutcome {
        compacted: true,
        reason: "compacted".to_string(),
        compressed_count: 7,
        kept_count: 5,
        before_tokens: 9_000,
        after_tokens: 3_000,
    }
}

fn untouched_outcome(reason: &str, kept_count: usize) -> ManualCompactionOutcome {
    ManualCompactionOutcome {
        compacted: false,
        reason: reason.to_string(),
        compressed_count: 0,
        kept_count,
        before_tokens: 1_234,
        after_tokens: 1_234,
    }
}

#[test]
fn g3a_success_message_reports_counts_and_claims_compaction() {
    let message = compact_outcome_message(&Ok(compacted_outcome()));

    // ① 成功文案必须含"已压缩"与真实计数（compressed/kept），以及压缩前后 token。
    assert!(
        message.contains("已压缩"),
        "G-3a: 成功文案必须明确声明已压缩，实际 = {message:?}"
    );
    assert!(
        message.contains('7') && message.contains('5'),
        "G-3a: 成功文案必须含 compressed_count=7 与 kept_count=5，实际 = {message:?}"
    );
    assert!(
        message.contains("9000") && message.contains("3000"),
        "G-3a: 成功文案必须含压缩前后 token（9000 / 3000），实际 = {message:?}"
    );
    // 成功路径不得同时声称「无需压缩」（语义互斥）。
    assert!(
        !message.contains("无需压缩"),
        "G-3a: 成功文案不得出现『无需压缩』，实际 = {message:?}"
    );
}

#[test]
fn g3b_history_too_short_message_states_no_compaction_needed() {
    let message = compact_outcome_message(&Ok(untouched_outcome("history_too_short", 24)));

    // ② 历史过短：含「无需压缩」，**不得**出现「已压缩」（不得谎报）。
    assert!(
        message.contains("无需压缩"),
        "G-3b: history_too_short 文案必须含『无需压缩』，实际 = {message:?}"
    );
    assert!(
        message.contains("历史过短"),
        "G-3b: 文案必须如实说明原因（历史过短），实际 = {message:?}"
    );
    assert!(
        message.contains("24"),
        "G-3b: 文案必须含保留条数 kept_count=24，实际 = {message:?}"
    );
    assert!(
        !message.contains("已压缩"),
        "G-3b: history_too_short 不得出现『已压缩』（契约 C-7 禁止谎报），实际 = {message:?}"
    );
}

#[test]
fn g3c_below_threshold_message_states_no_compaction_needed() {
    let message = compact_outcome_message(&Ok(untouched_outcome("below_threshold", 12)));

    // ③ 未达阈值：同样含「无需压缩」，不得出现「已压缩」。
    assert!(
        message.contains("无需压缩"),
        "G-3c: below_threshold 文案必须含『无需压缩』，实际 = {message:?}"
    );
    assert!(
        message.contains("阈值"),
        "G-3c: 文案必须如实体现在阈值原因，实际 = {message:?}"
    );
    assert!(
        message.contains("12"),
        "G-3c: 文案必须含保留条数 kept_count=12，实际 = {message:?}"
    );
    assert!(
        !message.contains("已压缩"),
        "G-3c: below_threshold 不得出现『已压缩』，实际 = {message:?}"
    );
}

#[test]
fn g3d_error_message_carries_reason_and_never_claims_compaction() {
    let message = compact_outcome_message(&Err("provider 401 unauthorized".to_string()));

    // ④ 失败：含错误片段与失败措辞，且不得出现「已压缩」，也不得伪装成「无需压缩」。
    assert!(
        message.contains("provider 401 unauthorized"),
        "G-3d: 失败文案必须带出错误摘要，实际 = {message:?}"
    );
    assert!(
        message.contains("失败"),
        "G-3d: 失败文案必须有失败措辞，实际 = {message:?}"
    );
    assert!(
        !message.contains("已压缩"),
        "G-3d: 失败文案不得出现『已压缩』（禁止假成功），实际 = {message:?}"
    );
    assert!(
        !message.contains("无需压缩"),
        "G-3d: 失败文案不得伪装成『无需压缩』，实际 = {message:?}"
    );
}

#[test]
fn g3e_overlong_error_is_truncated_to_200_chars() {
    // 边界：超长错误信息必须被截断到 200 chars（按字符而非字节，CJK 不得切出乱码/panic），
    // 否则一条超长上游错误就能把 TUI 转录区文案撑爆。
    let prefix = "上下文压缩失败: ";
    let long_error = "错误".repeat(300); // 600 chars
    let message = compact_outcome_message(&Err(long_error));

    assert!(
        message.starts_with(prefix),
        "G-3e: 失败文案前缀应稳定，实际 = {message:?}"
    );
    let summary_chars = message.chars().count() - prefix.chars().count();
    assert!(
        summary_chars <= 200,
        "G-3e: 错误摘要必须截断到 ≤200 chars，实际 {summary_chars} chars"
    );
    assert_eq!(
        summary_chars, 200,
        "G-3e: 600 chars 的错误必须恰好截断到 200 chars，实际 {summary_chars} chars"
    );
    // 按字符截断的证据：200 chars 恰为「错误」× 100（若按字节截断会劈开一个 3 字节汉字）。
    assert!(
        message.contains(&"错误".repeat(100)),
        "G-3e: 截断必须按字符边界进行（CJK 不得被劈开）"
    );
}

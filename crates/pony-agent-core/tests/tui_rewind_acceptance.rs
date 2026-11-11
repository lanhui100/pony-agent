//! ADR 0113「撤回」验收：控制面 `rewind_session` 的端到端行为（TUI 真实入口 + 本地 mock provider）。
//!
//! 覆盖判据：
//! - **R1**：两轮之后 `list_rewind_points` 返回 2 条（新→旧），最新一条带原文「第二问」；
//! - **R2**（仅会话）：撤回到第 2 轮之前 ⇒ 会话只剩第 1 轮的 user+assistant，回执回填「第二问」；
//!   活动链上只剩 1 个可撤回轮次；已取消节点不得再次撤回（明确 Err）；
//! - **R3**（会话+代码）：文件按「被撤销轮次里最早 pre-image 取胜」还原，被撤销轮次新建的文件被删除。
//!
//! 纪律：AAA、真实断言、无 `#[ignore]`、不吞异常；沙箱隔离口径与 `tests/ask_resume_e2e.rs` 同源
//! （集成测试绝不触碰用户真实数据目录与真实快照目录）。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::rewind_commands::{
    RewindMode, RewindPointsQuery, RewindSessionCommand,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, SessionRuntimeViewQuery, StartTurnStreamCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{
    FileSessionBackend, RewindFileStore, SessionStore, SNAPSHOT_ROOT_ENV,
};
use pony_agent_core::agent::turn_flow::TurnEventSink;

const WAIT_BUDGET_MS: u64 = 10_000;

static ISOLATE_PROCESS_DATA_DIR: Once = Once::new();

fn isolate_process_data_dir() {
    ISOLATE_PROCESS_DATA_DIR.call_once(|| {
        let unique = format!(
            "pony-agent-it-rewind-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        );
        let sandbox = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
        // 撤回文件快照同样必须落沙箱：控制面用 `RewindFileStore::shared()` 读该 env。
        std::env::set_var(SNAPSHOT_ROOT_ENV, sandbox.join("rewind"));
    });
}

fn snapshot_root() -> std::path::PathBuf {
    isolate_process_data_dir();
    std::path::PathBuf::from(std::env::var(SNAPSHOT_ROOT_ENV).expect("snapshot root env"))
}

fn isolated_workspace(test_name: &str) -> std::path::PathBuf {
    isolate_process_data_dir();
    let dir = std::env::temp_dir().join(format!(
        "pony-agent-rewind-ws-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create isolated workspace dir");
    dir
}

/// 同进程用例共享沙箱会话库 ⇒ 串行夹具（不含断言）。
static SERIAL_TEST_GUARD: Mutex<()> = Mutex::new(());

fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
    SERIAL_TEST_GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_until<F: FnMut() -> bool>(mut predicate: F, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 记录型 sink：断言事件序列用（只做观测，不参与判定真值）。
#[derive(Default)]
struct RecordingSink {
    captured: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self::default()
    }

    fn names(&self) -> Vec<String> {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .push((name.to_string(), payload));
    }
}

fn turn_input(session_id: &str, message: &str) -> TurnInput {
    TurnInput {
        message: message.to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    }
}

/// 经 TUI 真实入口跑一轮，并断言以 `turn:completed` 收口。
fn run_turn(
    control_plane: &HostControlPlane,
    sink: &RecordingSink,
    session_id: &str,
    turn_id: &str,
    message: &str,
) {
    let outcome = control_plane.start_turn_stream_checked(
        sink,
        StartTurnStreamCommand {
            turn_id: turn_id.to_string(),
            input: turn_input(session_id, message),
        },
    );
    assert!(
        outcome.is_ok(),
        "轮次 {turn_id} 必须以 Ok 收口，实际 = {:?}",
        outcome.err()
    );
    let names = sink.names();
    assert!(
        names.iter().any(|name| name == "turn:completed"),
        "轮次 {turn_id} 必须出现 turn:completed，实际事件序列 = {names:?}"
    );
}

// ── 本地回环 mock provider（非流式 JSON：每次请求出队一段 assistant 文本）──────────

struct MockProviderServer {
    base_url: String,
    addr: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockProviderServer {
    fn start(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("mock addr").port();
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                let _body = read_http_request_body(&mut stream);
                if let Ok(mut captured) = requests_for_thread.lock() {
                    captured.push("request".to_string());
                }
                let reply = replies
                    .lock()
                    .expect("mock reply queue poisoned")
                    .pop_front()
                    .unwrap_or_else(|| "mock-queue-exhausted".to_string());
                write_http_json(&mut stream, &text_completion_body(&reply));
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            shutdown,
            handle: Some(handle),
            requests,
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("mock request lock poisoned").len()
    }

    fn wait_request_count(&self, expected: usize) -> bool {
        wait_until(|| self.request_count() >= expected, WAIT_BUDGET_MS)
    }
}

impl Drop for MockProviderServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }
        if header_end.is_none() {
            if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = Some(index + 4);
                let headers = String::from_utf8_lossy(&buffer[..index + 4]).to_lowercase();
                for line in headers.lines() {
                    if let Some(value) = line.strip_prefix("content-length:") {
                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
        }
        if let Some(header_end) = header_end {
            if buffer.len().saturating_sub(header_end) >= content_length {
                break;
            }
        }
    }
    let body_start = header_end.unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[body_start..]).to_string()
}

fn write_http_json(stream: &mut TcpStream, body: &str) {
    let payload = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

fn text_completion_body(text: &str) -> String {
    json!({
        "choices": [{ "message": { "role": "assistant", "content": text } }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 7, "total_tokens": 14 }
    })
    .to_string()
}

struct StaticProviderResolver {
    selection: ResolvedProviderSelection,
}

impl ProviderSelectionResolver for StaticProviderResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

fn provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "rewind-test".to_string(),
        provider_name: "rewind-test".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "REWIND_TEST_API_KEY".to_string(),
        api_key: Some("rewind-test-key".to_string()),
        model: "rewind-test-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn control_plane_with_mock_provider(base_url: String, workspace: &Path) -> HostControlPlane {
    isolate_process_data_dir();
    // 文件后端（无 turn 事件表）：节点保留内嵌快照，撤回的会话还原走 legacy 真值路径。
    // 这样验收不依赖沙箱 SQLite 的事件装载细节，也不触碰用户真实会话库。
    let store_path = workspace.join("sessions.json");
    let session_store =
        SessionStore::with_backend(Box::new(FileSessionBackend::new(store_path)));
    let runtime = AgentRuntimeBuilder::new()
        .session_store(session_store)
        .provider_resolver(Box::new(StaticProviderResolver {
            selection: provider_selection(base_url),
        }))
        .workspace_root(workspace)
        .build();
    HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::new())
        .build()
}

fn history_roles(control_plane: &HostControlPlane, session_id: &str) -> Vec<(String, String)> {
    let view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        session_id: Some(session_id.to_string()),
        turn_id: None,
        node_id: None,
        run_id: None,
    });
    view.session
        .history
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect()
}

// ── R1 / R2：会话撤回（仅会话）─────────────────────────────────────────────

#[test]
fn r1_r2_rewind_lists_user_turns_and_truncates_transcript() {
    let _guard = serial_guard();
    let _rt = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    let workspace = isolated_workspace("conversation");
    let server = MockProviderServer::start(vec![
        "第一答".to_string(),
        "第二答".to_string(),
        "重开答".to_string(),
    ]);
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);

    // Arrange：真实跑两轮，产生两个 TurnCommitted 节点。
    let session_id = "rewind-conversation";
    let sink = RecordingSink::new();
    run_turn(&control_plane, &sink, session_id, "turn-1", "第一问");
    run_turn(&control_plane, &sink, session_id, "turn-2", "第二问");

    // R1：两个可撤回轮次，新→旧，且带原文。
    let points = control_plane.list_rewind_points(RewindPointsQuery {
        session_id: Some(session_id.to_string()),
    });
    assert_eq!(points.len(), 2, "两轮 ⇒ 两个撤回点，实际 {points:?}");
    assert_eq!(points[0].user_message, "第二问", "最新一轮在最前");
    assert_eq!(points[1].user_message, "第一问");

    // R2：撤回到第 2 轮之前。
    let response = control_plane
        .rewind_session(RewindSessionCommand {
            session_id: Some(session_id.to_string()),
            node_id: points[0].node_id.clone(),
            mode: RewindMode::ConversationOnly,
        })
        .expect("撤回必须成功");
    assert_eq!(response.user_message.as_deref(), Some("第二问"));
    assert_eq!(response.turn_index, 2);
    assert!(
        response.files_restored.is_empty(),
        "仅会话模式不得触碰文件：{:?}",
        response.files_restored
    );
    assert_eq!(
        history_roles(&control_plane, session_id),
        vec![
            ("user".to_string(), "第一问".to_string()),
            ("assistant".to_string(), "第一答".to_string())
        ],
        "会话必须截断到第 1 轮"
    );
    let remaining = control_plane.list_rewind_points(RewindPointsQuery {
        session_id: Some(session_id.to_string()),
    });
    assert_eq!(remaining.len(), 1, "撤回后只剩第 1 轮可撤回");

    // 已取消节点再撤 ⇒ 明确 Err（不伪造成功）。
    let stale = control_plane.rewind_session(RewindSessionCommand {
        session_id: Some(session_id.to_string()),
        node_id: points[0].node_id.clone(),
        mode: RewindMode::ConversationOnly,
    });
    assert!(stale.is_err(), "已取消的节点不得再次撤回");

    // R4：撤回到**第一轮之前** ⇒ 走「空会话 checkpoint 根」路径；撤完还能继续聊。
    let to_empty = control_plane
        .rewind_session(RewindSessionCommand {
            session_id: Some(session_id.to_string()),
            node_id: remaining[0].node_id.clone(),
            mode: RewindMode::ConversationOnly,
        })
        .expect("撤回到空会话必须成功");
    assert_eq!(to_empty.user_message.as_deref(), Some("第一问"));
    assert_eq!(to_empty.turn_index, 1);
    assert_eq!(
        history_roles(&control_plane, session_id),
        Vec::new(),
        "撤回到第一轮之前 ⇒ 会话历史清空"
    );
    assert!(
        control_plane
            .list_rewind_points(RewindPointsQuery {
                session_id: Some(session_id.to_string()),
            })
            .is_empty(),
        "空会话不得再有可撤回轮次"
    );
    let restarted = RecordingSink::new();
    run_turn(&control_plane, &restarted, session_id, "turn-3", "重开一问");
    assert_eq!(
        history_roles(&control_plane, session_id),
        vec![
            ("user".to_string(), "重开一问".to_string()),
            ("assistant".to_string(), "重开答".to_string())
        ],
        "撤回后新轮只带本轮上下文"
    );
    assert!(server.wait_request_count(3), "三轮回执各一次上游调用");
}

// ── R3：会话 + 代码（pre-image 还原 / 新建文件删除）────────────────────────

#[test]
fn r3_rewind_restores_files_from_earliest_pre_image_of_cancelled_turns() {
    let _guard = serial_guard();
    let _rt = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    let workspace = isolated_workspace("files");
    let server = MockProviderServer::start(vec!["答一".to_string(), "答二".to_string()]);
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);

    let session_id = "rewind-files";
    let sink = RecordingSink::new();
    run_turn(&control_plane, &sink, session_id, "turn-1", "改第一处");
    run_turn(&control_plane, &sink, session_id, "turn-2", "改第二处");

    let points = control_plane.list_rewind_points(RewindPointsQuery {
        session_id: Some(session_id.to_string()),
    });
    assert_eq!(points.len(), 2, "两轮 ⇒ 两个撤回点");
    let turn_1 = points[1].turn_id.clone().expect("第 1 轮有 turn id");
    let turn_2 = points[0].turn_id.clone().expect("第 2 轮有 turn id");

    // Arrange：模拟写工具在落盘前的两轮快照 + 一次新建。
    let store = RewindFileStore::new(snapshot_root());
    let edited = workspace.join("edited.txt");
    let created = workspace.join("created.txt");
    std::fs::write(&edited, "v0").expect("seed v0");
    store
        .record_pre_image(session_id, &turn_1, &edited)
        .expect("turn1 pre-image");
    std::fs::write(&edited, "v1").expect("write v1");
    store
        .record_pre_image(session_id, &turn_2, &edited)
        .expect("turn2 pre-image");
    store
        .record_pre_image(session_id, &turn_2, &created)
        .expect("turn2 creation pre-image");
    std::fs::write(&edited, "v2").expect("write v2");
    std::fs::write(&created, "agent-created").expect("create file");

    // Act：撤回到第 2 轮之前，且要求恢复代码。
    let response = control_plane
        .rewind_session(RewindSessionCommand {
            session_id: Some(session_id.to_string()),
            node_id: points[0].node_id.clone(),
            mode: RewindMode::ConversationAndFiles,
        })
        .expect("撤回必须成功");

    // Assert：最早 pre-image 取胜（turn2 的 pre-image = v1），新建文件被删除。
    assert_eq!(
        std::fs::read_to_string(&edited).expect("read edited"),
        "v1",
        "必须还原到第 2 轮之前的版本"
    );
    assert!(!created.exists(), "被撤销轮次新建的文件必须删除");
    assert!(!response.file_restore_degraded, "本次还原不应降级");
    assert_eq!(response.files_restored.len(), 1);
    assert_eq!(response.files_deleted.len(), 1);
    assert_eq!(
        response.cancelled_turn_ids,
        vec![turn_2],
        "只撤销第 2 轮及之后"
    );
    assert_eq!(
        history_roles(&control_plane, session_id).len(),
        2,
        "会话同口径截断到第 1 轮"
    );
    assert!(server.wait_request_count(2), "两轮各一次上游调用");
}

fn history_node_views(
    control_plane: &HostControlPlane,
    session_id: &str,
) -> Vec<pony_agent_core::agent::control_plane::HistoryNodeView> {
    control_plane
        .load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some(session_id.to_string()),
            turn_id: None,
            node_id: None,
            run_id: None,
        })
        .history_nodes
        .unwrap_or_default()
}

fn empty_root_node_id(
    control_plane: &HostControlPlane,
    session_id: &str,
) -> String {
    history_node_views(control_plane, session_id)
        .into_iter()
        .find(|node| node.parent_node_id.is_none() && node.turn_id.is_none())
        .map(|node| node.node_id)
        .expect("撤回/截断后的会话必须有可 checkout 的根节点")
}

// ── R5：D3 对抗复核——空会话 checkpoint 根与截断必须跨存储重载存活 ─────────────

/// 怀疑点（ADR 0113 D3）：`ensure_rewind_root_checkpoint` 在内存态里**合成**一个无事件
/// 承载的空 checkpoint 根，并把现有无父节点挂到它下面。若它没被持久化，重启后：
/// （a）截断状态复活；（b）新轮的父节点指向不存在的节点 ⇒ 撤回到空会话直接 Err。
/// 本测试用「同一落盘文件上新建会话存储」模拟重启，断言两条都不会发生。
#[test]
fn r5_rewind_to_empty_survives_store_reload() {
    let _guard = serial_guard();
    let _rt = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    let workspace = isolated_workspace("reload");
    // 同名工作区可能被历史运行留下 sessions.json ⇒ 先清干净，保证是真空装载。
    let _ = std::fs::remove_file(workspace.join("sessions.json"));
    let server = MockProviderServer::start(vec![
        "第一答".to_string(),
        "第二答".to_string(),
        "重载答".to_string(),
    ]);
    let session_id = "rewind-reload";

    // Arrange：两轮之后撤回到第一轮之前（此刻才补出「空会话 checkpoint 根」）。
    let root_before = {
        let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);
        let sink = RecordingSink::new();
        run_turn(&control_plane, &sink, session_id, "turn-1", "第一问");
        run_turn(&control_plane, &sink, session_id, "turn-2", "第二问");
        let points = control_plane.list_rewind_points(RewindPointsQuery {
            session_id: Some(session_id.to_string()),
        });
        let first = points.last().expect("两轮必有撤回点").node_id.clone();
        let response = control_plane
            .rewind_session(RewindSessionCommand {
                session_id: Some(session_id.to_string()),
                node_id: first,
                mode: RewindMode::ConversationOnly,
            })
            .expect("撤回到空会话必须成功");
        assert_eq!(response.turn_index, 1);
        assert_eq!(
            history_roles(&control_plane, session_id),
            Vec::new(),
            "撤回后会话必须为空"
        );
        empty_root_node_id(&control_plane, session_id)
    };

    // Act：同一落盘文件上新建存储（等价于重启）。
    let restarted = control_plane_with_mock_provider(server.base_url.clone(), &workspace);

    // Assert（a）：截断状态存活——会话仍为空，已撤销轮次不得复活。
    assert_eq!(
        history_roles(&restarted, session_id),
        Vec::new(),
        "重启后会话必须仍是撤回后的空状态"
    );
    assert!(
        restarted
            .list_rewind_points(RewindPointsQuery {
                session_id: Some(session_id.to_string()),
            })
            .is_empty(),
        "重启后不得把已撤销轮次重新列为可撤回点"
    );
    assert!(
        history_node_views(&restarted, session_id)
            .iter()
            .any(|node| node.node_id == root_before),
        "空会话 checkpoint 根必须跨重启保留同一 node_id（否则新轮父节点悬空）：{root_before}"
    );

    // Assert（b）：空根存活——重启后新轮仍能撤回到它之前（父节点必须真实存在）。
    let sink = RecordingSink::new();
    run_turn(&restarted, &sink, session_id, "turn-3", "重载问");
    let points = restarted.list_rewind_points(RewindPointsQuery {
        session_id: Some(session_id.to_string()),
    });
    assert_eq!(points.len(), 1, "重启 + 新轮 ⇒ 唯一可撤回点，实际 {points:?}");
    let turn_node = history_node_views(&restarted, session_id)
        .into_iter()
        .find(|node| node.node_id == points[0].node_id)
        .expect("新轮节点必须出现在会话视图里");
    assert_eq!(
        turn_node.parent_node_id.as_deref(),
        Some(root_before.as_str()),
        "重启后的新轮必须挂在持久化下来的空根之下"
    );
    let back_to_empty = restarted
        .rewind_session(RewindSessionCommand {
            session_id: Some(session_id.to_string()),
            node_id: points[0].node_id.clone(),
            mode: RewindMode::ConversationOnly,
        })
        .expect("重启后二次撤回到空会话必须成功（空根必须已持久化）");
    assert_eq!(back_to_empty.user_message.as_deref(), Some("重载问"));
    assert_eq!(
        history_roles(&restarted, session_id),
        Vec::new(),
        "二次撤回后仍必须为空"
    );
    assert!(server.wait_request_count(3), "三轮各一次上游调用");
}

/// ISSUE-20261111-004 红相测试：引用化节点在撤回后，TUI 重放使用的 load_session_runtime_view 必须完整保留历史消息。
#[test]
fn r6_rewind_with_referenced_node_preserves_history_in_runtime_view() {
    let _guard = serial_guard();
    let _rt = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    let workspace = isolated_workspace("referenced_rewind");
    let server = MockProviderServer::start(vec![
        "第一答".to_string(),
        "第二答".to_string(),
        "第三答".to_string(),
    ]);
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);

    let session_id = "rewind-referenced-session";
    let sink = RecordingSink::new();
    run_turn(&control_plane, &sink, session_id, "turn-1", "第一问");
    run_turn(&control_plane, &sink, session_id, "turn-2", "第二问");
    run_turn(&control_plane, &sink, session_id, "turn-3", "第三问");

    // 模拟长会话中历史节点被引用化（event_seq_range 已定，内嵌快照清空）：
    let points = control_plane.list_rewind_points(RewindPointsQuery {
        session_id: Some(session_id.to_string()),
    });
    assert_eq!(points.len(), 3, "三轮 ⇒ 三个撤回点");

    // 撤回到第 3 轮之前（保留第 1、2 轮）：
    let response = control_plane
        .rewind_session(RewindSessionCommand {
            session_id: Some(session_id.to_string()),
            node_id: points[0].node_id.clone(),
            mode: RewindMode::ConversationOnly,
        })
        .expect("撤回到第 3 轮之前必须成功");
    assert_eq!(response.user_message.as_deref(), Some("第三问"));
    assert_eq!(response.turn_index, 3);

    // 模拟 TUI `hydrate_state_from_session` 调用：
    let view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        session_id: Some(session_id.to_string()),
        turn_id: None,
        node_id: None,
        run_id: None,
    });

    let history: Vec<(String, String)> = view
        .session
        .history
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();

    assert_eq!(
        history,
        vec![
            ("user".to_string(), "第一问".to_string()),
            ("assistant".to_string(), "第一答".to_string()),
            ("user".to_string(), "第二问".to_string()),
            ("assistant".to_string(), "第二答".to_string()),
        ],
        "撤回后 load_session_runtime_view 必须能够读到第 1、2 轮的历史消息，绝不得为空！"
    );

    assert_eq!(
        view.session.history_cursor.branch_head_node_id.as_deref(),
        Some(points[1].node_id.as_str()),
        "分支头必须稳定停留在保留的第 2 轮节点，不得反弹回已取消的第 3 轮节点"
    );
}

/// ISSUE-20261111-004 单元守卫：ensure_history_graph 不得把已被截断/取消的末尾节点复活为主分支头。
#[test]
fn ensure_history_graph_does_not_resurrect_cancelled_head() {
    use pony_agent_core::agent::session::{
        ensure_history_graph, HistoryBranch, HistoryCursor, HistoryCursorMode, HistoryNode,
        HistoryNodeKind, SessionState, TraceMigrationState,
    };
    let session_id = "test-rewind-cancelled-head";
    let root_node = format!("{}-legacy-root", session_id);
    let node_1 = "node-turn-1".to_string();
    let node_2 = "node-turn-2".to_string();
    let mut session = SessionState {
        conversation_id: session_id.to_string(),
        title: "测试".to_string(),
        summary: "摘要".to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: TraceMigrationState::default(),
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 1000,
        history_nodes: vec![
            HistoryNode {
                node_id: root_node.clone(),
                session_id: session_id.to_string(),
                parent_node_id: None,
                branch_id: "main".to_string(),
                kind: HistoryNodeKind::Checkpoint,
                title: "测试".to_string(),
                summary: "摘要".to_string(),
                turn_count: 0,
                ..Default::default()
            },
            HistoryNode {
                node_id: node_1.clone(),
                session_id: session_id.to_string(),
                parent_node_id: Some(root_node.clone()),
                branch_id: "main".to_string(),
                kind: HistoryNodeKind::TurnCommitted,
                title: "第一轮".to_string(),
                summary: "第一轮摘要".to_string(),
                turn_count: 1,
                ..Default::default()
            },
            HistoryNode {
                node_id: node_2.clone(),
                session_id: session_id.to_string(),
                parent_node_id: Some(node_1.clone()),
                branch_id: "main".to_string(),
                kind: HistoryNodeKind::TurnCancelled,
                title: "第二轮(已撤销)".to_string(),
                summary: "已撤销".to_string(),
                turn_count: 2,
                ..Default::default()
            },
        ],
        history_branches: vec![HistoryBranch {
            branch_id: "main".to_string(),
            head_node_id: Some(node_1.clone()),
            base_node_id: Some(root_node.clone()),
            ..Default::default()
        }],
        history_cursor: HistoryCursor {
            session_id: session_id.to_string(),
            visible_node_id: Some(node_1.clone()),
            active_branch_id: Some("main".to_string()),
            branch_head_node_id: Some(node_1.clone()),
            workspace_node_id: Some(node_1.clone()),
            mode: HistoryCursorMode::Live,
            ..Default::default()
        },
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    };

    let changed = ensure_history_graph(&mut session);
    assert!(
        !changed,
        "当主分支头已指向有效的非取消节点且光标同步时，ensure_history_graph 不得将 changed 置为 true"
    );
    let main_branch = session
        .history_branches
        .iter()
        .find(|b| b.branch_id == "main")
        .expect("main branch exists");
    assert_eq!(
        main_branch.head_node_id.as_deref(),
        Some(node_1.as_str()),
        "已取消的 node_2 绝不能被篡改复活为分支头"
    );
    assert_eq!(
        session.history_cursor.branch_head_node_id.as_deref(),
        Some(node_1.as_str()),
        "光标的 branch_head_node_id 绝不能被重置为已取消的 node_2"
    );
}


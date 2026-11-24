//! wave-acp-delegate P0–P2 · 契约矩阵 **C6.1–C6.3**（turn 事件持久化）L2 验收（红相）。
//!
//! 契约真值源：`.dev-team/contract-matrix-acp-delegate-p0p2.md:26-28`
//! - **C6.1**（T-PERSIST-01）：每 step 结束增量落盘——第一个 step 结束（**不等到 turn_end**）时
//!   该 step 事件已在 `turn_events` 可读；终态仍写 `turn_end`；同一事件不重复。
//! - **C6.2**（T-PERSIST-02）：`turn_events.created_at_ms` = 事件**真实产生时间**
//!   （同批不得全部塌缩到 flush 时刻）；会话 `createdAt` = 会话创建时刻。
//! - **C6.3**（T-PERSIST-03）：模拟崩溃（已 flush、无 `turn_end`）后**重新打开同一 DB**：
//!   已 flush 事件可读、turn 呈未完成，不得整轮 0 行。
//!
//! 观测面（黑盒 + DB 只读）：`SessionStore::load_turn_events` 不暴露 `created_at_ms`，故 C6.2 只能
//! 在 `turn_events` 列上取证（同一库文件、独立只读连接）；C6.3 的「重新打开同 DB」= 丢弃内存
//! store 后新建 `SessionStore` + `SqliteSessionBackend` 读同一库（`fresh_store_events`）。
//! 「第一个 step 结束后」的采样点 = **mock provider 收到第二次（followup）请求的瞬间**
//! （step 0 已闭合、turn 未终态），采样在 mock 线程内完成，无 sleep 盲等。
//!
//! 红相基线（实施前）：生产持久化通道只在 terminal 事件 flush（`control_plane/mod.rs:1228` 起的
//! persist 闭包）且 `flush_events_tx` 的 `created_at_ms` 取整批一次 `now()`（`sqlite_session.rs:1513`）
//! ⇒ step 边界读到 0 行、时间戳滞后 turn 开始。
//! 纪律：AAA、真实断言、无 `#[ignore]`/无盲等/无恒真断言；临时目录随夹具 `Drop` 清理。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{SeparateTraceTableMode, SessionStore};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor, ToolResult};
use pony_agent_core::agent::turn_event::TurnEvent;
use pony_agent_core::agent::turn_flow::TurnEventSink;

/// C6.2 判据输入：首跳应答延迟，把「turn 开始」与「终态 flush」在时间轴上拉开。
const INITIAL_REPLY_DELAY_MS: u64 = 250;
const FINAL_TEXT: &str = "p0p2 persistence probe done";
const INITIAL_USAGE: &str = r#"{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}"#;
const FOLLOWUP_USAGE: &str = r#"{"prompt_tokens":20,"completion_tokens":6,"total_tokens":26}"#;

// ── 沙箱 / 夹具（进程级数据目录重定向，绝不碰真实 ~/.local/share/PonyAgent）──
static ISOLATE: Once = Once::new();

fn isolate_process_data_dir() {
    ISOLATE.call_once(|| {
        let sandbox = std::env::temp_dir()
            .join(format!("pony-agent-it-persist-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("XDG_CONFIG_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
}

/// 全局事件持久化通道是进程级单槽（`turn_flow::register_event_persist`）⇒ 三用例串行。

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "pony-persist-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }
    fn db(&self) -> PathBuf {
        self.root.join("sessions.db")
    }
    fn workspace(&self) -> PathBuf {
        let workspace = self.root.join("ws");
        std::fs::create_dir_all(&workspace).expect("create fixture workspace");
        workspace
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64
}

// ── DB 只读观测面 ──
#[derive(Clone, Debug)]
struct EventRow {
    seq: i64,
    event_type: String,
    payload: String,
    created_at_ms: i64,
}

fn open_ro(db: &Path) -> Option<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(db).ok()?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    Some(conn)
}

/// `turn_events` 行（seq 升序）；空库/未建表 ⇒ 空集合（即「0 行」这一红相事实）。
fn db_event_rows(db: &Path, session_id: &str) -> Vec<EventRow> {
    let Some(conn) = open_ro(db) else { return Vec::new() };
    let has_table: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='turn_events'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false);
    if !has_table {
        return Vec::new();
    }
    let Ok(mut statement) = conn.prepare(
        "SELECT seq, event_type, payload, created_at_ms FROM turn_events \
         WHERE session_id = ?1 ORDER BY seq",
    ) else {
        return Vec::new();
    };
    statement
        .query_map(rusqlite::params![session_id], |row| {
            Ok(EventRow {
                seq: row.get(0)?,
                event_type: row.get(1)?,
                payload: row.get(2)?,
                created_at_ms: row.get(3)?,
            })
        })
        .map(|mapped| mapped.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// 会话级 `createdAt`（C6.2 后半；该列可空 ⇒ 未落库时返回 None）。
fn db_session_created_at(db: &Path, session_id: &str) -> Option<i64> {
    use rusqlite::OptionalExtension;
    open_ro(db)?
        .query_row(
            "SELECT created_at_ms FROM normalized_sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
}

/// C6.3 的「重新打开同 DB」读面：全新 store + SQLite backend 读同一库（坏 payload fail loud）。
fn fresh_store_events(db: &Path, session_id: &str) -> Vec<(u64, TurnEvent)> {
    let store = SessionStore::with_backend(Box::new(SqliteSessionBackend::new_with_trace_mode(
        db.to_path_buf(),
        SeparateTraceTableMode::DualWrite,
    )));
    store
        .load_turn_events_checked(session_id, None)
        .unwrap_or_else(|error| panic!("C6.3：重新打开同 DB 的事件读取必须成功，实际 Err={error}"))
        .into_iter()
        .map(|(seq, _branch_id, event)| (seq, event))
        .collect()
}

// ── mock provider（回环，零外网）──
#[derive(Clone)]
struct Reply {
    body: String,
    delay: Duration,
}

/// step 边界采样：mock 收到 followup 请求（= 第一个 step 已闭合）瞬间读取 DB。
#[derive(Clone, Debug, Default)]
struct BoundarySample {
    rows: Vec<EventRow>,
    replayed: Vec<(u64, TurnEvent)>,
}

struct MockProvider {
    base_url: String,
    addr: String,
    boundary: Arc<Mutex<Option<BoundarySample>>>,
    requests: Arc<Mutex<usize>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockProvider {
    fn start(initial: Reply, followup: Reply, db: PathBuf, session_id: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock provider 回环绑定（零外网）");
        let addr = listener.local_addr().expect("mock provider 取本地地址");
        let boundary: Arc<Mutex<Option<BoundarySample>>> = Arc::new(Mutex::new(None));
        let requests = Arc::new(Mutex::new(0usize));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (boundary_for, requests_for) = (Arc::clone(&boundary), Arc::clone(&requests));
        let shutdown_for = Arc::clone(&shutdown);
        let session_id = session_id.to_string();
        let handle = std::thread::spawn(move || {
            while !shutdown_for.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let body = read_http_body(&mut stream);
                if body.is_null() {
                    continue; // 收尾唤醒连接
                }
                *requests_for
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
                let is_followup = has_tool_result(&body);
                if is_followup {
                    let mut slot = boundary_for
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if slot.is_none() {
                        // ★ step 边界采样：step 0 已闭合、turn 未终态。
                        *slot = Some(BoundarySample {
                            rows: db_event_rows(&db, &session_id),
                            replayed: fresh_store_events(&db, &session_id),
                        });
                    }
                }
                let reply = if is_followup { &followup } else { &initial };
                if !reply.delay.is_zero() {
                    std::thread::sleep(reply.delay);
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.body.as_bytes().len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(reply.body.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                drain_incoming(&mut stream);
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            addr: addr.to_string(),
            boundary,
            requests,
            shutdown,
            handle: Some(handle),
        }
    }

    fn boundary(&self) -> Option<BoundarySample> {
        self.boundary
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    fn requests(&self) -> usize {
        *self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_body(stream: &mut TcpStream) -> Value {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut expected: Option<usize> = None;
    loop {
        if let Some(total) = expected {
            if raw.len() >= total {
                break;
            }
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if expected.is_none() {
                    if let Some(index) = find_subslice(&raw, b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..index]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        expected = Some(index + 4 + length);
                    }
                }
            }
            Err(_) => break,
        }
    }
    match find_subslice(&raw, b"\r\n\r\n") {
        Some(index) => serde_json::from_slice(&raw[index + 4..]).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn drain_incoming(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut chunk = [0u8; 4096];
    while matches!(stream.read(&mut chunk), Ok(read) if read > 0) {}
}

/// 请求体里出现 `role=tool` ⇒ 工具之后的 followup（第一个 step 已闭合）。
fn has_tool_result(body: &Value) -> bool {
    body.get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages
                .iter()
                .any(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        })
}

fn tool_call_reply(delay: Duration) -> Reply {
    Reply {
        delay,
        body: json!({
            "id": "p0p2-persist-initial",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant", "content": null,
                    "tool_calls": [{
                        "id": "call_persist_probe", "type": "function",
                        "function": { "name": "workspace_list_files", "arguments": "{\"path\":\".\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": serde_json::from_str::<Value>(INITIAL_USAGE).expect("initial usage")
        })
        .to_string(),
    }
}

fn text_reply() -> Reply {
    Reply {
        delay: Duration::ZERO,
        body: json!({
            "id": "p0p2-persist-followup",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": FINAL_TEXT },
                "finish_reason": "stop"
            }],
            "usage": serde_json::from_str::<Value>(FOLLOWUP_USAGE).expect("followup usage")
        })
        .to_string(),
    }
}

// ── runtime / control plane 装配 ──
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

/// 固定工具执行器：不碰文件系统，只回一条可辨识结果（工具事件闭合的证据面）。
struct FixedExecutor;

impl ToolExecutor for FixedExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            output: "{\"entries\":[\"a.rs\"]}".to_string(),
            duration_ms: 1,
        }
    }
}

fn selection(base_url: String, tag: &str) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: tag.to_string(),
        provider_name: tag.to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "P0P2_PERSIST_API_KEY".to_string(),
        api_key: Some("p0p2-dummy-key".to_string()),
        model: "p0p2-persist-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            // buffered（非流式）：决策一次到齐，step 边界在 HTTP 请求层可辨识。
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn turn_input(session_id: &str) -> TurnInput {
    TurnInput {
        message: "p0p2 persistence probe".to_string(),
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

/// `SqliteSessionBackend`（turn_events 能力）+ 生产持久化通道（control plane 注册）的完整栈。
fn build_plane(db: &Path, base_url: String, workspace: &Path, tag: &str) -> HostControlPlane {
    let sessions = Arc::new(RwLock::new(SessionStore::with_backend(Box::new(
        SqliteSessionBackend::new_with_trace_mode(db.to_path_buf(), SeparateTraceTableMode::DualWrite),
    ))));
    let runtime = AgentRuntimeBuilder::new()
        .shared_session_store(sessions)
        .provider_resolver(Box::new(StaticResolver {
            selection: selection(base_url, tag),
        }))
        .tool_executor(Box::new(FixedExecutor))
        .workspace_root(workspace)
        .build();
    HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::new())
        .build()
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<String>>,
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, _payload: TurnStreamEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(name.to_string());
    }
}

fn run_turn(plane: &HostControlPlane, sink: &RecordingSink, session_id: &str, turn_id: &str) {
    plane.start_turn_stream(
        sink,
        StartTurnStreamCommand {
            turn_id: turn_id.to_string(),
            input: turn_input(session_id),
        },
    );
    let names = sink
        .events
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert!(
        names.iter().any(|name| name == "turn:completed"),
        "夹具轮必须以 turn:completed 收口（否则后续断言无意义），实际事件序列 = {names:?}"
    );
}

fn two_step_stack(label: &str, session_id: &str, delay_ms: u64, fixture: &Fixture) -> (MockProvider, HostControlPlane, RecordingSink) {
    let provider = MockProvider::start(
        tool_call_reply(Duration::from_millis(delay_ms)),
        text_reply(),
        fixture.db(),
        session_id,
    );
    let plane = build_plane(&fixture.db(), provider.base_url.clone(), &fixture.workspace(), label);
    (provider, plane, RecordingSink::default())
}

// ═════════════ C6.1 每 step 增量落盘 ═════════════
#[test]
fn c6_1_first_step_events_are_readable_at_step_boundary_without_duplicates() {
    let _serial = crate::env_lock::lock();
    isolate_process_data_dir();

    // Arrange + Act：首跳 tool_call（step 0 闭合）⇒ followup 正文（step 1 收口）。
    let session_id = "pa_c6_1_step_boundary";
    let fixture = Fixture::new("c61");
    let (provider, plane, sink) = two_step_stack("c61", session_id, 0, &fixture);
    run_turn(&plane, &sink, session_id, "turn-c61");

    // Assert 1（红线）：第一个 step 结束的瞬间，`turn_events` 已能读到该 step 的事件。
    let sample = provider
        .boundary()
        .unwrap_or_else(|| panic!("C6.1：mock 未收到 followup 请求（第二 provider 调用未发生）"));
    assert_eq!(provider.requests(), 2, "C6.1：夹具应为两跳调用（首跳 + followup）");
    assert!(
        !sample.rows.is_empty(),
        "C6.1：第一个 step 结束后（followup 请求到达时）turn_events 仍为 0 行 —— 事件只在 turn 终态整批 \
         flush，未做每 step 增量落盘（契约 C6.1）。session={session_id}"
    );

    // Assert 2：终态仍写 `turn_end`，且同一事件不重复（增量 flush 重放已提交事件会在此暴露）。
    let final_rows = db_event_rows(&fixture.db(), session_id);
    let terminal_count = final_rows.iter().filter(|row| row.event_type == "turn/end").count();
    let timeline: Vec<(i64, &str)> = final_rows.iter().map(|r| (r.seq, r.event_type.as_str())).collect();
    assert_eq!(terminal_count, 1, "C6.1：turn 终态必须且只能有 1 条 `turn/end`，实际 {terminal_count}，事件流 = {timeline:?}");
    let last = final_rows.last().expect("终态事件流非空");
    assert_eq!(last.event_type, "turn/end", "C6.1：`turn/end` 必须是事件流最后一条（seq={}），事件流 = {timeline:?}", last.seq);
    let mut seen: Vec<(&str, &str)> = Vec::new();
    let mut duplicated: Vec<(String, i64)> = Vec::new();
    for row in &final_rows {
        let key = (row.event_type.as_str(), row.payload.as_str());
        if seen.contains(&key) {
            duplicated.push((row.event_type.clone(), row.seq));
        } else {
            seen.push(key);
        }
    }
    assert!(
        duplicated.is_empty(),
        "C6.1：同一事件不得重复落盘（增量 flush 后终态批重放会重复），重复项(type,seq) = {duplicated:?}"
    );
}

// ═════════════ C6.2 事件时间戳 = 事件真实产生时间 ═════════════
#[test]
fn c6_2_event_created_at_is_production_time_not_flush_time() {
    let _serial = crate::env_lock::lock();
    isolate_process_data_dir();

    // Arrange：首跳应答人为延迟 ⇒「turn 开始」与「turn 终态」在时间轴上分开。
    let session_id = "pa_c6_2_event_time";
    let fixture = Fixture::new("c62");
    // Windows CI 以 `-D warnings` 构建测试：本用例只用 plane/sink，provider 绑定仅是为了保持
    // two_step_stack 的返回形状可读 ⇒ 前缀下划线，避免 unused variable 变硬错误。
    let (_provider, plane, sink) = two_step_stack("c62", session_id, INITIAL_REPLY_DELAY_MS, &fixture);

    // Act
    let turn_start_ms = now_ms();
    run_turn(&plane, &sink, session_id, "turn-c62");
    let turn_end_ms = now_ms();

    // Assert
    let rows = db_event_rows(&fixture.db(), session_id);
    assert!(rows.len() >= 3, "C6.2：一轮两跳 turn 至少应有 3 条事件，实际 {} 条", rows.len());
    let stamps: Vec<i64> = rows.iter().map(|row| row.created_at_ms).collect();
    let mut distinct = stamps.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() > 1,
        "C6.2：同批 turn_events.created_at_ms 全部相等（{distinct:?}）—— 时间戳塌缩到 flush 时刻，\
         不满足「created_at_ms = 事件真实产生时间」"
    );

    // 首条事件（turn/start / user/message）产生于 provider 首次应答之前 ⇒ 必须接近 turn 开始，
    // 而非终态 flush。INITIAL_REPLY_DELAY_MS 提供区分度（取一半为上界）。
    let first_stamp = stamps[0];
    let start_lag = first_stamp - turn_start_ms as i64;
    assert!(
        start_lag <= (INITIAL_REPLY_DELAY_MS as i64) / 2,
        "C6.2：首条事件时间戳滞后 turn 开始 {start_lag}ms（上界 {}ms）—— 事件时间戳取的是 flush 时刻 \
         而非产生时刻。时间轴 = [start={turn_start_ms}, end={turn_end_ms}]，stamps={stamps:?}",
        (INITIAL_REPLY_DELAY_MS as i64) / 2
    );
    for stamp in &stamps {
        assert!(
            *stamp >= turn_start_ms as i64 - 1_000 && *stamp <= turn_end_ms as i64 + 1_000,
            "C6.2：事件时间戳 {stamp} 必须落在本轮真实时间窗 [{turn_start_ms}, {turn_end_ms}]（±1s 容差）"
        );
    }

    // C6.2 后半（会话 createdAt = 会话创建时刻）：`normalized_sessions.created_at_ms` 可空；
    // 该列非空即为公开可读事实，必须不晚于首条事件（为空则本轮无可断言对象，报告中登记）。
    match db_session_created_at(&fixture.db(), session_id) {
        Some(session_created) => assert!(
            session_created <= first_stamp,
            "C6.2：会话 createdAt({session_created}) 不得晚于首条事件时间({first_stamp}) —— \
             createdAt 必须是会话创建时刻，不是首轮 flush 时刻"
        ),
        None => println!("[C6.2] normalized_sessions.created_at_ms IS NULL ⇒ 会话 createdAt 尚未落库"),
    }
}

// ═════════════ C6.3 崩溃后已 flush 事件可读且 turn 未完成 ═════════════
#[test]
fn c6_3_crashed_turn_keeps_flushed_events_and_reads_back_unfinished() {
    let _serial = crate::env_lock::lock();
    isolate_process_data_dir();

    // Arrange + Act：「崩溃」= step 边界之后不写 turn_end 就丢弃内存 store，
    // 改用全新 store 打开同一 DB（`fresh_store_events`，在 mock 线程的边界采样里完成）。
    let session_id = "pa_c6_3_crash_recovery";
    let fixture = Fixture::new("c63");
    let (provider, plane, sink) = two_step_stack("c63", session_id, 0, &fixture);
    run_turn(&plane, &sink, session_id, "turn-c63");

    // Assert
    let sample = provider
        .boundary()
        .unwrap_or_else(|| panic!("C6.3：未采到 step 边界快照（mock 未收到 followup 请求）"));
    assert!(
        !sample.rows.is_empty() && !sample.replayed.is_empty(),
        "C6.3：模拟崩溃后（重新打开同 DB）整轮事件 0 行 —— 已 flush 事件未持久化，重启即丢（契约 C6.3）。\
         turn_events 行数 = {}，fresh-store 回读 = {} 条",
        sample.rows.len(),
        sample.replayed.len()
    );
    let replay: Vec<(u64, &str)> = sample.replayed.iter().map(|(seq, e)| (*seq, e.type_name())).collect();
    assert!(
        !sample.replayed.iter().any(|(_, e)| matches!(e, TurnEvent::TurnEnd { .. })),
        "C6.3：崩溃快照（turn 未完成）里不得出现 turn/end，实际事件 = {replay:?}"
    );
    assert_ne!(
        replay.last().map(|(_, name)| *name).unwrap_or("none"),
        "turn/end",
        "C6.3：崩溃快照的最后一条事件不得是 turn/end（turn 未完成语义），实际事件 = {replay:?}"
    );
    assert!(
        sample
            .replayed
            .iter()
            .any(|(_, e)| matches!(e, TurnEvent::ToolCall { .. } | TurnEvent::ToolResult { .. })),
        "C6.3：已闭合第一个 step 的工具事件必须可回读（证明是 step 增量落盘而非 0 行），实际事件 = {replay:?}"
    );
}

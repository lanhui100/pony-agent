//! PA-174 **红相**验收：历史接线降级 + 1:1 followup 转录/错误保真 + 同步总 deadline +
//! 生效策略观测出口。
//!
//! 契约真值源：Lead 冻结的 PA-174 契约（2026-11-09）。本文件由独立 Test Agent 冻结；
//! Executor 只读，禁改。
//!
//! - **F4（最高优先）新轮次未接入历史**：`context.rs:1862-1870` 把历史 assistant 轮写成
//!   `ProviderRole::Developer` + 文本前缀 `"Previous assistant reply: "`；`provider/mod.rs:4112`
//!   的 `to_chat_role` 把它映射成 `"system"`；`openai_request_messages`（:4784）**不做还原**
//!   ⇒ 走 openai-completions 的 provider 收到的历史是「一串 system 消息，内容带前缀」。
//!   Anthropic 路径（:4570-4610）会剥前缀并产出 `role: assistant`，两条协议行为已分叉。
//! - **F1 1:1 followup**：包装 `retry_provider_followup_timeout_with_policy_deadline("followup_sync",…)`（当时无 deadline）
//!   （:1435/:1460）无 pristine-base 重置（构造器每次尝试回写 `*accumulated_messages`），
//!   Anthropic 1:1 臂（:1483）**完全没有重试壳**；耗尽时重试内核返回
//!   `extract_provider_error_detail`（:5097，空正文 ⇒ `""`）。
//! - **F2 同步路径缺跨 attempt 总 deadline**：需 `aggregate_sync_call_budget_ms(policy)` +
//!   `UPSTREAM_AGGREGATE_DEADLINE_MS_MAX = 180_000`。
//! - **F3 生效策略无观测出口**：需 `upstream_policy_report(raw, effective)` 纯函数。
//!
//! 判据口径（只锁可观察行为）：
//! 1. F4/F1 的证据面是 mock 端捕获的**真实请求体**（`messages` 数组的 role/content/条数），
//!    不读被测进程内部变量；
//! 2. F1 的错误面只走 **Anthropic 1:1 臂**——OpenAI/Responses 臂会把 `Err` 折成本地
//!    fallback（`or_else(local_tool_followup_fallback_response)`），观测不到原始错误；
//! 3. F2 的行为上界取宽松值（避免 flaky），数值边界以纯函数断言为准。
//!
//! 红相预期（本 wave 实现前）：
//! - F4：roles == ["system","system","user","system","user"] 且 content 带前缀；
//! - F1：OpenAI 1:1 第 2 次请求 messages 条数 3 → 5（转录重复追加）；
//!         Anthropic 1:1 臂 script=[503,200] 只发 1 次 HTTP；
//! - F2/F3：`aggregate_sync_call_budget_ms` / `upstream_policy_report` /
//!   `UPSTREAM_AGGREGATE_DEADLINE_MS_MAX` 不存在 ⇒ 编译红（E0425/E0433，签名锚点）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    aggregate_sync_call_budget_ms, upstream_policy_report, ProviderModelCapabilities,
    ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
    UPSTREAM_AGGREGATE_DEADLINE_MS_MAX,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderFailoverChain, ProviderManager, ProviderMessage, ProviderProtocol,
    ProviderRequest, ProviderRole,
};
use pony_agent_core::agent::context::{
    ContextStateQuery, ContextStateRetriever, DefaultContextStateRetriever, DefaultTurnContextBuilder,
    TurnContextBuilder,
};
use pony_agent_core::agent::session::{
    history_from_turn_events, SessionBackend, SessionSnapshot,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use pony_agent_core::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use pony_agent_core::agent::turn_event::TurnEvent;
use serde_json::{json, Value};

/// 503 报文：命中 `UPSTREAM_SERVER_ERROR_HINTS`（`502/503/504/408`）⇒ TransientRetryable。
const SERVER_ERROR_BODY: &str =
    "{\"error\":{\"type\":\"server_error\",\"message\":\"upstream temporarily unavailable\"}}";

/// OpenAI 200：`choices[0].message` 契约（decision / followup 共用）。
const OPENAI_OK_BODY: &str = "{\"id\":\"pa174-ok\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"pa174-ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}";

/// Anthropic 200：`content` 块数组（`extract_anthropic_output_text` 契约）。
const ANTHROPIC_OK_BODY: &str = "{\"id\":\"msg_pa174\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"pa174-anthropic-ok\"}],\"stop_reason\":\"end_turn\",\"usage\":{\"input_tokens\":5,\"output_tokens\":2}}";

/// 历史降级的事实字面量（`context.rs:1869`）：openai 路径必须剥掉它。
const HISTORY_PREFIX: &str = "Previous assistant reply: ";

// ── 回环 mock provider（真实请求体捕获：roles / content / 条数） ──────────────────────

#[derive(Clone, Copy)]
struct MockReply {
    status: u16,
    reason: &'static str,
    body: &'static str,
    content_type: &'static str,
    /// 应答前的固定延迟（F2 行为上界用；禁止概率等待）。
    delay: Duration,
}

/// 按剧本应答的 mock：第 N 个请求取第 N 条应答，超出后重复最后一条。
/// 捕获每次**真实到达**的请求体（解析后的 JSON）——所有断言的唯一证据面。
struct MockProvider {
    base_url: String,
    addr: String,
    requests: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockProvider {
    fn script(replies: Vec<MockReply>) -> Self {
        assert!(!replies.is_empty(), "PA-174：mock 剧本不得为空");
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("PA-174：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("PA-174：mock provider 必须能取本地地址")
            .port();
        let requests = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let bodies_for_thread = Arc::clone(&bodies);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                let raw = read_http_request(&mut stream);
                // 唤醒用空连接（Drop 发起）不得计入真实请求。
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                // 计数与请求体捕获都在 **accept 时**完成（先于任何延迟）⇒ 「恒不响应」场景下
                // `request_count()` 仍等于真实尝试数，不受应答延迟/排队影响。
                let index = requests_for_thread.fetch_add(1, Ordering::SeqCst);
                if let Ok(payload) = serde_json::from_str::<Value>(&raw) {
                    bodies_for_thread
                        .lock()
                        .expect("PA-174：请求体捕获锁不得中毒")
                        .push(payload);
                }
                let reply = replies[index.min(replies.len() - 1)];
                // 每连接独立线程：延迟不得阻塞 accept 循环（否则第二次尝试的计数会晚到，
                // 「恒不响应」用例的请求数会假性偏小）。
                std::thread::spawn(move || {
                    let mut stream = stream;
                    if !reply.delay.is_zero() {
                        std::thread::sleep(reply.delay);
                    }
                    let mut head = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.reason,
                        reply.content_type,
                        reply.body.as_bytes().len()
                    );
                    head.push_str(reply.body);
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.flush();
                });
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            bodies,
            shutdown,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn bodies(&self) -> Vec<Value> {
        self.bodies
            .lock()
            .expect("PA-174：请求体捕获锁不得中毒")
            .clone()
    }

    /// 每次请求体里 `messages` 数组的条数（顺序 = 到达顺序）。
    fn message_counts(&self) -> Vec<usize> {
        self.bodies()
            .iter()
            .map(|body| {
                body.get("messages")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or_else(|| panic!("PA-174：请求体必须携带 messages 数组，实际 {body}"))
            })
            .collect()
    }

    fn selection(&self, provider_name: &str, protocol: ProviderProtocol) -> ResolvedProviderSelection {
        provider_selection(self.base_url.clone(), provider_name, protocol)
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

fn read_http_request(stream: &mut TcpStream) -> String {
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

fn provider_selection(
    base_url: String,
    provider_name: &str,
    protocol: ProviderProtocol,
) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: provider_name.to_string(),
        provider_name: provider_name.to_string(),
        protocol,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "PA174_TEST_API_KEY".to_string(),
        api_key: Some("pa174-dummy-key".to_string()),
        model: format!("{provider_name}-model"),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn request_probe() -> ProviderRequest {
    ProviderRequest {
        model: "pa174-model".to_string(),
        input: vec![ProviderMessage::user("pa174 probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

fn tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "todo_write",
        description: "write todos",
        input_schema: json!({"type": "object"}),
    }]
}

fn single_call_and_result() -> (ToolCall, ToolResult) {
    (
        ToolCall {
            call_id: Some("call-pa174-1".to_string()),
            name: "todo_write".to_string(),
            arguments: json!({"todos": []}),
            plan: None,
        },
        ToolResult {
            tool_name: "todo_write".to_string(),
            status: "ok".to_string(),
            output: "todos written".to_string(),
            duration_ms: 3,
        },
    )
}

fn transient_503_reply() -> MockReply {
    MockReply {
        status: 503,
        reason: "Service Unavailable",
        body: SERVER_ERROR_BODY,
        content_type: "application/json",
        delay: Duration::ZERO,
    }
}

/// 网关常见形态：**空正文** 503/502（`Content-Length: 0`）。
fn empty_body_503_reply() -> MockReply {
    MockReply {
        status: 503,
        reason: "Service Unavailable",
        body: "",
        content_type: "application/json",
        delay: Duration::ZERO,
    }
}

fn openai_ok_reply() -> MockReply {
    MockReply {
        status: 200,
        reason: "OK",
        body: OPENAI_OK_BODY,
        content_type: "application/json",
        delay: Duration::ZERO,
    }
}

fn anthropic_ok_reply() -> MockReply {
    MockReply {
        status: 200,
        reason: "OK",
        body: ANTHROPIC_OK_BODY,
        content_type: "application/json",
        delay: Duration::ZERO,
    }
}

fn policy_with_attempts(decision_max_attempts: u32) -> UpstreamPolicy {
    UpstreamPolicy {
        decision_max_attempts,
        ..UpstreamPolicy::default()
    }
}

/// 驱动 1:1 followup 公开入口（`calls.len()==1` 分支），返回结果与调用方可见转录。
fn drive_one_to_one_followup(
    mock: &MockProvider,
    provider_name: &str,
    protocol: ProviderProtocol,
    decision_max_attempts: u32,
) -> (
    Result<pony_agent_core::agent::provider::ProviderResponse, String>,
    Vec<Value>,
) {
    let manager = ProviderManager::new(mock.selection(provider_name, protocol))
        .with_policy(policy_with_attempts(decision_max_attempts));
    let request = request_probe();
    let tools = tool_definitions();
    let (call, result) = single_call_and_result();
    let mut accumulated_messages: Vec<Value> = Vec::new();
    let outcome = manager.continue_with_tool_result(
        &request,
        &tools,
        &mut accumulated_messages,
        None,
        &call,
        &result,
    );
    (outcome, accumulated_messages)
}

// ── P1（最高优先）：重启后历史为零 —— 必须能从 durable `turn_events` 重建 ────────────

/// P1 会话 id 夹具（真实用户报告形态：`sessions.session_data = __index_only`、
/// `history: []`、`normalized_messages` 0 行，但 `turn_events` 有消息事件）。
const P1_SESSION: &str = "pa174-p1-session";

struct TempDbDir {
    dir: std::path::PathBuf,
}

impl TempDbDir {
    fn new(tag: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "pa174-p1-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("PA-174 P1：必须能创建临时会话库目录");
        Self { dir }
    }

    fn db_path(&self) -> std::path::PathBuf {
        self.dir.join("sessions.db")
    }
}

impl Drop for TempDbDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 写入「轻量索引行 + 零 normalized 行」的最小夹具（真实事故现场的关键形态）。
///
/// ⚠ 契约盲点（Lead 已核验）：既有夹具总是写 `normalized_*` 行，掩盖了
/// 「轻量索引行 + 零 normalized 行」这个态；本夹具**刻意只写 `sessions` + `turn_events`**。
fn seed_index_only_session_with_turn_events(db_path: &std::path::Path) {
    // 索引侧带**血缘信息**（activeBranchId = main）：真实事故会话是带 cursor 的
    // `__index_only` 轻量行，不是「裸空行」——B1 的「无血缘信息」用例另有独立夹具。
    let session_data = format!(
        r#"{{"__index_only":true,"conversationId":"{P1_SESSION}","title":"P1 会话","summary":"","history":[],"turnCount":0,"lastReferencedFile":null,"historyCursor":{{"sessionId":"{P1_SESSION}","activeBranchId":"main","cursorVersion":0,"eventWatermark":6}},"historyBranches":[{{"branchId":"main","sessionId":"{P1_SESSION}","label":"main","createdAtMs":0,"updatedAtMs":0}}]}}"#
    );
    let events: Vec<(i64, &str, &str, &str)> = vec![
        (1, "main", "user/message", r#"{"type":"user_message","turn_id":"turn-1","text":"上一轮问题"}"#),
        (2, "main", "assistant/chunk", r#"{"type":"assistant_chunk","turn_id":"turn-1","step":1,"text":"半截增量"}"#),
        (3, "main", "tool_result", r#"{"type":"tool_result","turn_id":"turn-1","step":1,"call_id":"call-1","status":"ok"}"#),
        (4, "main", "assistant/message", r#"{"type":"assistant_message","turn_id":"turn-1","step":1,"text":"上一轮回答"}"#),
        (5, "main", "turn_start", r#"{"type":"turn_start","turn_id":"turn-2"}"#),
        (6, "main", "user/message", r#"{"type":"user_message","turn_id":"turn-2","text":"   "}"#),
    ];
    seed_session_with_events(db_path, P1_SESSION, &session_data, &events);
}

/// 通用夹具：`sessions` 轻量索引行（给定 `session_data`）+ 指定 `turn_events`。
///
/// `events` = `(seq, branch_id, event_type, payload)`，按给定顺序写入（seq 唯一）。
fn seed_session_with_events(
    db_path: &std::path::Path,
    session_id: &str,
    session_data: &str,
    events: &[(i64, &str, &str, &str)],
) {
    let conn = rusqlite::Connection::open(db_path)
        .expect("PA-174：必须能打开临时会话库");
    conn.execute(
        "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data, version)
         VALUES (?1, ?2, ?3, ?4, 0)",
        rusqlite::params![session_id, "夹具会话", 1_700_000_000_000_i64, session_data],
    )
    .expect("PA-174：必须能写入轻量索引行");
    for (seq, branch_id, event_type, payload) in events {
        conn.execute(
            "INSERT OR REPLACE INTO turn_events
                (session_id, turn_id, branch_id, seq, event_type, payload, created_at_ms)
             VALUES (?1, 'turn-1', ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                session_id,
                branch_id,
                seq,
                event_type,
                payload,
                1_700_000_000_000_i64
            ],
        )
        .expect("PA-174：必须能写入 turn_events");
    }
}

/// P1-1（纯函数）：`history_from_turn_events` 按 seq 升序把消息事件还原成
/// `user` / `assistant` 历史；忽略其它事件类型与空文本。
///
/// 参数类型锚点：契约写作 `TurnEventLike`，代码库暂无该类型 ⇒ 本测试用现成的
/// `pub enum TurnEvent`（`turn_event.rs:40`，`serde(tag="type")`，正是 `turn_events.payload`
/// 的落库类型）。「按 seq 升序」由装载侧 `ORDER BY seq` 保证，切片顺序即 seq 顺序。
#[test]
fn p1_history_is_rebuilt_from_turn_events() {
    // ── Arrange：故意乱序放入非消息事件，验证「只取消息事件 + 保序」。 ──────────────
    let events = vec![
        TurnEvent::UserMessage {
            turn_id: "turn-1".to_string(),
            text: "问题一".to_string(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantChunk {
            turn_id: "turn-1".to_string(),
            step: 1,
            text: "半截增量".to_string(),
        },
        TurnEvent::ToolResult {
            turn_id: "turn-1".to_string(),
            step: 1,
            call_id: "call-1".to_string(),
            result: Some("ok".to_string()),
            error: None,
            status: Some("ok".to_string()),
            duration_ms: Some(3),
            artifacts: None,
            capability_invocation: None,
        },
        TurnEvent::AssistantMessage {
            turn_id: "turn-1".to_string(),
            step: 1,
            text: "回答一".to_string(),
            reasoning_content: None,
            usage: None,
            chunk_missing: None,
        },
        TurnEvent::TurnStart {
            turn_id: "turn-2".to_string(),
        },
        TurnEvent::UserMessage {
            turn_id: "turn-2".to_string(),
            text: "   ".to_string(),
            attachments: Vec::new(),
        },
        TurnEvent::UserMessage {
            turn_id: "turn-2".to_string(),
            text: "问题二".to_string(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: "turn-2".to_string(),
            step: 1,
            text: "回答二".to_string(),
            reasoning_content: Some("思考".to_string()),
            usage: None,
            chunk_missing: None,
        },
    ];

    // ── Act ────────────────────────────────────────────────────────────────────
    let history = history_from_turn_events(&events);

    // ── Assert ─────────────────────────────────────────────────────────────────
    let triples: Vec<(&str, &str)> = history
        .iter()
        .map(|message| (message.role.as_str(), message.content.as_str()))
        .collect();
    assert_eq!(
        triples,
        vec![
            ("user", "问题一"),
            ("assistant", "回答一"),
            ("user", "问题二"),
            ("assistant", "回答二"),
        ],
        "PA-174 P1：必须只取 user/message 与 assistant/message（空文本与 chunk/tool/trace 事件忽略），\
         实际 {triples:?}"
    );

    // 空事件 ⇒ 空历史（不得 panic、不得造出默认消息）。
    assert!(
        history_from_turn_events(&[]).is_empty(),
        "PA-174 P1：空事件流必须重建出空历史"
    );
}

/// P1-2（装载侧，端到端）：**轻量索引行 + 零 normalized 行 + 有消息事件的 turn_events**
/// ⇒ 装载后历史必须非空（不得返回空壳）。
///
/// 走真实装载路径：临时 sqlite 文件（`SqliteSessionBackend::new` 建 schema）→ 写夹具 →
/// `SessionStore::with_backend(...)` 装载 → `sessions_view()` / `snapshot()` 断言历史。
#[test]
fn p1_loader_rebuilds_history_from_turn_events_when_projection_is_empty() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let temp = TempDbDir::new("loader");
    let db_path = temp.db_path();
    let backend = SqliteSessionBackend::new(db_path.clone());
    let seeded = backend.load_store();
    assert!(
        seeded.is_some(),
        "PA-174 P1 前置：临时库首读必须成功（schema 初始化）"
    );
    seed_index_only_session_with_turn_events(&db_path);

    // ── Act：真实装载（唯一内容来源只剩 turn_events）。 ──────────────────────────
    // 只走 `SqliteSessionBackend::load_store()`（装载侧本体），不经 `SessionStore::with_backend`
    // ——后者会触发 workspace 引导等进程级副作用，与「历史是否重建」无关。
    let loaded = backend
        .load_store()
        .expect("PA-174 P1：临时库必须可装载（`SessionBackend::load_store`）");
    let view = loaded.sessions_view();
    let state = view.get(P1_SESSION).unwrap_or_else(|| {
        panic!(
            "PA-174 P1：装载后必须仍有该会话（轻量索引行不得被丢弃），实际会话={:?}",
            view.keys().collect::<Vec<_>>()
        )
    });

    // ── Assert ─────────────────────────────────────────────────────────────────
    let triples: Vec<(String, String)> = state
        .history
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();
    assert_eq!(
        triples,
        vec![
            ("user".to_string(), "上一轮问题".to_string()),
            ("assistant".to_string(), "上一轮回答".to_string()),
        ],
        "PA-174 P1：投影侧全空但 turn_events 有消息事件 ⇒ 装载必须从事件日志重建历史\
         （当前缺陷：返回空壳 `history: []`，重启后历史为零），实际 {triples:?}"
    );
    assert!(
        state
            .history
            .iter()
            .all(|message| !message.content.trim().is_empty()),
        "PA-174 P1：重建历史不得包含空文本项，实际 {triples:?}"
    );
}

/// P1-3（端到端）：用重建后的历史 `build_request`，请求消息里必须出现上一轮
/// user 与 assistant（conversation carry 非空）。
#[test]
fn p1_rebuilt_history_reaches_request() {
    // ── Arrange：重建历史并注入内存会话存储。 ────────────────────────────────────
    let rebuilt = history_from_turn_events(&[
        TurnEvent::UserMessage {
            turn_id: "turn-1".to_string(),
            text: "上一轮问题".to_string(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: "turn-1".to_string(),
            step: 1,
            text: "上一轮回答".to_string(),
            reasoning_content: None,
            usage: None,
            chunk_missing: None,
        },
    ]);
    assert_eq!(
        rebuilt.len(),
        2,
        "PA-174 P1 前置：纯函数必须重建出 2 条历史，实际 {} 条",
        rebuilt.len()
    );

    // 直接构造快照（不落盘、不触 workspace 引导等进程级副作用，仅验证 carry 语义）。
    let snapshot = SessionSnapshot {
        conversation_id: P1_SESSION.to_string(),
        title: "P1 会话".to_string(),
        summary: String::new(),
        history: rebuilt,
        attachment_assets: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: Default::default(),
        run_control_audit_summary: Default::default(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        resolved_node_id: None,
        latest_node_id: None,
        env_info: None,
        workspace_id: None,
    };

    // ── Act：走真实上下文构建入口。 ───────────────────────────────────────────────
    let retrieved = DefaultContextStateRetriever.retrieve(ContextStateQuery {
        user_message: "新一轮问题",
        images: &[],
        workspace_mode: None,
        session: &snapshot,
        run: None,
        checkpoint: None,
    });
    let mock = MockProvider::script(vec![openai_ok_reply()]);
    let provider = ProviderManager::new(
        mock.selection("pa174-p1-request", ProviderProtocol::OpenAiCompletions),
    )
    .with_policy(policy_with_attempts(2));
    let request =
        DefaultTurnContextBuilder.build_request("agent", &provider, &retrieved, &[]);

    // ── Assert：conversation carry 必须含先前 user + assistant。 ────────────────────
    let carries_user = request
        .input
        .iter()
        .any(|message| message.content.contains("上一轮问题"));
    let carries_assistant = request
        .input
        .iter()
        .any(|message| message.content.contains("上一轮回答"));
    assert!(
        carries_user && carries_assistant,
        "PA-174 P1：重建历史必须进入请求（carry 非空）——user={carries_user} / \
         assistant={carries_assistant}；实际消息={:?}",
        request
            .input
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
    );
}

// ── B1：重建历史必须尊重**分支血缘**（两位审查 BLOCKING） ────────────────────────────

const B1_SESSION: &str = "pa174-b1-session";

/// B1 夹具：索引里明确「当前血缘 = main」，`turn_events` 混入已放弃的 `fork-2` 分支。
fn seed_branch_lineage_fixture(db_path: &std::path::Path) {
    let session_data = format!(
        r#"{{"__index_only":true,"conversationId":"{B1_SESSION}","title":"B1 会话","summary":"","history":[],"turnCount":0,"lastReferencedFile":null,"historyCursor":{{"sessionId":"{B1_SESSION}","visibleNodeId":"node-main-3","activeBranchId":"main","branchHeadNodeId":"node-main-3","cursorVersion":0,"eventWatermark":6}},"historyBranches":[{{"branchId":"main","sessionId":"{B1_SESSION}","headNodeId":"node-main-3","label":"main","createdAtMs":0,"updatedAtMs":0}},{{"branchId":"fork-2","sessionId":"{B1_SESSION}","baseNodeId":"node-main-1","headNodeId":"node-fork-2","forkedFromBranchId":"main","forkedFromNodeId":"node-main-1","label":"fork-2","createdAtMs":0,"updatedAtMs":0}}]}}"#
    );
    let events: Vec<(i64, &str, &str, &str)> = vec![
        (1, "main", "user/message", r#"{"type":"user_message","turn_id":"turn-m1","text":"Q1"}"#),
        (2, "main", "assistant/message", r#"{"type":"assistant_message","turn_id":"turn-m1","step":1,"text":"A1"}"#),
        (3, "fork-2", "user/message", r#"{"type":"user_message","turn_id":"turn-f2","text":"Q2"}"#),
        (4, "fork-2", "assistant/message", r#"{"type":"assistant_message","turn_id":"turn-f2","step":1,"text":"A2"}"#),
        (5, "main", "user/message", r#"{"type":"user_message","turn_id":"turn-m3","text":"Q3"}"#),
        (6, "main", "assistant/message", r#"{"type":"assistant_message","turn_id":"turn-m3","step":1,"text":"A3"}"#),
    ];
    seed_session_with_events(db_path, B1_SESSION, &session_data, &events);
}

/// B1-1（BLOCKING）：重建只能取**当前血缘（main）**的事件；已放弃的 `fork-2` 分支
/// （Q2/A2）不得混入——否则用户会看到一条自己已经切走的分支的对话。
#[test]
fn b1_rebuild_respects_branch_lineage() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let temp = TempDbDir::new("b1-lineage");
    let db_path = temp.db_path();
    let backend = SqliteSessionBackend::new(db_path.clone());
    assert!(
        backend.load_store().is_some(),
        "PA-174 B1 前置：临时库首读必须成功（schema 初始化）"
    );
    seed_branch_lineage_fixture(&db_path);

    // ── Act ────────────────────────────────────────────────────────────────────
    let loaded = backend.load_store().expect("PA-174 B1：临时库必须可装载");
    let view = loaded.sessions_view();
    let state = view.get(B1_SESSION).unwrap_or_else(|| {
        panic!(
            "PA-174 B1：装载后必须仍有该会话，实际 {:?}",
            view.keys().collect::<Vec<_>>()
        )
    });

    // ── Assert：只含 main 的 Q1/A1/Q3/A3（4 条），且顺序不变。 ─────────────────────
    let triples: Vec<(String, String)> = state
        .history
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();
    assert_eq!(
        triples,
        vec![
            ("user".to_string(), "Q1".to_string()),
            ("assistant".to_string(), "A1".to_string()),
            ("user".to_string(), "Q3".to_string()),
            ("assistant".to_string(), "A3".to_string()),
        ],
        "PA-174 B1：重建必须只取当前血缘 main 的消息（放弃分支 fork-2 的 Q2/A2 不得混入），\
         实际 {} 条：{triples:?}",
        triples.len()
    );
}

/// B1-2：索引**完全无血缘信息**（无 `historyCursor`/无分支表）⇒ 宁可**不重建**
/// （历史为空、且不报错），也**不得**把全部分支合并成一条历史。
#[test]
fn b1_rebuild_without_lineage_info_yields_empty_history() {
    // ── Arrange：裸 `__index_only` 行（没有 cursor / branches）。 ─────────────────
    let temp = TempDbDir::new("b1-no-lineage");
    let db_path = temp.db_path();
    let backend = SqliteSessionBackend::new(db_path.clone());
    assert!(
        backend.load_store().is_some(),
        "PA-174 B1 前置：临时库首读必须成功（schema 初始化）"
    );
    let session_data = format!(
        r#"{{"__index_only":true,"conversationId":"{B1_SESSION}","title":"B1 会话","summary":"","history":[],"turnCount":0,"lastReferencedFile":null}}"#
    );
    let events: Vec<(i64, &str, &str, &str)> = vec![
        (1, "main", "user/message", r#"{"type":"user_message","turn_id":"turn-m1","text":"Q1"}"#),
        (2, "fork-2", "user/message", r#"{"type":"user_message","turn_id":"turn-f2","text":"Q2"}"#),
    ];
    seed_session_with_events(&db_path, B1_SESSION, &session_data, &events);

    // ── Act ────────────────────────────────────────────────────────────────────
    let loaded = backend.load_store().expect("PA-174 B1：无血缘信息时装载也不得失败");
    let view = loaded.sessions_view();
    let state = view
        .get(B1_SESSION)
        .unwrap_or_else(|| panic!("PA-174 B1：装载后必须仍有该会话（不得丢弃轻量行）"));

    // ── Assert：空历史（不合并全分支），且装载不 panic / 会话仍在。 ──────────────────
    assert!(
        state.history.is_empty(),
        "PA-174 B1：索引无血缘信息 ⇒ 历史必须为空（宁可不重建，不得合并全分支），\
         实际 {} 条：{:?}",
        state.history.len(),
        state
            .history
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
    );
}
// ── F4：历史接线——上一轮 assistant 回复不得降级成 system 文本 ─────────────────────

/// F4（最高优先）：`openai_request_messages` 必须把「历史 assistant 轮」
/// （`Developer` + `"Previous assistant reply: "` 前缀）**还原**为 `role: assistant` +
/// 去前缀正文，且保序、system 文本不得残留前缀字面量。
#[test]
fn f4_openai_history_restores_assistant_role_and_strips_prefix() {
    // ── Arrange：历史 = sys / 上轮 assistant(A1) / u1 / 上轮 assistant(A2) / u2 ──
    let mock = MockProvider::script(vec![openai_ok_reply()]);
    let manager = ProviderManager::new(
        mock.selection("pa174-history", ProviderProtocol::OpenAiCompletions),
    )
    .with_policy(policy_with_attempts(2));
    let request = ProviderRequest {
        model: "pa174-history-model".to_string(),
        input: vec![
            ProviderMessage {
                role: ProviderRole::System,
                content: "sys".to_string(),
            },
            ProviderMessage {
                role: ProviderRole::Developer,
                content: format!("{HISTORY_PREFIX}A1"),
            },
            ProviderMessage::user("u1"),
            ProviderMessage {
                role: ProviderRole::Developer,
                content: format!("{HISTORY_PREFIX}A2"),
            },
            ProviderMessage::user("u2"),
        ],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    };

    // ── Act ────────────────────────────────────────────────────────────────────
    let decision = manager.decide_with_tools(&request, &[]);
    let bodies = mock.bodies();

    // ── Assert ─────────────────────────────────────────────────────────────────
    decision.expect("PA-174 F4 前置：mock 200 ⇒ decision 必须成功，否则请求体断言无意义");
    assert_eq!(
        bodies.len(),
        1,
        "PA-174 F4：必须恰好 1 次真实请求，实际 {}",
        bodies.len()
    );
    let messages = bodies[0]
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("PA-174 F4：请求体必须携带 messages 数组，实际 {}", bodies[0]));

    let roles: Vec<&str> = messages
        .iter()
        .map(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("PA-174 F4：每条消息必须有字符串 role，实际 {message}"))
        })
        .collect();
    assert_eq!(
        roles,
        vec!["system", "assistant", "user", "assistant", "user"],
        "PA-174 F4：历史 assistant 轮必须还原成 `assistant`（当前缺陷：Developer→system ⇒ \
         一串 system），实际 roles={roles:?}"
    );
    assert_eq!(
        messages[1].get("content").and_then(Value::as_str),
        Some("A1"),
        "PA-174 F4：assistant 消息正文必须剥掉 `{HISTORY_PREFIX}` 前缀，实际 {:?}",
        messages[1].get("content")
    );
    assert_eq!(
        messages[3].get("content").and_then(Value::as_str),
        Some("A2"),
        "PA-174 F4：第 2 条历史 assistant 正文必须剥掉前缀，实际 {:?}",
        messages[3].get("content")
    );
    // 顺序不变量：user/assistant 交替序列必须与 input 同序（用 content 再钉一次）。
    assert_eq!(
        messages[2].get("content").and_then(Value::as_str),
        Some("u1"),
        "PA-174 F4：顺序不得重排（u1 必须仍在第 3 条）"
    );
    assert_eq!(
        messages[4].get("content").and_then(Value::as_str),
        Some("u2"),
        "PA-174 F4：顺序不得重排（u2 必须仍在第 5 条）"
    );

    // system 文本里不得出现降级前缀字面量（F4 行为 3）。
    let serialized = serde_json::to_string(messages).expect("PA-174 F4：messages 必须可序列化");
    assert!(
        !serialized.contains("Previous assistant reply"),
        "PA-174 F4：请求体不得出现 `Previous assistant reply` 字面量（否则历史被降级成 \
         system 文本），实际请求体：{serialized}"
    );
}

// ── F1：1:1 followup —— 重试不得污染转录 / 不得降级错误 / Anthropic 臂也要重试 ────────

/// F1-a（OpenAI 1:1 臂，成功路径可见转录）：重试的每次 attempt 必须从 pristine base 重建，
/// 成功后 `accumulated_messages` 恰好 = base + 1 次追加。
///
/// 基数：base = 1 条 user；每次追加 1 assistant + 1 tool ⇒ 每次请求 3 条。
#[test]
fn f1_one_to_one_retry_does_not_duplicate_transcript() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let mock = MockProvider::script(vec![transient_503_reply(), openai_ok_reply()]);
    const EXPECTED_MESSAGES: usize = 3; // base(1) + assistant(1) + tool(1)
    const DUPLICATED_MESSAGES: usize = 5; // 复用上次写回转录 ⇒ 3 + 2

    // ── Act ────────────────────────────────────────────────────────────────────
    let (result, transcript) = drive_one_to_one_followup(
        &mock,
        "pa174-f1-openai",
        ProviderProtocol::OpenAiCompletions,
        2,
    );
    let counts = mock.message_counts();

    // ── Assert ─────────────────────────────────────────────────────────────────
    assert_eq!(
        counts.len(),
        2,
        "PA-174 F1：transient 失败后必须重发 1 次（共 2 次真实请求），实际各次条数={counts:?}"
    );
    assert_eq!(
        counts[1], counts[0],
        "PA-174 F1：1:1 重试必须从 pristine base 重建——第 2 次请求 messages 条数必须与第 1 次\
         相同（第 1 次 {} 条 / 第 2 次 {} 条；若为 {DUPLICATED_MESSAGES} 条 ⇒ 转录被重复追加）",
        counts[0], counts[1]
    );
    assert_eq!(
        counts[0], EXPECTED_MESSAGES,
        "PA-174 F1：第 1 次请求 messages 必须 = base(1) + assistant(1) + tool(1) = \
         {EXPECTED_MESSAGES}，实际 {}",
        counts[0]
    );
    result.expect("PA-174 F1：首次 503 后第二次 200 必须返回 Ok");
    assert_eq!(
        transcript.len(),
        EXPECTED_MESSAGES,
        "PA-174 F1：成功后 `accumulated_messages` 必须恰好 = base + 1 次追加\
         （{EXPECTED_MESSAGES} 条），实际 {} 条（{DUPLICATED_MESSAGES} 条 ⇒ 每次重试都追加）",
        transcript.len()
    );
}

/// F1-b（Anthropic 1:1 臂）：该臂**必须**与 OpenAI 臂同等重试 ——
/// script [503, 200] ⇒ 恰好 2 次真实 HTTP 且 `Ok`。
///
/// 现状缺陷：`provider/mod.rs:1483` 的 Anthropic 1:1 臂直接调用
/// `send_anthropic_tool_followup_request`，**没有** retry 壳 ⇒ 单次 503 即终态。
#[test]
fn f1_anthropic_one_to_one_arm_retries_transient_failure() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let mock = MockProvider::script(vec![transient_503_reply(), anthropic_ok_reply()]);

    // ── Act ────────────────────────────────────────────────────────────────────
    let (result, _transcript) = drive_one_to_one_followup(
        &mock,
        "pa174-f1-anthropic",
        ProviderProtocol::AnthropicMessages,
        2,
    );
    let hits = mock.request_count();

    // ── Assert ─────────────────────────────────────────────────────────────────
    assert_eq!(
        hits, 2,
        "PA-174 F1：Anthropic 1:1 臂必须与 OpenAI 臂同等重试（首次 503 后恰好 1 次重发 = \
         2 次真实 HTTP），实际 {hits} 次"
    );
    let response = result.expect(
        "PA-174 F1：Anthropic 1:1 臂首次 503 后第二次 200 必须返回 Ok（该臂不得折成本地 fallback）",
    );
    assert_eq!(
        response.output_text, "pa174-anthropic-ok",
        "PA-174 F1：重试成功后必须返回第二次的真实产出，实际 {:?}",
        response.output_text
    );
}

/// F1-c（Anthropic 1:1 臂，错误保真）：耗尽后必须返回**原始错误**（含状态码），
/// 空正文时也不得返回空串。
#[test]
fn f1_anthropic_exhausted_error_keeps_status_code() {
    for (label, reply) in [
        ("非空正文", transient_503_reply()),
        ("空正文", empty_body_503_reply()),
    ] {
        // ── Arrange ────────────────────────────────────────────────────────────
        let mock = MockProvider::script(vec![reply]);

        // ── Act ────────────────────────────────────────────────────────────────
        let (result, _transcript) = drive_one_to_one_followup(
            &mock,
            &format!("pa174-f1-anthropic-err-{}", label.len()),
            ProviderProtocol::AnthropicMessages,
            2,
        );
        let hits = mock.request_count();

        // ── Assert ─────────────────────────────────────────────────────────────
        let error = result.expect_err(
            "PA-174 F1：恒 503 必须返回 Err（Anthropic 臂不得折叠成 fallback Ok）",
        );
        assert_eq!(
            hits, 2,
            "PA-174 F1 前置（{label}）：attempts=2 ⇒ 必须恰好 2 次真实 HTTP，实际 {hits} 次"
        );
        assert!(
            !error.trim().is_empty(),
            "PA-174 F1（{label}）：耗尽错误不得为空串（空正文时也必须保留状态码/耗时），\
             实际 {error:?}"
        );
        assert!(
            error.contains("503"),
            "PA-174 F1（{label}）：耗尽错误必须保留原始状态码 `503`（禁止降级成纯正文），\
             实际 {error:?}"
        );
    }
}

// ── B2：聚合预算耗尽不得丢失真实错误、也不得阻断换手（审查 BLOCKING） ─────────────────

/// B2（BLOCKING）：聚合 deadline 耗尽时，终态错误必须保留真实原因（状态码/超时），
/// 且**不得阻断换手**（候选链必须被真实尝试）。
///
/// 缺陷机制：`with_aggregate_sync_deadline` 让预算耗尽后的下一次 attempt 返回内部文案
/// `"failover budget exhausted before attempt; remaining=0ms; no request sent"`；
/// 该文案不命中 `failover_eligible` 码表 ⇒ 换手直接放弃（候选 0 次 HTTP），
/// 且用户看到的终态错误完全不含真实失败原因。
#[test]
fn b2_aggregate_exhaustion_keeps_real_error_and_allows_failover() {
    let policy = UpstreamPolicy {
        request_timeout_ms: 1_000,
        decision_max_attempts: 5,
        retry_budget_ms: 3_600,
    };
    // 每次 attempt 都「慢 503」（900ms < 1000ms 单次超时 ⇒ 真实收到 503，而非客户端超时）。
    let slow_503 = MockReply {
        status: 503,
        reason: "Service Unavailable",
        body: SERVER_ERROR_BODY,
        content_type: "application/json",
        delay: Duration::from_millis(900),
    };

    // ── Act 1：带候选链 —— 耗尽后必须仍尝试候选。 ────────────────────────────────
    let primary = MockProvider::script(vec![slow_503]);
    let candidate = MockProvider::script(vec![openai_ok_reply()]);
    let manager = ProviderManager::new(
        primary.selection("pa174-b2-primary", ProviderProtocol::OpenAiCompletions),
    )
    .with_policy(policy.clone())
    .with_failover(ProviderFailoverChain::new(vec![candidate.selection(
        "pa174-b2-candidate",
        ProviderProtocol::OpenAiCompletions,
    )]));
    let with_chain = manager.decide_with_tools(&request_probe(), &[]);
    let candidate_hits = candidate.request_count();

    // ── Assert 1：候选必须收到 ≥1 次真实请求（内部预算耗尽不得阻断换手）。 ──────────
    assert!(
        candidate_hits >= 1,
        "PA-174 B2：聚合预算耗尽后仍必须尝试候选（换手不得被内部预算文案短路），\
         实际候选收到 {candidate_hits} 次真实 HTTP"
    );
    assert!(
        with_chain.is_ok(),
        "PA-174 B2：候选健康 ⇒ 换手成功必须返回 Ok，实际 {:?}",
        with_chain.as_ref().err()
    );

    // ── Act 2：去掉候选链 —— 终态错误必须保留真实原因。 ───────────────────────────
    let primary_only = MockProvider::script(vec![slow_503]);
    let manager = ProviderManager::new(
        primary_only.selection("pa174-b2-primary-only", ProviderProtocol::OpenAiCompletions),
    )
    .with_policy(policy);
    let error = manager
        .decide_with_tools(&request_probe(), &[])
        .expect_err("PA-174 B2：恒 503 ⇒ 无候选链时必须返回 Err（禁止伪造成功）");

    // ── Assert 2：真实失败事实保留，且不得是内部预算文案。 ──────────────────────────
    let lowered = error.to_ascii_lowercase();
    assert!(
        error.contains("503") || lowered.contains("timeout") || lowered.contains("timed out"),
        "PA-174 B2：终态错误必须保留真实失败原因（状态码 503 / 超时），实际：{error}"
    );
    assert!(
        !lowered.contains("failover budget exhausted"),
        "PA-174 B2：终态错误不得退化成内部预算文案（用户看不到真实原因），实际：{error}"
    );
}
// ── F2：同步路径必须有跨 attempt 总 deadline ─────────────────────────────────────────

/// F2 纯函数面：`aggregate_sync_call_budget_ms(policy)` =
/// `min(request_timeout_ms * decision_max_attempts + retry_budget_ms, MAX)`。
///
/// 数值边界（冻结）：
/// - `(45000, 5, 30000)` ⇒ 45000×5+30000 = 255000 > 180000 ⇒ **封顶 180000**；
/// - `(15000, 2, 5000)`  ⇒ 15000×2+5000  = 35000 ⇒ **不封顶 35000**。
#[test]
fn f2_aggregate_sync_call_budget_is_capped_and_computed() {
    // A1：常量冻结。
    assert_eq!(
        UPSTREAM_AGGREGATE_DEADLINE_MS_MAX, 180_000,
        "PA-174 F2：跨 attempt 总 deadline 上限必须冻结为 180000ms，实际 {}",
        UPSTREAM_AGGREGATE_DEADLINE_MS_MAX
    );

    // A2：默认策略封顶。
    let default_budget = aggregate_sync_call_budget_ms(&UpstreamPolicy::default());
    assert_eq!(
        default_budget, 180_000,
        "PA-174 F2：默认 (45000,5,30000) ⇒ 45000×5+30000=255000 必须被 180000 封顶，实际 {default_budget}"
    );

    // A3：不封顶组合逐字相等。
    let small_policy = UpstreamPolicy {
        request_timeout_ms: 15_000,
        decision_max_attempts: 2,
        retry_budget_ms: 5_000,
    };
    let small_budget = aggregate_sync_call_budget_ms(&small_policy);
    assert_eq!(
        small_budget, 35_000,
        "PA-174 F2：(15000,2,5000) ⇒ 15000×2+5000=35000，实际 {small_budget}"
    );
}

/// F2 行为面：同步决策必须有**跨 attempt 聚合 deadline**，且必须能区分「有 / 无」。
///
/// 参数选择（为什么这个组合能区分）：
/// policy = (request_timeout_ms=1000, decision_max_attempts=5, retry_budget_ms=1) ⇒
/// `aggregate_sync_call_budget_ms = min(1000×5+1, 180000) = 5001ms`。
/// 逐 attempt 超时合计 5×1000 = 5000ms，退避 500/1000/2000 ⇒ **无聚合 deadline** 时总墙钟
/// 明显超过 5001ms 且 5 次 attempt 全跑；**有聚合 deadline** 时剩余预算在约第 5 次前耗尽。
///
/// 判据只取两端可观察量：真实请求数（accept 计数，不受应答延迟影响）与总墙钟。
#[test]
fn f2_sync_call_wall_clock_is_bounded_across_attempts() {
    // ── Arrange：mock **恒不响应**（延迟 1500ms > 单次超时 1000ms ⇒ 每次 attempt 必然超时）。 ──
    let mock = MockProvider::script(vec![MockReply {
        status: 503,
        reason: "Service Unavailable",
        body: SERVER_ERROR_BODY,
        content_type: "application/json",
        delay: Duration::from_millis(1_500),
    }]);
    let manager = ProviderManager::new(
        mock.selection("pa174-f2-budget", ProviderProtocol::OpenAiCompletions),
    )
    .with_policy(UpstreamPolicy {
        request_timeout_ms: 1_000,
        decision_max_attempts: 5,
        retry_budget_ms: 3_600,
    });
    let request = request_probe();

    // ── Act ────────────────────────────────────────────────────────────────────
    let started = Instant::now();
    let result = manager.decide_with_tools(&request, &[]);
    let elapsed = started.elapsed();
    let hits = mock.request_count();

    // ── Assert ─────────────────────────────────────────────────────────────────
    assert!(
        result.is_err(),
        "PA-174 F2 前置：恒不响应必须返回 Err（禁止伪造成功），实际 Ok"
    );
    // A1：聚合 deadline 必须在第 5 次 attempt 之前咬住（严格少于 attempts）。
    assert!(
        hits <= 4,
        "PA-174 F2：aggregate=8600ms 必须在 attempts=5 全部跑完前耗尽 ⇒ 真实请求数必须 ≤4，\
         实际 {hits} 次（=5 说明没有跨 attempt 总闸）"
    );
    // A2：墙钟上界（留抖动余量）。
    assert!(
        elapsed < Duration::from_millis(11_000),
        "PA-174 F2：跨 attempt 总 deadline=8600ms ⇒ 总墙钟必须 < 11000ms，实际 {elapsed:?}\
         （{hits} 次尝试）"
    );
}

// ── F3：生效策略必须可观测（disk vs effective + clamped_fields） ─────────────────────

/// 从 report 的策略对象里取三个字段（命名容忍 camelCase / snake_case，按语义子串归类）。
fn policy_triple(report: &Value, key: &str) -> (u64, u64, u64) {
    let policy = report
        .get(key)
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("PA-174 F3：report 必须含对象字段 `{key}`，实际 {report}"));
    let mut timeout = None;
    let mut attempts = None;
    let mut budget = None;
    for (field, value) in policy {
        let normalized = field.to_ascii_lowercase().replace('_', "");
        let number = value.as_u64().unwrap_or_else(|| {
            panic!("PA-174 F3：`{key}.{field}` 必须是 u64 整数，实际 {value}")
        });
        if normalized.contains("timeout") {
            timeout = Some(number);
        } else if normalized.contains("attempt") {
            attempts = Some(number);
        } else if normalized.contains("budget") {
            budget = Some(number);
        } else {
            panic!("PA-174 F3：`{key}` 出现契约外字段 `{field}`");
        }
    }
    (
        timeout.unwrap_or_else(|| panic!("PA-174 F3：`{key}` 缺少超时字段")),
        attempts.unwrap_or_else(|| panic!("PA-174 F3：`{key}` 缺少尝试次数字段")),
        budget.unwrap_or_else(|| panic!("PA-174 F3：`{key}` 缺少退避预算字段")),
    )
}

/// F3：`upstream_policy_report(raw, effective)` 必须同时暴露**磁盘原值**与**生效值**，
/// 并逐字段列出被钳制项（合法配置下为空列表）。
#[test]
fn f3_upstream_policy_report_exposes_disk_and_effective() {
    // ── A1：越界磁盘值 ⇒ 三个字段全部被钳制。 ─────────────────────────────────────
    let raw_invalid = UpstreamPolicy {
        request_timeout_ms: 0,
        decision_max_attempts: 0,
        retry_budget_ms: 0,
    };
    let effective_invalid = raw_invalid.clone().clamped().0;
    let report = upstream_policy_report(&raw_invalid, &effective_invalid);

    assert_eq!(
        policy_triple(&report, "disk_policy"),
        (0, 0, 0),
        "PA-174 F3：`disk_policy` 必须是磁盘原值 (0,0,0)，实际 {report}"
    );
    assert_eq!(
        policy_triple(&report, "effective_policy"),
        (15_000, 2, 5_000),
        "PA-174 F3：`effective_policy` 必须是生效值 (15000,2,5000)，实际 {report}"
    );
    let clamped_fields = report
        .get("clamped_fields")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("PA-174 F3：report 必须含数组字段 `clamped_fields`，实际 {report}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("PA-174 F3：`clamped_fields` 项必须是字符串，实际 {value}"))
                .to_string()
        })
        .collect::<Vec<_>>();
    for expected in ["request_timeout_ms", "decision_max_attempts", "retry_budget_ms"] {
        assert!(
            clamped_fields.iter().any(|field| field == expected),
            "PA-174 F3：越界配置的 `clamped_fields` 必须含 `{expected}`，实际 {clamped_fields:?}"
        );
    }
    assert_eq!(
        clamped_fields.len(),
        3,
        "PA-174 F3：三个字段全部越界 ⇒ 必须恰有 3 项被钳制，实际 {clamped_fields:?}"
    );

    // ── A2：合法配置 ⇒ `clamped_fields` 为空（不得误报）。 ─────────────────────────
    let raw_legal = UpstreamPolicy::default();
    let effective_legal = raw_legal.clone().clamped().0;
    assert_eq!(
        effective_legal, raw_legal,
        "PA-174 F3 夹具自检：默认策略本身必须在边界内"
    );
    let legal_report = upstream_policy_report(&raw_legal, &effective_legal);
    assert_eq!(
        policy_triple(&legal_report, "disk_policy"),
        (45_000, 5, 30_000),
        "PA-174 F3：合法配置的 `disk_policy` 必须原样暴露，实际 {legal_report}"
    );
    assert_eq!(
        legal_report.get("clamped_fields").and_then(Value::as_array),
        Some(&Vec::new()),
        "PA-174 F3：合法配置的 `clamped_fields` 必须为空列表，实际 {legal_report}"
    );
}

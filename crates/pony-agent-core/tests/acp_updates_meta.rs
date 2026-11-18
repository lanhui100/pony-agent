//! ACP SessionUpdate 14 变体 + 未知兜底 + clientCapabilities._meta 契约（B166/B180 / task-20）。
//!
//! 真源：`.dev-team/contract-matrix-acp-updates-meta.md`（U1–U6）。
//! 官方 schema 样例参考 `/tmp/acp-probe/schema-v1.json`；该路径是**环境探针**、不保证存在于 CI，
//! 故本文件用与官方 v1 对齐的**内联样例**（判别键 + 典型字段），逐变体 typed round-trip。
//!
//! 纪律：无 `#[ignore]`、无空断言；只碰 ACP 测试与夹具。

#[path = "support/paths.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use pony_agent_core::acp::client::{AcpAgentClient, AcpClientCapabilities};
use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::{
    ClientCapabilities, CompactionStatus, ContentBlock, NoticeSeverity, SessionUpdate,
};
use pony_agent_core::acp::session::AcpAgentSession;
use pony_agent_core::acp::ACP_DEFAULT_TURN_BUDGET;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-upd-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    support::deverbatim(&path.canonicalize().expect("canonicalize 临时工作目录"))
}

fn fixture_script() -> PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script
}

fn spec_with(mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let mut args = vec![fixture_script().to_string_lossy().into_owned(), mode.to_string()];
    args.extend(extra.iter().map(|value| (*value).to_string()));
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
        inherit_proxy_env: true,
        mcp_servers: Some(Vec::new()),
    }
}

fn write_updates_file(cwd: &Path, updates: &[Value]) -> PathBuf {
    let path = cwd.join("updates.jsonl");
    let body: String = updates
        .iter()
        .map(|update| format!("{}\n", serde_json::to_string(update).expect("update 可序列化")))
        .collect();
    std::fs::write(&path, body).expect("写 updates 文件");
    path
}

fn spec_emit_updates(cwd: &Path, updates: &[Value]) -> AcpAgentSpec {
    let init_file = cwd.join("initialize.jsonl");
    let updates_file = write_updates_file(cwd, updates);
    spec_with(
        "emit_updates",
        &[
            init_file.to_string_lossy().as_ref(),
            updates_file.to_string_lossy().as_ref(),
        ],
    )
}

fn spec_capturing_initialize(cwd: &Path) -> AcpAgentSpec {
    let init_file = cwd.join("initialize.jsonl");
    let session_file = cwd.join("session-new.jsonl");
    spec_with(
        "init_capture",
        &[
            init_file.to_string_lossy().as_ref(),
            "1",
            session_file.to_string_lossy().as_ref(),
        ],
    )
}

#[derive(Default)]
struct CapturingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl TurnEventSink for CapturingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((name.to_string(), payload));
    }
}

impl CapturingSink {
    fn snapshot(&self) -> Vec<(String, TurnStreamEvent)> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn payload_summary(payload: &TurnStreamEvent) -> String {
    serde_json::to_string(payload).unwrap_or_else(|error| format!("<unserializable:{error}>"))
}

fn events_summary(events: &[(String, TurnStreamEvent)]) -> String {
    events
        .iter()
        .map(|(name, payload)| format!("{name}:{}", payload_summary(payload)))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn prompt_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

fn spawn_session(spec: &AcpAgentSpec, cwd: &Path) -> AcpAgentSession {
    match AcpAgentSession::spawn_with_mcp_and_handlers(spec, cwd, Vec::new(), None, None) {
        Ok(session) => session,
        Err(error) => panic!("spawn 必须成功（fake agent 可用）：{error:?}"),
    }
}

fn captured_capabilities(cwd: &Path) -> Value {
    let path = cwd.join("initialize.jsonl");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获 initialize（{}：{error}）", path.display()));
    let frame: Value = serde_json::from_str(raw.lines().next().expect("initialize 行缺失"))
        .expect("initialize 必须是合法 JSON");
    assert_eq!(frame["method"], json!("initialize"));
    frame["params"]["clientCapabilities"].clone()
}

// ───────────────────────── U1：14 变体 round-trip ─────────────────────────

/// U1：官方 v1 的 14 个 `sessionUpdate` 变体逐一 typed round-trip，判别键与主字段逐字段等价。
#[test]
fn u1_fourteen_variants_roundtrip() {
    let samples: Vec<(&str, Value)> = vec![
        ("user_message_chunk", json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"u"},"messageId":"m-1"})),
        ("agent_message_chunk", json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"a"},"messageId":"m-2"})),
        ("agent_thought_chunk", json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"think"}})),
        ("tool_call", json!({"sessionUpdate":"tool_call","toolCallId":"tc-1","title":"edit","kind":"edit","status":"pending"})),
        ("tool_call_update", json!({"sessionUpdate":"tool_call_update","toolCallId":"tc-1","status":"completed"})),
        ("plan", json!({"sessionUpdate":"plan","entries":[{"content":"step","priority":"high","status":"pending"}]})),
        ("available_commands_update", json!({"sessionUpdate":"available_commands_update","availableCommands":[{"name":"/init","description":"init"}]})),
        ("current_mode_update", json!({"sessionUpdate":"current_mode_update","currentModeId":"code"})),
        ("config_option_update", json!({
            "sessionUpdate":"config_option_update",
            "configOptions":[
                {"type":"select","id":"mode","name":"Mode","currentValue":"code","options":[{"value":"code","name":"Code"}]},
                {"type":"boolean","id":"fast","name":"Fast","currentValue":true}
            ]
        })),
        ("session_info_update", json!({"sessionUpdate":"session_info_update","title":"T","updatedAt":"2026-11-15T00:00:00Z"})),
        ("usage_update", json!({"sessionUpdate":"usage_update","used":1234,"size":200000,"cost":{"amount":0.42,"currency":"USD"}})),
        ("notice", json!({"sessionUpdate":"notice","severity":"warning","title":"heads up","description":"d"})),
        ("compaction_update", json!({"sessionUpdate":"compaction_update","compactionId":"cmp-1","status":"completed","summary":[{"type":"text","text":"sum"}]})),
        ("compaction_summary_chunk", json!({"sessionUpdate":"compaction_summary_chunk","compactionId":"cmp-1","content":{"type":"text","text":"chunk"}})),
    ];
    assert_eq!(samples.len(), 14, "官方 v1 SessionUpdate 必须恰好 14 个变体");

    for (expected, sample) in &samples {
        let parsed: SessionUpdate = serde_json::from_value(sample.clone())
            .unwrap_or_else(|error| panic!("U1 违约：{expected} 反序列化失败：{error}\n{sample}"));
        let back = serde_json::to_value(&parsed)
            .unwrap_or_else(|error| panic!("U1 违约：{expected} 再序列化失败：{error}"));
        assert_eq!(
            back["sessionUpdate"],
            json!(expected),
            "U1 违约：判别键必须原样保留（{expected}），实际 {back}"
        );
        assert_eq!(
            back, *sample,
            "U1 违约：{expected} round-trip 不等价（字段名/camelCase/skip 语义被破坏）"
        );
    }

    // 6 个新增变体的主字段（typed）——防「能解析但字段丢弃」的假绿。
    let parse = |sample: &Value| -> SessionUpdate {
        serde_json::from_value(sample.clone()).expect("样例必须可解析")
    };
    match parse(&samples[8].1) {
        SessionUpdate::ConfigOptionUpdate(update) => {
            assert_eq!(update.config_options.len(), 2, "U1：configOptions 数量");
            match &update.config_options[0] {
                pony_agent_core::acp::protocol::SessionConfigOption::Select { current_value, .. } => {
                    assert_eq!(current_value, "code", "U1：select currentValue")
                }
                other => panic!("U1 违约：第 1 项应为 select，实际 {other:?}"),
            }
            match &update.config_options[1] {
                pony_agent_core::acp::protocol::SessionConfigOption::Boolean { current_value, .. } => {
                    assert!(*current_value, "U1：boolean currentValue")
                }
                other => panic!("U1 违约：第 2 项应为 boolean，实际 {other:?}"),
            }
        }
        other => panic!("U1：config_option_update 变体错误：{other:?}"),
    }
    match parse(&samples[9].1) {
        SessionUpdate::SessionInfoUpdate(update) => {
            assert_eq!(update.title.as_deref(), Some("T"), "U1：title");
            assert_eq!(
                update.updated_at.as_deref(),
                Some("2026-11-15T00:00:00Z"),
                "U1：updatedAt"
            );
        }
        other => panic!("U1：session_info_update 变体错误：{other:?}"),
    }
    match parse(&samples[10].1) {
        SessionUpdate::UsageUpdate(update) => {
            assert_eq!((update.used, update.size), (1234, 200000), "U1：used/size");
            let cost = update.cost.expect("U1：usage_update 必须带 cost");
            assert!((cost.amount - 0.42).abs() < f64::EPSILON, "U1：cost.amount");
            assert_eq!(cost.currency, "USD", "U1：cost.currency");
        }
        other => panic!("U1：usage_update 变体错误：{other:?}"),
    }
    match parse(&samples[11].1) {
        SessionUpdate::Notice(notice) => {
            assert_eq!(notice.severity, NoticeSeverity::Warning, "U1：severity");
            assert_eq!(notice.title, "heads up", "U1：title");
            assert_eq!(notice.description.as_deref(), Some("d"), "U1：description");
        }
        other => panic!("U1：notice 变体错误：{other:?}"),
    }
    match parse(&samples[12].1) {
        SessionUpdate::CompactionUpdate(update) => {
            assert_eq!(update.compaction_id, "cmp-1", "U1：compactionId");
            assert_eq!(update.status, CompactionStatus::Completed, "U1：status");
            assert_eq!(update.summary.as_ref().map(Vec::len), Some(1), "U1：summary");
        }
        other => panic!("U1：compaction_update 变体错误：{other:?}"),
    }
    match parse(&samples[13].1) {
        SessionUpdate::CompactionSummaryChunk(update) => {
            assert_eq!(update.compaction_id, "cmp-1", "U1：compactionId");
            assert!(
                matches!(update.content, ContentBlock::Text { ref text } if text == "chunk"),
                "U1：content 必须是 text chunk，实际 {:?}",
                update.content
            );
        }
        other => panic!("U1：compaction_summary_chunk 变体错误：{other:?}"),
    }
}

// ───────────────────────── U2：未知变体兜底 ─────────────────────────

/// U2：未知/厂商私有 `sessionUpdate` 必须降级为 `Unknown`（不 Err、不 panic），
/// 且**后续正常帧仍被实时 emit**（prompt 不中断）。
#[test]
fn u2_unknown_variant_falls_back_and_prompt_continues() {
    let unknown = json!({"sessionUpdate":"brand_new_variant_v99","foo":1});
    let parsed: SessionUpdate = serde_json::from_value(unknown.clone())
        .expect("U2 违约：未知变体不得导致整条反序列化失败");
    assert_eq!(
        parsed,
        SessionUpdate::Unknown,
        "U2 违约：未知 sessionUpdate 必须落到 Unknown 兜底变体"
    );

    let cwd = temp_dir("unknown-variant");
    let updates = vec![
        unknown,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}),
    ];
    let spec = spec_emit_updates(&cwd, &updates);
    let mut session = spawn_session(&spec, &cwd);
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);
    let sink = CapturingSink::default();
    let result = session.prompt(prompt_blocks("unknown then normal"), &sink);
    assert_eq!(result.status, "ok", "U2：未知变体不得中断本轮归约，实际 {result:?}");

    let events = sink.snapshot();
    assert!(
        events
            .iter()
            .any(|(name, payload)| name == "turn:delta" && payload.text.as_deref() == Some("hello")),
        "U2 违约：未知帧之后的正常帧仍必须被 emit，实际 {}",
        events_summary(&events)
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── U3：_meta 透传 ─────────────────────────

/// U3：`connect_with_capabilities_and_meta(..., Some(meta))` 必须把 `_meta` 原样写进
/// `initialize.clientCapabilities._meta`；`None` 时该键**不出现**（不是空对象），
/// 且 `ClientCapabilities` 反序列化容忍任意 `_meta` 内容。
#[test]
fn u3_client_capabilities_meta_passthrough() {
    let meta = json!({"opencode/child-session-updates": true, "nested": {"list": [1, 2]}});

    // (a) Some(meta) ⇒ 原样出现（值逐字节保留）。
    {
        let cwd = temp_dir("meta-present");
        let spec = spec_capturing_initialize(&cwd);
        let mut client = match AcpAgentClient::connect_with_capabilities_and_meta(
            &spec,
            &cwd,
            AcpClientCapabilities::NONE,
            Some(meta.clone()),
        ) {
            Ok(client) => client,
            Err(error) => panic!("U3：带 _meta 的 connect 必须成功：{error:?}"),
        };
        let capabilities = captured_capabilities(&cwd);
        assert_eq!(
            capabilities["_meta"], meta,
            "U3 违约：clientCapabilities._meta 必须原样透传，实际 {capabilities}"
        );
        client.close();
        let _ = std::fs::remove_dir_all(&cwd);
    }

    // (b) None ⇒ 键不出现（不得落空对象）。
    {
        let cwd = temp_dir("meta-absent");
        let spec = spec_capturing_initialize(&cwd);
        let mut client = match AcpAgentClient::connect_with_capabilities_and_meta(
            &spec,
            &cwd,
            AcpClientCapabilities::NONE,
            None,
        ) {
            Ok(client) => client,
            Err(error) => panic!("U3：无 _meta 的 connect 必须成功：{error:?}"),
        };
        let capabilities = captured_capabilities(&cwd);
        assert!(
            capabilities.get("_meta").is_none(),
            "U3 违约：未设置 _meta 时不得出现该键（不是空对象），实际 {capabilities}"
        );
        // 既有入口（connect_with_capabilities）行为不变。
        client.close();
        let _ = std::fs::remove_dir_all(&cwd);
    }

    // (c) 反序列化容忍任意 _meta（宿主只透传不解读）。
    let caps: ClientCapabilities =
        serde_json::from_value(json!({"_meta": {"weird": {"deep": [1, 2, 3]}}})).expect("U3：_meta 必须可容忍");
    assert_eq!(
        caps.meta,
        Some(json!({"weird": {"deep": [1, 2, 3]}})),
        "U3 违约：任意 _meta 内容必须原样保留"
    );
}

// ───────────────────────── U4：usage_update 消费位置 ─────────────────────────

/// U4：`usage_update` 只走 `emit_update` 的结构化日志 arm（`:853`），**不**产生
/// `turn:delta` / `turn:tool`；`used`/`size`/`cost{amount,currency}` 必须可解析。
#[test]
fn u4_usage_update_is_parsed_and_not_mapped_to_turn_events() {
    let usage = json!({
        "sessionUpdate": "usage_update",
        "used": 1234,
        "size": 200000,
        "cost": {"amount": 0.42, "currency": "USD"}
    });
    let parsed: SessionUpdate = serde_json::from_value(usage.clone()).expect("U4：usage_update 必须可解析");
    match &parsed {
        SessionUpdate::UsageUpdate(update) => {
            let cost = update.cost.as_ref().expect("U4：cost 必须可拿到");
            assert!((cost.amount - 0.42).abs() < f64::EPSILON, "U4：cost.amount");
            assert_eq!(cost.currency, "USD", "U4：cost.currency");
            assert_eq!((update.used, update.size), (1234, 200000), "U4：used/size");
        }
        other => panic!("U4 违约：usage_update 变体错误：{other:?}"),
    }

    let cwd = temp_dir("usage-position");
    let updates = vec![
        usage,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}),
    ];
    let spec = spec_emit_updates(&cwd, &updates);
    let mut session = spawn_session(&spec, &cwd);
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);
    let sink = CapturingSink::default();
    let result = session.prompt(prompt_blocks("usage then delta"), &sink);
    assert_eq!(result.status, "ok", "U4：usage_update 不得中断本轮归约，实际 {result:?}");

    let events = sink.snapshot();
    // usage_update 不得进 UI 事件流（U4：映射位置 = 结构化日志 arm）。
    for (name, payload) in &events {
        assert_ne!(
            name, "turn:tool",
            "U4 违约：usage_update 不得产生 turn:tool，实际 {}",
            events_summary(&events)
        );
        if name == "turn:delta" {
            assert!(
                payload.text.as_deref() != Some("usage_update"),
                "U4 违约：usage 内容不得以 delta 形式进 UI，实际 {}",
                events_summary(&events)
            );
        }
    }
    // 后续正常帧仍实时可见（不被打断）。
    assert!(
        events
            .iter()
            .any(|(name, payload)| name == "turn:delta" && payload.text.as_deref() == Some("hello")),
        "U4 违约：usage_update 之后的正常帧仍必须 emit，实际 {}",
        events_summary(&events)
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── U6：既有 8 变体映射回归锚 ─────────────────────────

/// U6（回归锚）：B166 新增 6 变体后，既有 update 的 `turn:delta`/`turn:tool` 映射逐字段不变。
#[test]
fn u5_legacy_update_mapping_unchanged() {
    let cwd = temp_dir("legacy-mapping");
    let updates = vec![
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}),
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"reasoning"}}),
        json!({"sessionUpdate":"tool_call","toolCallId":"tc-1","title":"edit a.rs","kind":"edit","status":"pending"}),
    ];
    let spec = spec_emit_updates(&cwd, &updates);
    let mut session = spawn_session(&spec, &cwd);
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);
    let sink = CapturingSink::default();
    let result = session.prompt(prompt_blocks("legacy"), &sink);
    assert_eq!(result.status, "ok", "U6：本轮应正常结束，实际 {result:?}");

    let events = sink.snapshot();
    let deltas: Vec<(Option<String>, Option<String>)> = events
        .iter()
        .filter(|(name, _)| name == "turn:delta")
        .map(|(_, payload)| (payload.text.clone(), payload.reasoning_content.clone()))
        .collect();
    assert_eq!(
        deltas,
        vec![
            (Some("hello".to_string()), None),
            (None, Some("reasoning".to_string())),
        ],
        "U6 违约：既有 delta 映射（text / reasoning_content）不得被 B166 改变，实际 {}",
        events_summary(&events)
    );
    let tool_events: Vec<&(String, TurnStreamEvent)> = events
        .iter()
        .filter(|(name, _)| name == "turn:tool")
        .collect();
    assert_eq!(
        tool_events.len(),
        1,
        "U6 违约：tool_call 必须仍恰好 emit 一次 turn:tool，实际 {}",
        events_summary(&events)
    );
    assert!(
        tool_events[0]
            .1
            .tool_activities
            .as_ref()
            .is_some_and(|activities| activities.iter().any(|activity| activity.id.contains("tc-1")
                || activity.name.contains("tc-1")
                || activity.description.contains("a.rs"))),
        "U6 违约：tool 事件必须仍指向 tc-1，实际 {}",
        events_summary(&events)
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

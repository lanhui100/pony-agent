//! wave-59 / PA-164 无头 CLI 契约验收（L2-T 功能契约 + L2-AT 对抗）。
//!
//! 真值源：`.dev-team/contract-matrix-wave59-headless.md`（**v2**，Lead 冻结）。
//! 被测面：`crates/pony-agent-core/src/agent/headless.rs`（T1 空桩 → T3 实现）。
//!
//! ## 契约 v1 → v2 的测试重同步（Lead 裁定 D1 / D2，不是我放宽断言）
//!
//! - **D1（撤销）**：v1 的 `PermissionMode::Deny/AllowList/Yolo` + `decide(allow_tools, tool)`
//!   与 `HeadlessOptions::allow_tools` **全部删除**。事实依据：宿主不存在可注入的策略求值
//!   接缝（`governed_executor.rs:281-287` 兜底 `governed_default → Allow`；evaluator 由
//!   runtime 内部注册于 `governed_executor.rs:463-469`），上层自判放行**永不被 runtime 消费**
//!   ⇒ 测它等于测死代码。改为断言 `PermissionMode{ReadOnly,WorkspaceWrite,FullAccess}` →
//!   `tool_authorization_mode()` 的三档**透传**。
//! - **A2（撤销，v3）**：`HeadlessOptions::max_turns` **删除**。一次 CLI 调用只跑一轮 turn，
//!   该值恒等于 1（语义空洞）；真正的「工具跳数上限」在 `runtime/limits.rs:76` 是
//!   `pub(super)` 内部原子量，宿主无公开接线口 ⇒ 声明即静默谎报，已连同 `--allow-tools`
//!   一并登记 PA-167。本文件不构造、不断言该字段。
//! - **D2（新增）**：`turn:suspended` 必须作为**终态**被测。事实：审批/ask 触发时 turn 立即
//!   返回且只发 `turn:suspended`（`runtime/turn_stream.rs:376-438`），`bin/cli.rs` 的 sink 不认
//!   该事件 ⇒ 死循环挂死。这是在测本波最真实的缺陷，不是补覆盖率。
//!
//! ## 本文件的分工（重要，勿误读）
//!
//! Rust 侧做**黑盒类型 / 纯函数 / 注入式接缝**断言（C1 sink 纯净性与全事件透传、C2 退出码
//! 与状态映射、C3 封套形状、C4 审批模式透传、C5 注入 runner 的有界执行）。
//!
//! **封套 `status ⟺ exit_code` 的自洽性不在 Rust 侧对「测试自造的对象」重复断言**——那会是
//! 同义反复（fake test）。真实产物上的交叉比对落在
//! `tests/acceptance/wave59/headless-cli-e2e.sh` 的 E4（`exit_code` vs 真实进程退出码）。
//! Rust 侧只做**产品函数驱动**的组合断言：`envelope(status, exit_code_for(status).code())`
//! 的 `exit_code` 必须等于契约值——该断言随 `exit_code_for` 的实现退化而变红，非恒真。
//!
//! ## 事实依据（Lead E1/E2 侦察，非本测试臆造）
//! - `ControlPlane::start_turn_stream` 是**同步阻塞**的（`control_plane/mod.rs:1931`），故有界执行
//!   只能由 `run_with_deadline` 在独立 worker 线程跑 runner、调用线程带超时 join；
//! - `TurnStreamEvent.error` 是自由文本 `Option<String>`（`runtime/types.rs:166`），`ErrorCode`
//!   未上 wire ⇒ `classify_failure` 只能按文本分档，**未知文本必须 fail-closed**；
//! - 真实事件名形如 `turn:started` / `turn:delta` / `turn:tool` / `turn:completed`
//!   （`runtime/turn_stream.rs:118,582,1237`）——sink 契约是**全事件透传，不得按名过滤**。
//!
//! ## 零假测试自律
//! - 每条断言都随产品能力退化而变红（`classify_failure` 兜底 Success → C2 立刻红；
//!   `tool_authorization_mode` 串档 → C4 立刻红；sink 按名过滤 → 全事件透传立刻红；
//!   suspended 不认 → D2 用例立刻红）。
//! - C5 用注入 runner：①立即返回 + timeout 1s ⇒ Completed；②runner sleep 300ms +
//!   timeout 10ms ⇒ Timeout。断言的是**结局**，不是「猜时序」。
//! - 无 `assert!(true)` 类恒真；每例至少一条「状态突变或关键返回值」断言。
//!
//! 纪律：本文件只读 `pony_agent_core::agent::headless::*` 的公开面，不触碰任何 `src/` 实现细节。

use pony_agent_core::agent::headless::{
    exit_code_for, run_with_deadline, ExitCode, HeadlessOptions, HeadlessOutcome, JsonlSink,
    OutputFormat, PermissionMode, ResultEnvelope, SseSink, ToolCallSummary, TurnStatus,
    UsageSummary, RESULT_ENVELOPE_SCHEMA,
};
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::Value;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// 构造一个最小合法 `TurnStreamEvent`。
///
/// `kind` 与 `emit` 的 `name` 保持一致：真实 runtime 的 `emit("turn:xxx", payload)`
/// 中 payload.kind 同样是 `turn:xxx`（`runtime/turn_stream.rs`），故 sink 无论按
/// name 还是按 payload.kind 归纳终态都应得到同一结论。
fn stream_event(kind: &str, text: &str) -> TurnStreamEvent {
    serde_json::from_value(serde_json::json!({
        "turnId": "turn-headless-contract",
        "kind": kind,
        "text": text,
        "sessionId": "session-headless-contract",
    }))
    .expect("TurnStreamEvent fixture must deserialize")
}

/// 把 sink 的 `Vec<String>` 摊平成非空行（容忍行内自带换行/末尾换行）。
fn non_empty_lines(chunks: &[String]) -> Vec<String> {
    chunks
        .iter()
        .flat_map(|chunk| chunk.split('\n'))
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect()
}

fn envelope(status: TurnStatus, exit_code: i32) -> ResultEnvelope {
    ResultEnvelope {
        schema: RESULT_ENVELOPE_SCHEMA.to_string(),
        session_id: "session-headless-contract".to_string(),
        turn_id: "turn-headless-contract".to_string(),
        status,
        exit_code,
        final_text: "done".to_string(),
        usage: UsageSummary {
            input_tokens: 11,
            output_tokens: 22,
        },
        tool_calls: vec![ToolCallSummary {
            name: "Read".to_string(),
            status: "ok".to_string(),
        }],
        duration_ms: 1234,
    }
}

// ---------------------------------------------------------------------------
// C2 退出码：8 个值 + fail-closed 分类 + 终态映射表
// ---------------------------------------------------------------------------

#[test]
fn c2_exit_code_values_are_frozen_and_unique() {
    // Arrange: 契约冻结的 (变体, 数值) 全表
    let table = [
        (ExitCode::Success, 0),
        (ExitCode::Internal, 1),
        (ExitCode::Usage, 2),
        (ExitCode::Config, 3),
        (ExitCode::ProviderNetwork, 4),
        (ExitCode::ToolFailure, 5),
        (ExitCode::PermissionDenied, 6),
        (ExitCode::Timeout, 7),
    ];

    // Act + Assert
    for (variant, expected) in table {
        assert_eq!(
            variant.code(),
            expected,
            "ExitCode::{variant:?}.code() 必须为 {expected}（契约 C2 冻结）"
        );
    }

    // 唯一性：8 个变体不得共用数值（否则 CLI 退出码语义塌陷）
    let mut codes: Vec<i32> = table.iter().map(|(v, _)| v.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(
        codes.len(),
        8,
        "8 个 ExitCode 的数值必须两两不同，实际 {codes:?}"
    );
}

#[test]
fn c2_classify_unknown_error_fails_closed_to_internal_not_success() {
    // Arrange: 一段不含任何已知分类关键词的失败原因（payload.error 是自由文本）
    let unknown = "subsystem blew up: quantum flux reversed at stage zeta-9";

    // Act
    let classified = ExitCode::classify_failure(unknown);

    // Assert: fail-closed —— 未知必须落 Internal(1)，严禁兜底 Success(0)
    assert_ne!(
        classified,
        ExitCode::Success,
        "未知失败原因被兜底为 Success —— fail-closed 契约被击穿"
    );
    assert_eq!(
        classified,
        ExitCode::Internal,
        "未知失败原因必须归类为 Internal(1)，实际 {classified:?}"
    );
    assert_eq!(classified.code(), 1, "未知失败的进程退出码必须为 1");
}

/// Lead 硬要求：一条直接锁死「turn 失败不得被映射为 Success」的断言。
#[test]
fn c2_turn_failed_must_never_map_to_success() {
    // Arrange: runtime 失败事件的真实名字（runtime/tests.rs:2002 断言该名字存在）
    let turn_failed = "turn:failed";

    // Act
    let classified = ExitCode::classify_failure(turn_failed);

    // Assert
    assert_ne!(
        classified, ExitCode::Success,
        "`turn:failed` 被映射成 Success —— 无头流水线会以 0 退出，CI 将把失败当成功"
    );
    assert_ne!(
        classified.code(),
        0,
        "`turn:failed` 的退出码为 0 —— 违反 C2「turn:failed 绝不退 0」"
    );
}

#[test]
fn c2_classify_is_fail_closed_across_adversarial_inputs() {
    // Arrange: 空串 / 纯空白 / 看似成功的词 / 畸形 payload / ANSI / 超长串 / 非 ASCII
    let adversarial = [
        "",
        "   ",
        "ok",
        "success",
        "completed successfully",
        "{\"status\":\"completed\"}",
        "unimplemented",
        "\u{1b}[31mboom\u{1b}[0m",
        "失败原因包含中文与 emoji 🚨",
        &"x".repeat(4096),
    ];

    // Act + Assert
    for input in adversarial {
        let classified = ExitCode::classify_failure(input);
        let preview: String = input.chars().take(40).collect();
        assert_ne!(
            classified, ExitCode::Success,
            "对抗输入 {preview:?} 被分类为 Success —— classify_failure 必须 fail-closed"
        );
        assert_ne!(
            classified.code(),
            0,
            "对抗输入 {preview:?} 得到退出码 0 —— 违反 fail-closed"
        );
    }
}

/// D2 核心：终态 → 退出码的冻结映射表（含 suspended ⟹ 6，本波最真实缺陷）。
#[test]
fn c2_terminal_status_maps_to_frozen_exit_codes() {
    // Arrange + Act
    let completed = exit_code_for(TurnStatus::Completed);
    let failed = exit_code_for(TurnStatus::Failed);
    let suspended = exit_code_for(TurnStatus::Suspended);
    let timeout = exit_code_for(TurnStatus::Timeout);
    let cancelled = exit_code_for(TurnStatus::Cancelled);

    // Assert 1: 正常完成 = 0
    assert_eq!(
        completed,
        ExitCode::Success,
        "Completed 必须映射 Success(0)，实际 {completed:?}"
    );

    // Assert 2: 失败绝不退 0（具体档位由 classify_failure 决定，见 C2 分档用例）
    assert_ne!(
        failed,
        ExitCode::Success,
        "Failed 被映射为 Success —— turn 失败退 0，CI 将把失败当成功"
    );
    assert_ne!(failed.code(), 0, "Failed 的退出码必须非 0，实际 {failed:?}");

    // Assert 3: suspended 是终态且 fail-closed 到 PermissionDenied(6)
    assert_eq!(
        suspended,
        ExitCode::PermissionDenied,
        "Suspended 必须映射 PermissionDenied，实际 {suspended:?}（无头无审批应答方，挂起即拒绝）"
    );
    assert_eq!(
        suspended.code(),
        6,
        "Suspended 的退出码必须为 6，实际 {}",
        suspended.code()
    );

    // Assert 4: 超时 = 7
    assert_eq!(
        timeout,
        ExitCode::Timeout,
        "Timeout 必须映射 ExitCode::Timeout，实际 {timeout:?}"
    );
    assert_eq!(timeout.code(), 7, "Timeout 的退出码必须为 7");

    // Assert 5: 无头下无交互取消源 ⇒ 保守归 Internal(1)
    assert_eq!(
        cancelled,
        ExitCode::Internal,
        "Cancelled 必须映射 Internal(1)（契约 v2 映射表），实际 {cancelled:?}"
    );
    assert_ne!(
        cancelled.code(),
        0,
        "Cancelled 的退出码必须非 0，实际 {}",
        cancelled.code()
    );
}

// ---------------------------------------------------------------------------
// C1 结构化输出：OutputFormat 解析
// ---------------------------------------------------------------------------

#[test]
fn c1_output_format_parses_frozen_cli_tokens() {
    // Arrange + Act
    let text = OutputFormat::from_str("text");
    let jsonl = OutputFormat::from_str("jsonl");
    let sse = OutputFormat::from_str("sse");

    // Assert: 契约 C1 冻结的三个 `--output-format` 取值必须可解析
    assert_eq!(text.expect("`text` 必须可解析"), OutputFormat::Text);
    assert_eq!(jsonl.expect("`jsonl` 必须可解析"), OutputFormat::Jsonl);
    assert_eq!(sse.expect("`sse` 必须可解析"), OutputFormat::Sse);
}

#[test]
fn c1_output_format_rejects_unknown_token() {
    // Arrange + Act
    let parsed = OutputFormat::from_str("xml");

    // Assert: 非法值必须 Err（供 CLI 映射为 Usage=2），不得静默退化为 Text
    assert!(
        parsed.is_err(),
        "--output-format xml 必须解析失败（CLI 才能 exit 2），实际 Ok({:?})",
        parsed.ok()
    );
}

// ---------------------------------------------------------------------------
// C1 JsonlSink：逐行 JSON / 全事件透传 / 无 ANSI / 终态归纳（含 suspended）
// ---------------------------------------------------------------------------

#[test]
fn c1_jsonl_sink_emits_one_valid_json_line_per_event() {
    // Arrange: sink 会被 worker 线程调用 ⇒ 必须可跨线程共享（编译期断言）
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<JsonlSink>();
    assert_send_sync::<SseSink>();

    let sink = JsonlSink::new();

    // Act: 两个事件 → 必须两行
    sink.emit("turn:started", stream_event("turn:started", "hello"));
    sink.emit("turn:delta", stream_event("turn:delta", "world"));
    let lines = non_empty_lines(&sink.finish());

    // Assert: 事件数量守恒
    assert_eq!(
        lines.len(),
        2,
        "2 个事件必须产出 2 行 JSONL，实际 {} 行：{lines:?}",
        lines.len()
    );

    // Assert: 每行合法 JSON 对象 + 含 seq/type 字段
    for (idx, line) in lines.iter().enumerate() {
        let parsed: Value = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("jsonl 第 {idx} 行不是合法 JSON（{err}）：{line:?}"));
        let obj = parsed
            .as_object()
            .unwrap_or_else(|| panic!("jsonl 第 {idx} 行必须是 JSON 对象：{line:?}"));
        assert!(
            obj.get("seq").map(Value::is_number).unwrap_or(false),
            "jsonl 第 {idx} 行缺少数值型 `seq` 字段：{line:?}"
        );
        assert!(
            obj.get("type").map(Value::is_string).unwrap_or(false),
            "jsonl 第 {idx} 行缺失字符串型 `type` 字段：{line:?}"
        );
    }
}

#[test]
fn c1_jsonl_sink_passes_through_every_event_kind() {
    // Arrange: 真实 runtime 会发出的事件全集（Lead E1 侦察事实）
    let event_kinds = [
        "turn:started",
        "turn:delta",
        "turn:tool",
        "turn:trace",
        "turn:hop_complete",
        "turn:output_end",
        "turn:phase_changed",
        "turn:checkpoint_persisted",
        "turn:completed",
        "turn:failed",
        "turn:cancelled",
        "turn:suspended",
    ];
    let sink = JsonlSink::new();

    // Act
    for kind in event_kinds {
        sink.emit(kind, stream_event(kind, "x"));
    }
    let lines = non_empty_lines(&sink.finish());

    // Assert 1: 全事件透传——不得按事件名白名单过滤
    assert_eq!(
        lines.len(),
        event_kinds.len(),
        "sink 丢弃了事件（{} 个事件只出 {} 行）—— 契约要求全事件透传",
        event_kinds.len(),
        lines.len()
    );

    // Assert 2: 每行的 type 必须回指其事件名（避免 sink 把所有事件压成同一 type）
    let emitted: Vec<String> = lines
        .iter()
        .map(|line| {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("第 {line:?} 行不是合法 JSON：{err}"));
            value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("第 {line:?} 行缺少字符串型 `type`"))
                .to_string()
        })
        .collect();
    for kind in event_kinds {
        assert!(
            emitted.iter().any(|ty| ty == kind),
            "事件 {kind} 未以其名出现在 jsonl 的 `type` 字段中（实际 {emitted:?}）"
        );
    }
}

#[test]
fn c1_jsonl_sink_output_contains_no_ansi_escapes() {
    // Arrange: 事件文本本身夹带 ANSI（真实场景：模型文本/工具输出带颜色码）
    let sink = JsonlSink::new();

    // Act
    sink.emit(
        "turn:delta",
        stream_event("turn:delta", "\u{1b}[1;31mred\u{1b}[0m plain"),
    );
    let lines = non_empty_lines(&sink.finish());

    // Assert: 无 ESC 字节 —— 管道/CI 消费者不得收到终端控制序列
    assert!(!lines.is_empty(), "sink 必须至少产出一行（否则断言空转）");
    for line in &lines {
        assert!(
            !line.contains('\u{1b}'),
            "jsonl 输出含 ANSI 转义（\\x1b）：{line:?}"
        );
        assert!(!line.contains("\u{1b}["), "jsonl 输出含 CSI 序列：{line:?}");
    }
}

#[test]
fn c1_sink_outcome_tracks_terminal_events_only() {
    // Arrange: 非终态事件不得把 sink 判成「已收工」
    let sink = JsonlSink::new();

    // Act + Assert 1: 全新 sink 无终态
    assert_eq!(
        sink.outcome(),
        None,
        "未收到任何事件前 outcome() 必须为 None（否则 CLI 会提前收工）"
    );

    // Act + Assert 2: 过程事件不得被误判为终态
    sink.emit("turn:delta", stream_event("turn:delta", "partial"));
    assert_eq!(
        sink.outcome(),
        None,
        "turn:delta 不得被归纳为终态（fail-closed：宁可继续等，不可提前宣告完成）"
    );

    // Act + Assert 3: 终态事件必须被正确归纳
    let failed_sink = JsonlSink::new();
    failed_sink.emit("turn:failed", stream_event("turn:failed", "boom"));
    assert_eq!(
        failed_sink.outcome(),
        Some(HeadlessOutcome::Failed),
        "turn:failed 必须归纳为 HeadlessOutcome::Failed"
    );

    let completed_sink = JsonlSink::new();
    completed_sink.emit("turn:completed", stream_event("turn:completed", "done"));
    assert_eq!(
        completed_sink.outcome(),
        Some(HeadlessOutcome::Completed),
        "turn:completed 必须归纳为 HeadlessOutcome::Completed"
    );

    let cancelled_sink = JsonlSink::new();
    cancelled_sink.emit("turn:cancelled", stream_event("turn:cancelled", "stop"));
    assert_eq!(
        cancelled_sink.outcome(),
        Some(HeadlessOutcome::Cancelled),
        "turn:cancelled 必须归纳为 HeadlessOutcome::Cancelled"
    );
}

/// D2 核心用例：`turn:suspended` 是**终态**，sink 必须认它。
///
/// 缺陷背景：`bin/cli.rs` 的 sink 只认 `turn:completed/failed/cancelled`，
/// 审批/ask 触发时 runtime 只发 `turn:suspended` ⇒ CLI 死循环挂死（永不退出）。
#[test]
fn c1_jsonl_sink_treats_suspended_as_terminal() {
    // Arrange
    let sink = JsonlSink::new();

    // Act
    sink.emit("turn:suspended", stream_event("turn:suspended", "awaiting approval"));

    // Assert 1: 必须收敛为 Suspended 终态（否则调用方永远等不到收工信号）
    assert_eq!(
        sink.outcome(),
        Some(HeadlessOutcome::Suspended),
        "turn:suspended 未被识别为终态 —— 无头 CLI 将在审批挂起时死循环（D2 缺陷复现）"
    );

    // Assert 2: 该事件仍必须出现在事件流里（终态归纳不得吞掉事件本身）
    let lines = non_empty_lines(&sink.finish());
    assert_eq!(
        lines.len(),
        1,
        "turn:suspended 必须既被归纳为终态、又原样透传为一行事件，实际 {} 行",
        lines.len()
    );
}

#[test]
fn c1_sse_sink_frames_are_wellformed_and_ansi_free() {
    // Arrange
    let sink = SseSink::new();

    // Act
    sink.emit("turn:started", stream_event("turn:started", "hello"));
    sink.emit("turn:delta", stream_event("turn:delta", "wor\u{1b}[0mld"));
    let frames = sink.finish();
    let joined = frames.join("\n");

    // Assert 1: SSE 必须有 data: 行（否则不是 SSE 帧）
    let data_payloads: Vec<&str> = joined
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|payload| !payload.is_empty() && *payload != "[DONE]")
        .collect();
    assert!(
        !data_payloads.is_empty(),
        "SseSink 未产出任何 `data:` 帧：{joined:?}"
    );

    // Assert 2: 每个 data 载荷必须是合法 JSON 对象
    for payload in &data_payloads {
        let parsed: Value = serde_json::from_str(payload)
            .unwrap_or_else(|err| panic!("SSE data 载荷不是合法 JSON（{err}）：{payload:?}"));
        assert!(
            parsed.is_object(),
            "SSE data 载荷必须是 JSON 对象：{payload:?}"
        );
    }

    // Assert 3: 整帧流无 ANSI 转义
    assert!(
        !joined.contains('\u{1b}'),
        "SSE 输出含 ANSI 转义（\\x1b）：{joined:?}"
    );
}

// ---------------------------------------------------------------------------
// C3 结果封套形状（真实产物上的 exit_code 交叉比对在 E2E E4）
// ---------------------------------------------------------------------------

#[test]
fn c3_result_envelope_schema_literal_is_frozen() {
    // Arrange + Act
    let serialized = serde_json::to_value(envelope(TurnStatus::Completed, 0))
        .expect("ResultEnvelope 必须可序列化");

    // Assert: schema 字面量（跨波次冻结，consumer 依赖它做版本路由）
    assert_eq!(
        RESULT_ENVELOPE_SCHEMA, "pony.headless.result.v1",
        "RESULT_ENVELOPE_SCHEMA 常量被改动"
    );
    assert_eq!(
        serialized.get("schema").and_then(Value::as_str),
        Some("pony.headless.result.v1"),
        "序列化后的 `schema` 字段必须等于冻结字面量，实际 {serialized}"
    );
}

#[test]
fn c3_result_envelope_field_set_and_types() {
    // Arrange
    let serialized = serde_json::to_value(envelope(TurnStatus::Completed, 0))
        .expect("ResultEnvelope 必须可序列化");
    let obj = serialized
        .as_object()
        .expect("ResultEnvelope 序列化结果必须是 JSON 对象");

    // Act: 契约 C3 冻结的字段集
    let contract_fields = [
        "schema",
        "session_id",
        "turn_id",
        "status",
        "exit_code",
        "final_text",
        "usage",
        "tool_calls",
        "duration_ms",
    ];

    // Assert 1: 字段存在
    for field in contract_fields {
        assert!(
            obj.contains_key(field),
            "ResultEnvelope 缺少契约字段 `{field}`，实际字段集 {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    // Assert 2: 类型形状（防止 usage 塞成数字、tool_calls 塞成对象等漂移）
    assert!(obj["exit_code"].is_number(), "exit_code 必须是数字");
    assert!(obj["duration_ms"].is_number(), "duration_ms 必须是数字");
    assert!(obj["final_text"].is_string(), "final_text 必须是字符串");
    assert!(obj["session_id"].is_string(), "session_id 必须是字符串");
    assert!(obj["turn_id"].is_string(), "turn_id 必须是字符串");
    assert!(obj["status"].is_string(), "status 必须序列化为字符串令牌");

    let usage = obj["usage"]
        .as_object()
        .expect("usage 必须是对象（input_tokens/output_tokens）");
    assert!(
        usage
            .get("input_tokens")
            .map(Value::is_number)
            .unwrap_or(false),
        "usage.input_tokens 必须是数字，实际 {usage:?}"
    );
    assert!(
        usage
            .get("output_tokens")
            .map(Value::is_number)
            .unwrap_or(false),
        "usage.output_tokens 必须是数字，实际 {usage:?}"
    );

    let tool_calls = obj["tool_calls"]
        .as_array()
        .expect("tool_calls 必须是数组");
    assert!(
        !tool_calls.is_empty(),
        "tool_calls 夹具非空，序列化后不得丢项"
    );
    let first = tool_calls[0]
        .as_object()
        .expect("tool_calls 元素必须是对象 {name,status}");
    assert_eq!(
        first.get("name").and_then(Value::as_str),
        Some("Read"),
        "tool_calls[0].name 丢失"
    );
    assert_eq!(
        first.get("status").and_then(Value::as_str),
        Some("ok"),
        "tool_calls[0].status 丢失"
    );
}

#[test]
fn c3_turn_status_tokens_are_the_frozen_five() {
    // Arrange + Act（v2 起含 Suspended）
    let tokens: Vec<String> = [
        TurnStatus::Completed,
        TurnStatus::Failed,
        TurnStatus::Timeout,
        TurnStatus::Cancelled,
        TurnStatus::Suspended,
    ]
    .iter()
    .map(|status| {
        serde_json::to_value(status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| panic!("TurnStatus::{status:?} 必须序列化为字符串令牌"))
    })
    .collect();

    // Assert 1: 小写归一后必须精确等于契约五态（防 casing 漂移把 consumer 的等值判断打空）
    let normalized: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
    assert_eq!(
        normalized,
        vec!["completed", "failed", "timeout", "cancelled", "suspended"],
        "TurnStatus 序列化令牌必须为契约五态（大小写归一），实际 {tokens:?}"
    );

    // Assert 2: 令牌两两不同（否则 consumer 无法区分终态）
    let mut unique = tokens.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        5,
        "5 个终态令牌必须两两不同，实际 {tokens:?}"
    );
}

/// D2：信封 `status=Suspended ⟹ exit_code==6`。
///
/// 断言由**产品函数** `exit_code_for` 驱动（不是测试自造的常量），故它随映射退化而变红。
#[test]
fn c3_suspended_envelope_implies_permission_denied_exit_code() {
    // Arrange
    let status = TurnStatus::Suspended;
    let product_code = exit_code_for(status).code();

    // Act
    let serialized = serde_json::to_value(envelope(status, product_code))
        .expect("ResultEnvelope 必须可序列化");
    let obj = serialized.as_object().expect("封套必须是 JSON 对象");

    // Assert 1: 封套里的 exit_code 必须是 6（PermissionDenied）
    assert_eq!(
        obj.get("exit_code").and_then(Value::as_i64),
        Some(6),
        "status=Suspended 时封套 exit_code 必须为 6（PermissionDenied），实际 {serialized}"
    );

    // Assert 2: status 令牌归一为 suspended（consumer 用它对账）
    assert_eq!(
        obj.get("status")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
            .as_deref(),
        Some("suspended"),
        "封套 status 令牌必须为 suspended，实际 {serialized}"
    );

    // Assert 3: 挂起绝不退 0
    assert_ne!(
        obj.get("exit_code").and_then(Value::as_i64),
        Some(0),
        "status=Suspended 却以 exit_code=0 落盘 —— 无头调用方会把挂起当成功"
    );
}

// ---------------------------------------------------------------------------
// C4 审批模式（v2：只透传宿主 tool_authorization_mode，不逐工具裁决）
// ---------------------------------------------------------------------------

#[test]
fn c4_permission_mode_maps_to_host_tool_authorization_mode() {
    // Arrange: 宿主真值源取值（governed_executor.rs:310-333）
    let table = [
        (PermissionMode::ReadOnly, "read_only"),
        (PermissionMode::WorkspaceWrite, "workspace_write"),
        (PermissionMode::FullAccess, "full_access"),
    ];

    // Act + Assert: 三档必须逐字透传（错档 = 宿主读到别的授权级别）
    for (mode, expected) in table {
        assert_eq!(
            mode.tool_authorization_mode(),
            expected,
            "PermissionMode::{mode:?} 必须透传 {expected:?}，实际 {:?}",
            mode.tool_authorization_mode()
        );
    }

    // 唯一性：三档不得串成同一字符串（否则档位形同虚设）
    let mut modes: Vec<&str> = table
        .iter()
        .map(|(mode, _)| mode.tool_authorization_mode())
        .collect();
    modes.sort_unstable();
    modes.dedup();
    assert_eq!(modes, vec!["full_access", "read_only", "workspace_write"]);
}

#[test]
fn c4_permission_mode_parses_frozen_aliases_and_rejects_others() {
    // Arrange + Act: 冻结别名表（v2）
    let read_only_aliases = ["deny", "read-only", "read_only"];
    let workspace_write_aliases = ["workspace-write", "workspace_write"];
    let full_access_aliases = ["full-access", "full_access"];

    // Assert 1: 别名归一
    for alias in read_only_aliases {
        assert_eq!(
            PermissionMode::from_str(alias).ok(),
            Some(PermissionMode::ReadOnly),
            "--permission-mode {alias:?} 必须解析为 ReadOnly"
        );
    }
    for alias in workspace_write_aliases {
        assert_eq!(
            PermissionMode::from_str(alias).ok(),
            Some(PermissionMode::WorkspaceWrite),
            "--permission-mode {alias:?} 必须解析为 WorkspaceWrite"
        );
    }
    for alias in full_access_aliases {
        assert_eq!(
            PermissionMode::from_str(alias).ok(),
            Some(PermissionMode::FullAccess),
            "--permission-mode {alias:?} 必须解析为 FullAccess"
        );
    }

    // Assert 2: 已撤销档位与垃圾输入必须 Err（CLI 才能 exit 2）
    for bogus in ["allow-list", "allowlist", "yolo", "accept-all", "", "  ", "READ_ONLY"] {
        assert!(
            PermissionMode::from_str(bogus).is_err(),
            "非法 --permission-mode {bogus:?} 必须 Err（CLI 才能 exit 2），实际 Ok({:?})",
            PermissionMode::from_str(bogus).ok()
        );
    }
}

#[test]
fn c4_headless_options_shape_is_frozen() {
    // Arrange + Act: 结构契约（字段名/类型漂移会在编译期失败）
    // v2 撤销 allow_tools / v3 撤销 max_turns —— 二者均无宿主消费口，故不再出现在结构里。
    let options: HeadlessOptions = HeadlessOptions {
        output_format: OutputFormat::Jsonl,
        result_json_path: Some(PathBuf::from("/tmp/pony-headless-result.json")),
        permission_mode: PermissionMode::ReadOnly,
        timeout_secs: Some(30),
    };

    // Assert
    assert_eq!(options.output_format, OutputFormat::Jsonl);
    assert_eq!(options.permission_mode, PermissionMode::ReadOnly);
    assert_eq!(options.timeout_secs, Some(30));
    assert_eq!(
        options.permission_mode.tool_authorization_mode(),
        "read_only",
        "显式 read-only 必须透传宿主 read_only 档"
    );
}

/// D1 硬要求：**默认审批策略必须 fail-closed 到只读**。
///
/// 缺陷背景：今天 CLI 未传 `--permission-mode` 时传 `None`，宿主对 `None` 与
/// `workspace_write`/`full_access` 判定等价（`governed_executor.rs:310-333` 无独立分支）
/// ⇒ 默认即**完全不受限**。本断言锁死「默认 = read_only」这一修复。
#[test]
fn c4_headless_options_default_permission_mode_is_read_only() {
    // Arrange + Act
    let defaults = HeadlessOptions::default();

    // Assert 1: 默认审批档位必须是 ReadOnly（fail-closed）
    assert_eq!(
        defaults.permission_mode,
        PermissionMode::ReadOnly,
        "HeadlessOptions 默认 permission_mode 必须是 ReadOnly（fail-closed），实际 {:?}",
        defaults.permission_mode
    );

    // Assert 2: 透传到宿主的字符串也必须是 read_only（真正被消费的是这个值）
    assert_eq!(
        defaults.permission_mode.tool_authorization_mode(),
        "read_only",
        "默认档位透传宿主的 tool_authorization_mode 必须是 read_only"
    );

    // Assert 3: 其余默认值（Text / 不落文件 / 无超时）
    assert_eq!(
        defaults.output_format,
        OutputFormat::Text,
        "默认输出形态必须是 text（与基线逐字节兼容）"
    );
    assert!(
        defaults.result_json_path.is_none(),
        "默认不得落结果文件（显式 --result-json 才写）"
    );
    assert!(
        defaults.timeout_secs.is_none(),
        "默认无超时预算（显式 --timeout-secs 才有界）"
    );
}

// ---------------------------------------------------------------------------
// C5 有界执行：注入 runner（确定性，不靠猜时序）
// ---------------------------------------------------------------------------

#[test]
fn c5_fast_runner_within_budget_completes() {
    // Arrange: 立刻返回的 runner + 1s 预算
    let started = Instant::now();

    // Act
    let outcome = run_with_deadline(Some(Duration::from_secs(1)), || {});

    // Assert 1: 结局必须是 Completed（超时不得误伤快 runner）
    assert_eq!(
        outcome,
        HeadlessOutcome::Completed,
        "1s 预算内的立即返回 runner 必须得到 Completed，实际 {outcome:?}"
    );
    // Assert 2: 不得把整个预算耗完（说明有界等待而非固定 sleep）
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "run_with_deadline 在 runner 秒回后仍耗时 {:?} —— 未做提前收敛",
        started.elapsed()
    );
}

#[test]
fn c5_no_budget_means_unbounded_and_still_completes() {
    // Arrange + Act: timeout=None ⇒ 无界，runner 秒回仍应 Completed
    let outcome = run_with_deadline(None, || {});

    // Assert
    assert_eq!(
        outcome,
        HeadlessOutcome::Completed,
        "timeout=None 时 runner 正常结束必须得到 Completed，实际 {outcome:?}"
    );
}

#[test]
fn c5_expired_budget_returns_timeout_without_hanging() {
    // Arrange: runner 明显超出 10ms 预算（300ms），断言的是**结局**而非时序猜测
    let started = Instant::now();

    // Act
    let outcome = run_with_deadline(Some(Duration::from_millis(10)), || {
        std::thread::sleep(Duration::from_millis(300));
    });
    let elapsed = started.elapsed();

    // Assert 1: 结局必须是 Timeout（C5：到期 → Timeout，CLI 再映射 exit 7）
    assert_eq!(
        outcome,
        HeadlessOutcome::Timeout,
        "超出预算的 runner 必须得到 HeadlessOutcome::Timeout，实际 {outcome:?}"
    );
    // Assert 2: 调用线程不得被 worker 拖满（有界等待：远小于 300ms 的 runner 时长）
    assert!(
        elapsed < Duration::from_millis(300),
        "run_with_deadline 等待了 {elapsed:?}（worker 需 300ms）—— 未在 10ms 预算处收敛"
    );
    // Assert 3: 超时不得被误报为完成
    assert_ne!(
        outcome,
        HeadlessOutcome::Completed,
        "超时被误报为 Completed —— 失败被吞"
    );
}

#[test]
fn c5_timeout_outcome_maps_to_exit_code_7() {
    // Arrange + Act: Timeout 终态的进程侧语义（C2/C5）
    let exit = exit_code_for(TurnStatus::Timeout);

    // Assert
    assert_eq!(exit, ExitCode::Timeout, "Timeout 终态必须映射 ExitCode::Timeout");
    assert_eq!(exit.code(), 7, "Timeout 的退出码必须为 7");
    assert_ne!(exit, ExitCode::Success, "超时绝不能以 0 退出");
}

//! [W2 / task-2] `TurnMetricsSnapshot` 契约验收（蓝军正向 + 同源比对）。
//!
//! 契约源（只读，逐字冻结）：`.dev-team/contract-matrix-metrics-snapshot.md` §1 I1–I7 / §4 字段表。
//! 单一真值：`crates/pony-agent-core/src/agent/metrics_snapshot.rs`。
//!
//! 纪律（test-expert L2 规范）：
//! - AAA 结构；固定夹具、无随机、无 sleep、无 `#[ignore]`、无「只跑不报错」的空断言。
//! - 红相锚定：`build_turn_metrics_snapshot` 当前为 `unimplemented!`，本文件**必须**失败；
//!   实施者（task-3）落地逻辑后本文件应全绿，且**不得修改本文件**（验收测试冻结）。
//!
//! 本文件由 Test Agent（task-2）独占写域。

use pony_agent_core::agent::metrics_snapshot::{build_turn_metrics_snapshot, TurnMetricsSnapshot};
use pony_agent_core::agent::telemetry::{
    ProviderCallCacheRecord, ProviderLatencyKind, ProviderRequestKind,
};
use pony_agent_core::agent::turn_flow::{weighted_turn_speed, ProviderEventMeta};

// ---------------------------------------------------------------------------
// 夹具（fixture）
// ---------------------------------------------------------------------------

/// 一条「可计数」的 provider 调用记录：`dur > 0 && out > 0`（weighted_turn_speed 的有效调用判据）。
///
/// `context_input_tokens` 由调用方显式给出（= 上游协议归一值），因为
/// `ProviderCallCacheRecord` **没有 cache_creation 字段**，anthropic 的
/// `input + cache_read + cache_creation` 只能由上游写进该归一字段（I4）。
fn record(input: u64, cache_hit: u64, output: u64, duration: u64) -> ProviderCallCacheRecord {
    ProviderCallCacheRecord {
        request_kind: ProviderRequestKind::InitialRequest,
        provider_source: Some("fixture-source".to_string()),
        provider_mode: Some("fixture-mode".to_string()),
        input_tokens: Some(input),
        cache_hit_input_tokens: Some(cache_hit),
        cache_hit_source: Some("provider".to_string()),
        cache_miss_input_tokens: Some(input.saturating_sub(cache_hit)),
        reasoning_tokens: None,
        output_tokens: Some(output),
        total_tokens: Some(input.saturating_add(output)),
        first_token_latency_ms: Some(200),
        turn_duration_ms: Some(duration),
        usage_origin: Some("provider".to_string()),
        context_input_tokens: Some(input),
        latency_kind: ProviderLatencyKind::ProviderStream,
        prefix_mutation_reasons: Vec::new(),
    }
}

/// 一条「全字段 None」的 planner-only / 伪记录（真实路径上确实会出现）。
fn all_none_record() -> ProviderCallCacheRecord {
    ProviderCallCacheRecord {
        request_kind: ProviderRequestKind::ToolFollowup,
        provider_source: None,
        provider_mode: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        cache_hit_source: None,
        cache_miss_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        usage_origin: None,
        context_input_tokens: None,
        latency_kind: ProviderLatencyKind::Unknown,
        prefix_mutation_reasons: Vec::new(),
    }
}

/// `provider_meta`：协议与窗口是 I3/I4 的输入，provider/model 是快照 `provider`/`model` 的唯一来源。
///
/// 注意：`model` 刻意使用虚构型号 `fixture-model-unpriced`——内置价格表「只收录可公开核价的主流模型，
/// 虚构型号一律不收录」，因此 `cost_usd` 必须为 `None`（未知 ⇒ `None`，禁 0 伪造）。
fn meta(protocol: &str, window: Option<u64>) -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: "fixture-request".to_string(),
        provider_name: "fixture-provider".to_string(),
        protocol: protocol.to_string(),
        model: "fixture-model-unpriced".to_string(),
        context_window_tokens: window,
    }
}

/// 浮点近似比较（契约要求 `context_ratio` 容差 1e-9）。
fn approx(actual: Option<f64>, expected: f64, tol: f64) -> bool {
    match actual {
        Some(value) => (value - expected).abs() <= tol,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// I1 —— 附着条件：records 为空 ⇒ None
// ---------------------------------------------------------------------------

#[test]
fn i1_empty_records_yield_none() {
    // Arrange：窗口与 meta 都齐备（**不许**用「没有窗口」当借口返回 None）
    let empty: Vec<ProviderCallCacheRecord> = Vec::new();
    let meta = meta("openai", Some(200_000));

    // Act
    let snapshot = build_turn_metrics_snapshot(&empty, Some(200_000), Some(&meta));

    // Assert：空记录无法产出任何 per-call 值；用 Default 冒充会让「未知」变成「0」真值。
    assert!(
        snapshot.is_none(),
        "I1：records 为空时 build_turn_metrics_snapshot 必须返回 None，实际：{snapshot:?}"
    );
}

// ---------------------------------------------------------------------------
// I4 —— 协议归一：anthropic 三者相加 / 其余协议取 input
// ---------------------------------------------------------------------------

#[test]
fn i4_anthropic_context_input_is_input_plus_cache_read_plus_cache_creation() {
    // Arrange：anthropic 侧 `input` 既不含 cache_read 也不含 cache_creation。
    //   input = 1_000 / cache_read(cache_hit) = 8_000 / cache_creation = 2_000
    //   ⇒ 归一上下文输入 = 1_000 + 8_000 + 2_000 = 11_000
    // record 无 cache_creation 字段 ⇒ 上游把归一值写进 `context_input_tokens`，实现必须以该字段为准。
    let mut anthropic = record(1_000, 8_000, 300, 2_000);
    anthropic.context_input_tokens = Some(11_000);
    let records = vec![anthropic];
    let meta = meta("anthropic", Some(200_000));

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, Some(200_000), Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert
    assert_eq!(
        snapshot.context_input_tokens,
        Some(11_000),
        "I4：anthropic 的 contextInputTokens 必须是 input + cache_read + cache_creation = 11000，\
         实际：{:?}（9000 = 漏 cache_creation；1000 = 只取裸 input）",
        snapshot.context_input_tokens
    );
}

#[test]
fn i4_openai_context_input_equals_input() {
    // Arrange：openai 侧 `input` 已含缓存命中 ⇒ 归一值就是 input 本身（不得再加 cache_hit）
    let openai = record(10_000, 9_000, 500, 2_000);
    let records = vec![openai];
    let meta = meta("openai", Some(200_000));

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, Some(200_000), Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert
    assert_eq!(
        snapshot.context_input_tokens,
        Some(10_000),
        "I4：非 anthropic 协议 contextInputTokens 必须等于裸 input = 10000，\
         实际：{:?}（19000 = 错误地对所有协议都加 cache_read）",
        snapshot.context_input_tokens
    );
}

// ---------------------------------------------------------------------------
// I2 —— 缓存命中比必须同量纲（禁止「轮累计命中 ÷ 末次 input」）
// ---------------------------------------------------------------------------

#[test]
fn i2_cache_hit_ratio_is_same_dimension_single_call() {
    // Arrange：多跳轮 —— 3 条记录，每条 cache_hit = 8_000 / input = 10_000（单跳命中率 0.8）。
    // 轮累计 cache_hit = 24_000；若用「累计 24000 ÷ 末次 input 10000」会得到 2.4，
    // 再被 clamp 成 100%，即契约点名禁止的「恒 100% 掩盖路径」。
    let records = vec![
        record(10_000, 8_000, 100, 1_000),
        record(10_000, 8_000, 100, 1_000),
        record(10_000, 8_000, 100, 1_000),
    ];
    let meta = meta("openai", Some(200_000));

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, Some(200_000), Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert
    let ratio = snapshot
        .cache_hit_ratio
        .expect("I2：分母 contextInputTokens = 10000 > 0，cacheHitRatio 必须有值（不得为 None）");
    assert_eq!(
        snapshot.cache_hit_input_tokens,
        Some(8_000),
        "I2：分子必须是**末次调用**的 cache_hit_input_tokens = 8000（不是轮累计 24000），\
         实际：{:?}",
        snapshot.cache_hit_input_tokens
    );
    assert!(
        (0.0..=1.0).contains(&ratio),
        "I2：cacheHitRatio 必须恒 ∈ [0,1]，实际：{ratio}"
    );
    assert!(
        (ratio - 0.8).abs() < 1e-12,
        "I2：cacheHitRatio 必须是同一次调用的 8000/10000 = 0.8，实际：{ratio}\
         （2.4 = 轮累计 24000 ÷ 末次 input 10000 的混量纲产物）"
    );
    assert!(
        ratio * 100.0 <= 100.0,
        "I2：换算后百分比绝不得超过 100%，实际：{}%",
        ratio * 100.0
    );
}

// ---------------------------------------------------------------------------
// I3 —— 上下文占比：context_input / window；窗口缺失 ⇒ None
// ---------------------------------------------------------------------------

#[test]
fn i3_context_ratio_is_context_input_over_window() {
    // Arrange：context_input = 10_000、window = 200_000 ⇒ 0.05。
    // 窗口同时由入参与本字段给出（同一真值），两种取值口径都必须得出 0.05。
    let records = vec![record(10_000, 0, 200, 1_000)];
    let meta = meta("openai", Some(200_000));

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, Some(200_000), Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert
    assert_eq!(
        snapshot.context_window_tokens,
        Some(200_000),
        "I3：快照必须透传上下文窗口容量"
    );
    let ratio = snapshot
        .context_ratio
        .expect("I3：窗口已知时 contextRatio 必须有值");
    assert!(
        (0.0..=1.0).contains(&ratio),
        "I3：contextRatio 必须恒 ∈ [0,1]，实际：{ratio}"
    );
    assert!(
        approx(snapshot.context_ratio, 0.05, 1e-9),
        "I3：contextRatio 必须是 10000/200000 = 0.05，实际：{ratio}"
    );
}

#[test]
fn i3_missing_window_yields_none_ratio() {
    // Arrange：窗口未知（入参与 meta 同时缺失，不得用固定 200k 常量兜底）
    let records = vec![record(10_000, 100, 200, 1_000)];
    let meta = meta("openai", None);

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, None, Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert：窗口未知 ⇒ 占比无定义（不得伪造成 0% 或 100%）
    assert!(
        snapshot.context_ratio.is_none(),
        "I3：窗口缺失时 contextRatio 必须为 None（不得回退成 0/1），实际：{:?}",
        snapshot.context_ratio
    );
    assert!(
        snapshot.context_window_tokens.is_none(),
        "I3：窗口缺失时 contextWindowTokens 必须为 None，实际：{:?}",
        snapshot.context_window_tokens
    );
}

// ---------------------------------------------------------------------------
// I5 —— 轮速度必须与 weighted_turn_speed 完全同源
// ---------------------------------------------------------------------------

/// 字段映射 + 同源比对共用的多跳夹具：
/// call#0: input 1000 / hit 100 / out 50 / dur 800 / ttft 200 ⇒ gen 600
/// call#1: input 2000 / hit 400 / out 80 / dur 1500 / ttft 200 ⇒ gen 1300
/// 轮累计：Σout = 130、Σgen = 1900、calls = 2。
fn mapping_fixture() -> (Vec<ProviderCallCacheRecord>, ProviderEventMeta) {
    let mut first = record(1_000, 100, 50, 800);
    first.reasoning_tokens = Some(7);
    let mut last = record(2_000, 400, 80, 1_500);
    last.reasoning_tokens = Some(11);
    last.total_tokens = Some(2_080);
    (vec![first, last], meta("openai", Some(200_000)))
}

#[test]
fn i5_turn_speed_is_same_source_as_weighted_turn_speed() {
    // Arrange
    let (records, meta) = mapping_fixture();
    let truth = weighted_turn_speed(&records);

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, Some(200_000), Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert：同一函数产出 ⇒ 严格相等（禁止另写 Σout/Σdur 之类公式）
    assert_eq!(
        snapshot.turn_speed_tokens_per_second, truth.speed,
        "I5：turnSpeedTokensPerSecond 必须与 weighted_turn_speed(records).speed 严格相等，\
         实际：{:?} vs {:?}",
        snapshot.turn_speed_tokens_per_second, truth.speed
    );
    assert_eq!(
        snapshot.turn_generation_ms, truth.generation_ms,
        "I5：turnGenerationMs 必须与 weighted_turn_speed 的 Σgen 同源"
    );
    assert_eq!(
        snapshot.turn_calls, truth.calls,
        "I5：turnCalls 必须是 weighted_turn_speed 的有效调用数"
    );
    assert_eq!(
        snapshot.turn_output_tokens, truth.output_tokens,
        "I5：turnOutputTokens 必须是有效调用的 Σout"
    );
    // 夹具自检（真实值，防止「同源但两者都算错」蒙混）：
    assert_eq!(truth.calls, 2, "夹具自检：两条有效调用");
    assert_eq!(truth.output_tokens, 130, "夹具自检：Σout = 50 + 80");
    assert_eq!(truth.generation_ms, 1_900, "夹具自检：Σgen = 600 + 1300");
}

// ---------------------------------------------------------------------------
// 字段映射（§4 字段表）：末次调用的 per-call 值 + provider_meta 的来源
// ---------------------------------------------------------------------------

#[test]
fn field_mapping_uses_last_callable_record_and_provider_meta() {
    // Arrange
    let (records, meta) = mapping_fixture();
    let last = records.last().expect("夹具非空").clone();

    // Act
    let snapshot = build_turn_metrics_snapshot(&records, Some(200_000), Some(&meta))
        .expect("非空 records 必须产出快照");

    // Assert：per-call 字段一律取**末次**记录（禁止跨调用求和）
    assert_eq!(
        snapshot.input_tokens, last.input_tokens,
        "inputTokens = 末次裸 input"
    );
    assert_eq!(
        snapshot.cache_hit_input_tokens, last.cache_hit_input_tokens,
        "cacheHitInputTokens = 末次缓存命中"
    );
    assert_eq!(
        snapshot.cache_miss_input_tokens, last.cache_miss_input_tokens,
        "cacheMissInputTokens = 末次缓存未命中"
    );
    assert_eq!(
        snapshot.output_tokens, last.output_tokens,
        "outputTokens = 末次输出"
    );
    assert_eq!(
        snapshot.reasoning_tokens, last.reasoning_tokens,
        "reasoningTokens = 末次推理 token"
    );
    assert_eq!(
        snapshot.total_tokens, last.total_tokens,
        "totalTokens = 末次总量"
    );
    assert_eq!(
        snapshot.first_token_latency_ms, last.first_token_latency_ms,
        "firstTokenLatencyMs = 末次 TTFT"
    );
    assert_eq!(
        snapshot.duration_ms, last.turn_duration_ms,
        "durationMs = 末次总耗时"
    );
    assert_eq!(
        snapshot.origin, last.usage_origin,
        "origin = 末次 usage 来源"
    );
    assert_eq!(
        snapshot.context_input_tokens,
        Some(2_000),
        "contextInputTokens = 末次协议归一上下文输入（openai ⇒ 裸 input 2000）"
    );
    assert_eq!(
        snapshot.context_window_tokens,
        Some(200_000),
        "contextWindowTokens = 入参窗口"
    );

    // 身份字段一律取 provider_meta
    assert_eq!(
        snapshot.provider, meta.provider_name,
        "provider 取 provider_meta"
    );
    assert_eq!(snapshot.model, meta.model, "model 取 provider_meta");

    // 成本：虚构型号不在价格表 ⇒ 未知 ⇒ None（禁 0.0 伪造）
    assert!(
        snapshot.cost_usd.is_none(),
        "costUsd：未知价格必须为 None（禁 0 伪造），实际：{:?}",
        snapshot.cost_usd
    );
}

// ---------------------------------------------------------------------------
// wire 契约（§4 camelCase）：前端 task-4 与 TUI task-3 都按这些键名消费
// ---------------------------------------------------------------------------

#[test]
fn wire_keys_are_camel_case_and_roundtrip() {
    // Arrange
    let snapshot = TurnMetricsSnapshot {
        provider: "fixture-provider".to_string(),
        model: "fixture-model-unpriced".to_string(),
        ..Default::default()
    };

    // Act
    let value = serde_json::to_value(&snapshot).expect("TurnMetricsSnapshot 必须可序列化");

    // Assert：22 个 camelCase 键（字段表逐条）
    let mut actual: Vec<String> = value
        .as_object()
        .expect("序列化结果必须是 JSON 对象")
        .keys()
        .cloned()
        .collect();
    actual.sort();
    let mut expected: Vec<String> = [
        "step",
        "contextInputTokens",
        "inputTokens",
        "cacheHitInputTokens",
        "cacheMissInputTokens",
        "outputTokens",
        "reasoningTokens",
        "totalTokens",
        "firstTokenLatencyMs",
        "durationMs",
        "callSpeedTokensPerSecond",
        "turnSpeedTokensPerSecond",
        "turnOutputTokens",
        "turnGenerationMs",
        "turnCalls",
        "cacheHitRatio",
        "contextWindowTokens",
        "contextRatio",
        "origin",
        "provider",
        "model",
        "costUsd",
    ]
    .iter()
    .map(|key| key.to_string())
    .collect();
    expected.sort();
    assert_eq!(
        actual, expected,
        "§4：wire 键名必须与冻结字段表的 camelCase 名逐字一致"
    );

    // 往返无损（TUI/前端反序列化同一结构）
    let roundtrip: TurnMetricsSnapshot =
        serde_json::from_value(value).expect("wire 结构必须可反序列化");
    assert_eq!(roundtrip, snapshot, "wire 往返必须无损");
}

//! [W2 / task-2] `TurnMetricsSnapshot` 对抗破坏用例（红军 / L2-AT，确定性、无 sleep）。
//!
//! 契约源（只读）：`.dev-team/contract-matrix-metrics-snapshot.md` §1 I1–I7。
//! 目标：畸形 usage / 计时输入下 **不 panic、不产出负值、不产出 NaN/Infinity**
//! （NaN/Inf 一律收敛为 `None`），且 I2/I3/I5 的不变量仍成立。
//!
//! 纪律：固定夹具、无随机、无 `sleep`、无 `#[ignore]`、每条用例均有非空断言。
//! 红相锚定：`build_turn_metrics_snapshot` 当前为 `unimplemented!` ⇒ 本文件必须失败。
//!
//! 本文件由 Test Agent（task-2）独占写域。

use pony_agent_core::agent::metrics_snapshot::{build_turn_metrics_snapshot, TurnMetricsSnapshot};
use pony_agent_core::agent::telemetry::{
    ProviderCallCacheRecord, ProviderLatencyKind, ProviderRequestKind,
};
use pony_agent_core::agent::turn_flow::{weighted_turn_speed, ProviderEventMeta};
use std::panic::{catch_unwind, AssertUnwindSafe};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

fn base_record() -> ProviderCallCacheRecord {
    ProviderCallCacheRecord {
        request_kind: ProviderRequestKind::InitialRequest,
        provider_source: Some("adversarial-source".to_string()),
        provider_mode: Some("adversarial-mode".to_string()),
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

fn meta() -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: "adversarial-request".to_string(),
        provider_name: "adversarial-provider".to_string(),
        protocol: "openai".to_string(),
        model: "adversarial-model-unpriced".to_string(),
        context_window_tokens: Some(200_000),
    }
}

/// 在 `catch_unwind` 里调用构建函数：panic 即失败（契约要求任何畸形输入都不得 panic）。
fn build_guarded(
    records: &[ProviderCallCacheRecord],
    window: Option<u64>,
    meta: Option<&ProviderEventMeta>,
) -> Option<TurnMetricsSnapshot> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        build_turn_metrics_snapshot(records, window, meta)
    }));
    assert!(
        outcome.is_ok(),
        "对抗不变量：畸形输入下 build_turn_metrics_snapshot 绝不允许 panic"
    );
    outcome.ok().flatten()
}

/// 全部浮点字段的卫生断言：`None` 或「有限且非负」；两个比值额外满足 I2/I3 的 [0,1]。
fn assert_sanitized(snapshot: &TurnMetricsSnapshot, case: &str) {
    let floats: [(&str, Option<f64>); 5] = [
        ("cache_hit_ratio", snapshot.cache_hit_ratio),
        ("context_ratio", snapshot.context_ratio),
        (
            "call_speed_tokens_per_second",
            snapshot.call_speed_tokens_per_second,
        ),
        (
            "turn_speed_tokens_per_second",
            snapshot.turn_speed_tokens_per_second,
        ),
        ("cost_usd", snapshot.cost_usd),
    ];
    for (name, value) in floats {
        if let Some(v) = value {
            assert!(
                v.is_finite(),
                "[{case}] {name} 不得为 NaN/Infinity，实际：{v}"
            );
            assert!(v >= 0.0, "[{case}] {name} 不得为负值，实际：{v}");
        }
    }
    if let Some(ratio) = snapshot.cache_hit_ratio {
        assert!(
            (0.0..=1.0).contains(&ratio),
            "[{case}] I2：cacheHitRatio 必须 ∈ [0,1]，实际：{ratio}"
        );
    }
    if let Some(ratio) = snapshot.context_ratio {
        assert!(
            (0.0..=1.0).contains(&ratio),
            "[{case}] I3：contextRatio 必须 ∈ [0,1]，实际：{ratio}"
        );
    }
}

/// 同源断言（I5）：凡产出快照，轮速度/生成时长/调用数/输出都必须与 `weighted_turn_speed` 一致。
fn assert_same_source(
    records: &[ProviderCallCacheRecord],
    snapshot: &TurnMetricsSnapshot,
    case: &str,
) {
    let truth = weighted_turn_speed(records);
    assert_eq!(
        snapshot.turn_speed_tokens_per_second, truth.speed,
        "[{case}] I5：轮速度必须与 weighted_turn_speed 完全同源"
    );
    assert_eq!(
        snapshot.turn_generation_ms, truth.generation_ms,
        "[{case}] I5：turnGenerationMs 必须与 weighted_turn_speed 的 Σgen 同源"
    );
    assert_eq!(
        snapshot.turn_calls, truth.calls,
        "[{case}] I5：turnCalls 必须等于有效调用数"
    );
    assert_eq!(
        snapshot.turn_output_tokens, truth.output_tokens,
        "[{case}] I5：turnOutputTokens 必须等于 Σout"
    );
}

// ---------------------------------------------------------------------------
// 1. duration = 0 + output = 100（除零 / 0 时长）
// ---------------------------------------------------------------------------

#[test]
fn a1_zero_duration_never_panics_or_emits_infinity() {
    // Arrange：dur = 0 但同时有 output ⇒ 速度公式分母为 0
    let mut record = base_record();
    record.input_tokens = Some(1_000);
    record.cache_hit_input_tokens = Some(0);
    record.cache_hit_source = Some("provider".to_string());
    record.output_tokens = Some(100);
    record.turn_duration_ms = Some(0);
    record.first_token_latency_ms = Some(200);
    record.usage_origin = Some("provider".to_string());
    record.context_input_tokens = Some(1_000);
    let records = vec![record];
    let meta = meta();

    // Act
    let snapshot = build_guarded(&records, Some(200_000), Some(&meta));

    // Assert
    if let Some(snapshot) = &snapshot {
        assert_sanitized(snapshot, "dur=0");
        assert_same_source(&records, snapshot, "dur=0");
        assert!(
            snapshot.call_speed_tokens_per_second.is_none(),
            "dur=0 ⇒ 单次调用速度无定义，必须为 None（不得 Infinity），实际：{:?}",
            snapshot.call_speed_tokens_per_second
        );
    }
    assert!(
        weighted_turn_speed(&records).speed.is_none(),
        "夹具自检：dur=0 的记录不是有效调用，轮速度必须为 None"
    );
}

// ---------------------------------------------------------------------------
// 2. ttft > duration（计时倒挂）
// ---------------------------------------------------------------------------

#[test]
fn a2_ttft_greater_than_duration_never_goes_negative() {
    // Arrange：ttft = 5000ms > dur = 1000ms ⇒ gen = dur - ttft 若直接相减会下溢/负数
    let mut record = base_record();
    record.input_tokens = Some(1_000);
    record.cache_hit_input_tokens = Some(0);
    record.cache_hit_source = Some("provider".to_string());
    record.output_tokens = Some(100);
    record.turn_duration_ms = Some(1_000);
    record.first_token_latency_ms = Some(5_000);
    record.usage_origin = Some("provider".to_string());
    record.context_input_tokens = Some(1_000);
    let records = vec![record];
    let meta = meta();

    // Act
    let snapshot = build_guarded(&records, Some(200_000), Some(&meta))
        .expect("含可计数调用的记录必须产出快照");

    // Assert：退化口径 gen = dur（不得出现负值/NaN）
    assert_sanitized(&snapshot, "ttft>dur");
    assert_same_source(&records, &snapshot, "ttft>dur");
    assert_eq!(
        snapshot.turn_generation_ms, 1_000,
        "ttft>dur 必须退化为 gen = dur = 1000（不得出现 u64 下溢或负值）"
    );
    assert_eq!(
        snapshot.turn_speed_tokens_per_second,
        Some(100.0),
        "ttft>dur ⇒ 轮速度 = 100 / (1000/1000) = 100，实际：{:?}",
        snapshot.turn_speed_tokens_per_second
    );
}

// ---------------------------------------------------------------------------
// 3. input = 0 + cache_hit = 500（比值除零）
// ---------------------------------------------------------------------------

#[test]
fn a3_zero_context_input_yields_none_ratio() {
    // Arrange：分母（归一上下文输入）为 0，分子 500
    let mut record = base_record();
    record.input_tokens = Some(0);
    record.cache_hit_input_tokens = Some(500);
    record.cache_hit_source = Some("provider".to_string());
    record.output_tokens = Some(100);
    record.turn_duration_ms = Some(1_000);
    record.first_token_latency_ms = Some(100);
    record.usage_origin = Some("provider".to_string());
    record.context_input_tokens = Some(0);
    let records = vec![record];
    let meta = meta();

    // Act
    let snapshot = build_guarded(&records, Some(200_000), Some(&meta))
        .expect("含可计数调用的记录必须产出快照");

    // Assert：I2「分母缺失/为 0 ⇒ None」——不得 500/0 = Infinity，也不得 clamp 成 1.0/100%
    assert_eq!(
        snapshot.cache_hit_ratio, None,
        "I2：contextInputTokens = 0 ⇒ cacheHitRatio 必须为 None，实际：{:?}",
        snapshot.cache_hit_ratio
    );
    assert_sanitized(&snapshot, "input=0");
    assert_same_source(&records, &snapshot, "input=0");
}

// ---------------------------------------------------------------------------
// 4. 全字段 None 的伪记录
// ---------------------------------------------------------------------------

#[test]
fn a4_all_none_record_never_panics() {
    // Arrange：planner-only 伪记录（真实路径确实存在）
    let records = vec![base_record()];
    let meta = meta();

    // Act
    let snapshot = build_guarded(&records, Some(200_000), Some(&meta));

    // Assert
    if let Some(snapshot) = &snapshot {
        assert_sanitized(snapshot, "all-none");
        assert_same_source(&records, snapshot, "all-none");
        assert_eq!(snapshot.turn_calls, 0, "全 None 记录不得计入有效调用");
        assert_eq!(
            snapshot.context_input_tokens, None,
            "全 None 记录不得把上下文输入伪造成 0"
        );
        assert_eq!(
            snapshot.cache_hit_ratio, None,
            "全 None 记录不得把缓存比伪造成 0"
        );
    }
    assert_eq!(
        weighted_turn_speed(&records).calls,
        0,
        "夹具自检：全 None 记录不是有效调用"
    );
}

// ---------------------------------------------------------------------------
// 5. 混合：[有效, 全 None, ttft>dur]
// ---------------------------------------------------------------------------

#[test]
fn a5_mixed_records_count_only_valid_calls() {
    // Arrange
    let mut valid = base_record();
    valid.input_tokens = Some(1_000);
    valid.cache_hit_input_tokens = Some(400);
    valid.cache_hit_source = Some("provider".to_string());
    valid.cache_miss_input_tokens = Some(600);
    valid.output_tokens = Some(100);
    valid.turn_duration_ms = Some(2_000);
    valid.first_token_latency_ms = Some(500);
    valid.usage_origin = Some("provider".to_string());
    valid.context_input_tokens = Some(1_000);
    valid.total_tokens = Some(1_100);

    let mut inverted = base_record();
    inverted.input_tokens = Some(2_000);
    inverted.cache_hit_input_tokens = Some(800);
    inverted.cache_hit_source = Some("provider".to_string());
    inverted.output_tokens = Some(50);
    inverted.turn_duration_ms = Some(1_000);
    inverted.first_token_latency_ms = Some(5_000);
    inverted.usage_origin = Some("provider".to_string());
    inverted.context_input_tokens = Some(2_000);

    let records = vec![valid, base_record(), inverted];
    let meta = meta();
    let truth = weighted_turn_speed(&records);

    // Act
    let snapshot = build_guarded(&records, Some(200_000), Some(&meta))
        .expect("混合记录中含有效调用，必须产出快照");

    // Assert：只计有效调用（全 None 记录被过滤），速度有限且 > 0
    assert_eq!(truth.calls, 2, "夹具自检：有效调用 = 2（全 None 记录不计）");
    assert_eq!(
        snapshot.turn_calls, 2,
        "turnCalls 必须只计有效调用（dur>0 && out>0），实际：{}",
        snapshot.turn_calls
    );
    assert_eq!(snapshot.turn_output_tokens, 150, "Σout = 100 + 50");
    assert_eq!(
        snapshot.turn_generation_ms, 2_500,
        "Σgen = 1500 + 1000（倒挂退化）"
    );
    let speed = snapshot
        .turn_speed_tokens_per_second
        .expect("存在有效调用 ⇒ 轮速度必须有值");
    assert!(speed.is_finite(), "轮速度必须有限，实际：{speed}");
    assert!(speed > 0.0, "轮速度必须为正，实际：{speed}");
    assert!(
        (speed - 150.0 / 2.5).abs() < 1e-9,
        "轮速度 = 150 / (2500/1000) = 60，实际：{speed}"
    );
    assert_sanitized(&snapshot, "mixed");
    assert_same_source(&records, &snapshot, "mixed");
}

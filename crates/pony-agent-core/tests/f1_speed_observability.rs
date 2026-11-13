//! [diag-tps / task-4] F1「口径可取证」验收（独立测试者 L2-T，红相先写、冻结）。
//!
//! 契约真值源（Lead 冻结，只读）：共享任务板 `task-4` 的 I1–I5。
//! 判据口径（本文件只断言语义，不绑实现细节）：
//! - I1：轮终态事件 payload 必须含 `turnSpeedTokensPerSecond` / `turnSpeedBasis` /
//!   `turnCallsTotal` / `turnCallsEligible` / `turnBurstExcluded`；**无值时不写键**
//!   （禁止 0/NaN 占位）。
//! - I3：`speed` 与 `basis` 必须与 `turn_flow::weighted_turn_speed(records)` **同源同帧**
//!   （本文件把 `weighted_turn_speed` 当唯一真值，逐值比对 payload，禁止消费面另写公式）。
//! - I4（显示端样本量/口径可表达）由 `src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts`
//!   覆盖（TS 侧）；本文件只负责 core 落盘面。
//!
//! 键位形态说明（**故意放宽**，不绑结构）：终态事件 payload 是 camelCase JSON，
//! 新字段可能落在顶层，也可能落在既有的 `metrics`（`TurnMetricsSnapshot`）子对象里。
//! 本文件用**递归展平 + 末段键名**匹配，任何嵌套深度都接受；只锁键名语义与取值，
//! 不锁嵌套层数。实现者请勿为了让下方用例变绿而改本文件（验收测试冻结，实施者只读）。
//!
//! 纪律（test-expert L2）：
//! - AAA 结构；固定夹具、无随机、无 sleep、无 `#[ignore]`、无「只跑不报错」的空断言；
//! - 红相锚定：当前基线（未修复）`turn:completed` 只带 `metrics.turnCalls`（≤ 参与计算集合的
//!   调用数），**没有** total/eligible/burstExcluded 三键 ⇒ `f1_*` 必失败（断言失败，非编译错误）。
//!
//! 本文件由 Test Agent 独占写域。

use std::sync::Mutex;

use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::telemetry::{
    ProviderCallCacheRecord, ProviderLatencyKind, ProviderRequestKind,
};
use pony_agent_core::agent::turn_flow::{
    emit_stream_event, weighted_turn_speed, ProviderEventMeta, TurnEventSink,
};
use serde_json::Value;

// ---------------------------------------------------------------------------
// 记录型 sink：冻结判据的证据面 = `(event_name, payload)` 序列
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RecordingSink {
    captured: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self::default()
    }

    fn events_named(&self, name: &str) -> Vec<TurnStreamEvent> {
        self.captured
            .lock()
            .expect("F1：recording sink 锁被毒化")
            .iter()
            .filter(|(captured_name, _)| captured_name == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.captured
            .lock()
            .expect("F1：recording sink 锁被毒化")
            .push((name.to_string(), payload));
    }
}

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// per-call 记录（除时间/用量外一律固定，避免随机与网络）。
fn record(
    duration_ms: Option<u64>,
    ttft_ms: Option<u64>,
    output_tokens: Option<u64>,
) -> ProviderCallCacheRecord {
    ProviderCallCacheRecord {
        request_kind: ProviderRequestKind::InitialRequest,
        provider_source: Some("f1-fixture-source".to_string()),
        provider_mode: Some("f1-fixture-mode".to_string()),
        input_tokens: Some(1_000),
        cache_hit_input_tokens: Some(400),
        cache_hit_source: Some("provider".to_string()),
        cache_miss_input_tokens: Some(600),
        reasoning_tokens: None,
        output_tokens,
        total_tokens: output_tokens.map(|output| 1_000_u64.saturating_add(output)),
        first_token_latency_ms: ttft_ms,
        turn_duration_ms: duration_ms,
        usage_origin: Some("provider".to_string()),
        context_input_tokens: Some(1_000),
        latency_kind: ProviderLatencyKind::ProviderStream,
        prefix_mutation_reasons: Vec::new(),
    }
}

fn provider_meta() -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: "f1-fixture-request".to_string(),
        provider_name: "f1-fixture-provider".to_string(),
        protocol: "openai".to_string(),
        // 虚构型号：内置价格表不收录 ⇒ 与并发修复的计价分支解耦。
        model: "f1-fixture-model-unpriced".to_string(),
        context_window_tokens: Some(200_000),
    }
}

/// **无歧义**夹具：两条可计数调用，一种口径命运各自一条（契约 I1/I3 的判据素材）。
///
/// | # | dur | ttft | out | gen=dur-ttft | floor(dur) | 判据 |
/// |---|-----|------|-----|--------------|------------|------|
/// | A | 1000 | 200 | 100 | 800 | 200 | **decode eligible**（gen ≥ floor 且 out ≥ 2） |
/// | B | 400 | 300 | 50 | 100 | 200 | **突发包**（有 ttft、out ≥ 2，但 gen=100 < floor=200） |
///
/// ⇒ 样本量真值：total = 2、eligible = 1、burstExcluded = **1**。
///
/// 为什么刻意**不**混入「无 TTFT 观测」的记录：那种排除项的归属在冻结契约里没有定义
/// （可归入「剔除」也可单列），混进来会让本用例的期望值取决于实现自选口径而不是契约。
/// 无歧义夹具让任何自洽定义都必须给出 1 —— 这才是可冻结的判据。
fn fixture_pure_burst_records() -> Vec<ProviderCallCacheRecord> {
    vec![
        record(Some(1_000), Some(200), Some(100)),
        record(Some(400), Some(300), Some(50)),
    ]
}

/// **混合**夹具：在无歧义夹具之上追加一条「无 TTFT 观测」的调用（非突发排除项）。
///
/// 该记录只能进 `end_to_end` 回退；它是否计入 `turnBurstExcluded` 属实现口径
/// （冻结契约未定义）⇒ 本夹具只断言**区间与恒等式**，不钉死具体值：
/// total = 3、eligible = 1、burstExcluded ∈ [1, 2]，且 `eligible + burstExcluded ≤ total`。
fn fixture_mixed_records() -> Vec<ProviderCallCacheRecord> {
    let mut records = fixture_pure_burst_records();
    records.push(record(Some(500), None, Some(40)));
    records
}

/// 触发**一轮终态事件发射**（生产唯一收口：`emit_stream_event` 在此处按 records 成型
/// `metrics` 快照；task-4 的 I1/I2 新字段也必须经同一收口上线缆/落盘）。
fn emit_terminal_turn_event(
    sink: &RecordingSink,
    records: Vec<ProviderCallCacheRecord>,
    turn_duration_ms: Option<u64>,
) {
    let meta = provider_meta();
    emit_stream_event(
        sink,
        "turn:completed",
        "turn-f1-fixture".to_string(),
        "completed",
        Some("completed"),
        Some("F1 fixture answer".to_string()),
        None,
        Some(&meta),
        Some("f1-fixture-source".to_string()),
        Some("f1-fixture-mode".to_string()),
        None,  // fallback_reason
        None,  // build_context_observation
        Some(1_000), // input_tokens
        Some(400),   // cache_hit_input_tokens
        None,        // reasoning_tokens
        Some(190),   // output_tokens
        Some(1_190), // total_tokens
        Some(200),   // first_token_latency_ms
        turn_duration_ms,
        None, // trace_steps
        None, // trace_timeline
        None, // tool_activities
        Some(records),
        None, // hook_trace_records
        None, // session_summary
        Some("session-f1-fixture".to_string()),
    );
}

// ---------------------------------------------------------------------------
// JSON 展平：键位匹配不绑嵌套深度
// ---------------------------------------------------------------------------

/// 递归展平成 `(dotted_path, leaf_value)`；数组下标写成 `[i]`。
fn flatten(value: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, &path, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, &format!("{prefix}[{index}]"), out);
            }
        }
        leaf => out.push((prefix.to_string(), leaf.clone())),
    }
}

fn flattened(payload: &TurnStreamEvent) -> Vec<(String, Value)> {
    let json = serde_json::to_value(payload).expect("F1：终态事件 payload 必须可序列化为 JSON");
    let mut out = Vec::new();
    flatten(&json, "", &mut out);
    out
}

/// 按**末段键名**取全部命中（允许 `metrics.turnCallsTotal` 与顶层 `turnCallsTotal` 两种形态）。
fn field_hits(flat: &[(String, Value)], key: &str) -> Vec<(String, Value)> {
    flat.iter()
        .filter(|(path, _)| {
            path.rsplit('.')
                .next()
                .map(|segment| segment == key)
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// 单键取值：命中 0 次 ⇒ `Err(缺失)`；命中 ≥1 次 ⇒ 要求所有命中的取值一致（防「同名两处各写一份」）。
fn single_value(flat: &[(String, Value)], key: &str) -> Result<Value, String> {
    let hits = field_hits(flat, key);
    let Some((first_path, first_value)) = hits.first() else {
        return Err(format!(
            "契约 I1 要求终态 payload 含键 `{key}`，实际缺失；payload 全部键路径 = {:?}",
            flat.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>()
        ));
    };
    for (path, value) in hits.iter().skip(1) {
        assert_eq!(
            value, first_value,
            "契约 I3：`{key}` 只允许一个真值出口，实际 {} 与 {} 不一致",
            first_path, path
        );
    }
    Ok(first_value.clone())
}

fn as_u64(value: &Value) -> Option<u64> {
    value.as_u64()
}

// ---------------------------------------------------------------------------
// I1 + I3：终态 payload 必须携带「速度 + 口径 + 样本量三键」，且与唯一真值同源
// ---------------------------------------------------------------------------

#[test]
fn f1_terminal_event_payload_carries_speed_basis_and_sample_counts() {
    // Arrange：两条可计数调用（A 可解码 / B 突发包），真值由唯一公式出口给出。
    let records = fixture_pure_burst_records();
    let truth = weighted_turn_speed(&records);
    assert_eq!(
        truth.basis.map(|basis| basis.as_str()),
        Some("stream_decode"),
        "F1 夹具前提：只有 A 可解码 ⇒ 轮速度必须走 stream_decode 口径（真值 = {truth:?}）"
    );
    let sink = RecordingSink::new();

    // Act
    emit_terminal_turn_event(&sink, records, Some(1_900));

    // Assert（证据面：终态事件 payload 的 JSON 展平结果）
    let payloads = sink.events_named("turn:completed");
    assert_eq!(
        payloads.len(),
        1,
        "F1：一轮只允许一条终态 `turn:completed`，实际 {} 条",
        payloads.len()
    );
    let flat = flattened(&payloads[0]);

    // ① 速度可取证（I1）且与 weighted_turn_speed 同源（I3）。
    let speed = single_value(&flat, "turnSpeedTokensPerSecond")
        .unwrap_or_else(|error| panic!("{error}"));
    let speed = speed.as_f64().unwrap_or_else(|| {
        panic!("契约 I1：`turnSpeedTokensPerSecond` 必须是数字（禁字符串/占位），实际 {speed:?}")
    });
    let expected_speed = truth.speed.expect("F1 夹具前提：truth.speed 必须存在");
    assert!(
        (speed - expected_speed).abs() <= 1e-6,
        "契约 I3：payload 的 turnSpeedTokensPerSecond 必须等于 `weighted_turn_speed` 的唯一真值 {expected_speed}，实际 {speed}"
    );

    // ② 口径可取证（I1/I10）：`stream_decode` | `end_to_end`，且与速度同源。
    let basis = single_value(&flat, "turnSpeedBasis").unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        basis.as_str(),
        truth.basis.map(|basis| basis.as_str()),
        "契约 I1/I3：`turnSpeedBasis` 必须与 truth.basis 同源同帧，实际 {basis:?}"
    );

    // ③ 样本量可取证（I1）：total / eligible / burstExcluded 三键必须可读。
    let total = single_value(&flat, "turnCallsTotal").unwrap_or_else(|error| panic!("{error}"));
    let eligible = single_value(&flat, "turnCallsEligible").unwrap_or_else(|error| panic!("{error}"));
    let burst = single_value(&flat, "turnBurstExcluded").unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        as_u64(&total),
        Some(2),
        "契约 I1：turnCallsTotal 必须是本轮 provider 调用总数 2（两条记录 dur>0 && out>0），实际 {total:?}"
    );
    assert_eq!(
        as_u64(&eligible),
        Some(1),
        "契约 I1：turnCallsEligible 必须是 `decode_eligible` 调用数 1（只有 A 可解码），实际 {eligible:?}"
    );
    assert_eq!(
        as_u64(&burst),
        Some(1),
        "契约 I1：turnBurstExcluded 必须是突发包剔除数 1（B：gen=100 < floor(400)=200），\
         实际 {burst:?}（=0 说明根本没统计；>1 说明把非突发记录也算进突发剔除）"
    );

    // ④ 口径自证：样本量必须能自解释速度口径（eligible = 参与 stream_decode 的调用数）。
    assert_eq!(
        as_u64(&eligible),
        Some(u64::from(truth.calls)),
        "契约 I3：stream_decode 口径下 turnCallsEligible 必须等于 truth.calls（{truth_calls}），实际 {eligible:?}",
        truth_calls = truth.calls
    );
}

#[test]
fn f1_sample_counts_are_consistent_on_mixed_exclusion_reasons() {
    // Arrange：混合夹具 = 可解码 1 条 + 突发 1 条 + 无 TTFT 1 条。
    //   后两者的归属（是否都算「突发剔除」）在冻结契约里未定义 ⇒ 只钉恒等式与区间。
    let records = fixture_mixed_records();
    let sink = RecordingSink::new();

    // Act
    emit_terminal_turn_event(&sink, records, Some(2_400));

    // Assert
    let payloads = sink.events_named("turn:completed");
    assert_eq!(payloads.len(), 1, "F1：一轮只允许一条终态事件");
    let flat = flattened(&payloads[0]);
    let total = single_value(&flat, "turnCallsTotal")
        .unwrap_or_else(|error| panic!("{error}"))
        .as_u64()
        .unwrap_or_else(|| panic!("契约 I1：turnCallsTotal 必须是整数"));
    let eligible = single_value(&flat, "turnCallsEligible")
        .unwrap_or_else(|error| panic!("{error}"))
        .as_u64()
        .unwrap_or_else(|| panic!("契约 I1：turnCallsEligible 必须是整数"));
    let burst = single_value(&flat, "turnBurstExcluded")
        .unwrap_or_else(|error| panic!("{error}"))
        .as_u64()
        .unwrap_or_else(|| panic!("契约 I1：turnBurstExcluded 必须是整数"));

    assert_eq!(total, 3, "契约 I1：混合夹具共 3 条可计数调用，实际 {total}");
    assert_eq!(eligible, 1, "契约 I1：只有 1 条可解码调用，实际 {eligible}");
    assert!(
        burst >= 1,
        "契约 I1/I4：至少 1 条被排除（突发包 B），burstExcluded 必须 > 0（否则显示端无法辨别剔除），实际 {burst}"
    );
    assert!(
        eligible + burst <= total,
        "契约 I1：样本量恒等式 eligible + burstExcluded ≤ total 必须成立，实际 {eligible}+{burst} > {total}"
    );
}

// ---------------------------------------------------------------------------
// I1 反向钉子：无值时不写键（禁止 0 / NaN 占位）
// ---------------------------------------------------------------------------

#[test]
fn f1_terminal_event_payload_omits_speed_keys_when_no_value() {
    // Arrange：可计数但**无值**的调用（dur=0 / out=0 一律不参与任何 Σ）⇒ 速度真值全 None。
    //   关键点：此形态仍会成型 `metrics` 快照（不是「没有快照」那种真空场景），
    //   因此能真正约束「快照存在但无值 ⇒ 不写键」。
    let records = vec![record(Some(0), Some(0), Some(0))];
    let truth = weighted_turn_speed(&records);
    assert_eq!(truth.speed, None, "F1 夹具前提：零时长/零输出的记录不得产出速度真值");
    assert_eq!(truth.basis, None, "F1 夹具前提：无速度 ⇒ 无口径");
    let sink = RecordingSink::new();

    // Act
    emit_terminal_turn_event(&sink, records, Some(0));

    // Assert
    let payloads = sink.events_named("turn:completed");
    assert_eq!(payloads.len(), 1, "F1：一轮只允许一条终态事件");
    let flat = flattened(&payloads[0]);

    // ① 速度口径两键：无值 ⇒ 键缺席或 JSON null（**禁止** 0 / NaN 占位）。
    for key in ["turnSpeedTokensPerSecond", "turnSpeedBasis"] {
        for (path, value) in field_hits(&flat, key) {
            assert!(
                value.is_null(),
                "契约 I1：无值场景禁止给 `{path}` 写 0/NaN 等占位值，实际 {value:?}\
                 （无值 ⇒ 键缺席，或至少是 JSON null）"
            );
        }
    }

    // ② 样本量三键：计数本身是**已知事实**（确实发生过 1 次调用），允许出现；
    //    但不得伪造成「有解码样本」或「剔除了突发包」——这两个数必须为 0。
    if let Some(value) = field_hits(&flat, "turnCallsEligible").first().map(|(_, value)| value) {
        assert_eq!(
            as_u64(value),
            Some(0),
            "契约 I1：无可用解码样本时 turnCallsEligible 必须是 0，实际 {value:?}"
        );
    }
    if let Some(value) = field_hits(&flat, "turnBurstExcluded").first().map(|(_, value)| value) {
        assert_eq!(
            as_u64(value),
            Some(0),
            "契约 I1：无可用解码样本时 turnBurstExcluded 必须是 0（不得伪造剔除），实际 {value:?}"
        );
    }
    if let Some(value) = field_hits(&flat, "turnCallsTotal").first().map(|(_, value)| value) {
        let total = as_u64(value).unwrap_or_else(|| {
            panic!("契约 I1：turnCallsTotal 必须是整数，实际 {value:?}")
        });
        assert!(
            total <= 1,
            "契约 I1：本轮实际只登记 1 条调用记录，turnCallsTotal 不得超过 1，实际 {total}"
        );
    }
}

// ---------------------------------------------------------------------------
// I1 反向钉子之二：没有任何可计数调用时，五个键一律不出现（含 null）
// ---------------------------------------------------------------------------

#[test]
fn f1_empty_records_omit_all_speed_keys() {
    // Arrange：空 records ⇒ 无法产出任何 per-call 值（不是「0 次调用」，是「未知」）。
    let sink = RecordingSink::new();

    // Act
    emit_terminal_turn_event(&sink, Vec::new(), None);

    // Assert
    let payloads = sink.events_named("turn:completed");
    assert_eq!(payloads.len(), 1, "F1：一轮只允许一条终态事件");
    let flat = flattened(&payloads[0]);
    let mut present: Vec<&str> = Vec::new();
    for key in [
        "turnSpeedTokensPerSecond",
        "turnSpeedBasis",
        "turnCallsTotal",
        "turnCallsEligible",
        "turnBurstExcluded",
    ] {
        if !field_hits(&flat, key).is_empty() {
            present.push(key);
        }
    }
    assert!(
        present.is_empty(),
        "契约 I1：无任何 per-call 记录时五个速度/样本量键必须全部缺席（禁 0/NaN 占位），实际出现：{present:?}"
    );
}

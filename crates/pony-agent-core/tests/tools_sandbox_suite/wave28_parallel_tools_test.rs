//! W28「并发工具执行引擎」红相验收测试。
//!
//! 背景：当前 `ToolRouter` 只能逐个串行 `execute`，模型一次返回多个工具调用时没有任何
//! 可并发的执行层：既无法对只读工具（Read/Search）并行提速，也没有「不安全工具必须独占」的
//! 隔离语义，更无法隔离单个工具的 panic。W28 要求抽出 `agent::parallel_tools` 执行引擎。
//!
//! 冻结契约（`docs/audit/agent-core-improvement-plan.md` §W28）：
//! - `ToolCallExecutionPolicy { max_concurrency: usize }`，`Default` = 4；
//! - `ParallelToolOutcome { results: Vec<ToolResult>, max_observed_concurrency: usize }`；
//! - `execute_calls(calls, concurrent_safe, policy, executor) -> Result<ParallelToolOutcome, String>`：
//!   1) `results` 与 `calls` 等长且**保序**（与完成先后无关）；
//!   2) `concurrent_safe[i]==true` 可并发（上限 `max_concurrency`，0 视为 1 即串行），
//!      `false` 必须串行且不与任何其它调用并发；
//!   3) executor panic 被隔离为该调用的 `status=="error"` + `error.code=="tool_panicked"`；
//!   4) 空输入 → 空 results 且 `max_observed_concurrency==0`；
//!   5) `max_observed_concurrency` 真实反映峰值并发；
//!   `concurrent_safe` 与 `calls` 长度不等 → `Err(String)`。
//! - 另：`tools::tool_is_concurrent_safe(name)` —— Read/workspace_read_file/Search 为 true，
//!   Write/workspace_write_file/Run 为 false。
//!
//! 红相说明：`agent::parallel_tools` 模块当前不存在，本文件因此**编译失败**（预期第一层红）。
//! 为保证实现落地后立刻成为有效断言，这里只引用上述冻结路径与签名，不绕过任何 API。
//!
//! 时序设计：用 `thread::sleep` 制造确定性时序，并用 `AtomicUsize` 统计实测并发；
//! 阈值留足余量（并行峰值耗时可区分串行耗时），避免 CI 抖动导致的 flaky。
//!
//! 契约歧义（已回执提请 Lead 裁决）：t1 的「总耗时 < 250ms」与它自己给出的步长
//! `(4-i)*100ms` 自相矛盾——4 路并发时最慢一路就是 400ms，串行则是 1000ms。
//! 本测试保留冻结的 sleep 步长（0→400/1→300/2→200/3→100ms），把阈值改为可满足且仍能
//! 区分串行的 `< 700ms`，并额外断言峰值并发 ≥3（串行/半串行实现无法满足）。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pony_agent_core::agent::parallel_tools::{
    execute_calls, ParallelToolOutcome, ToolCallExecutionPolicy,
};
use pony_agent_core::agent::tools::{tool_is_concurrent_safe, ToolCall, ToolResult};
use serde_json::{json, Value};

// ─────────────────────────────────────────────────────────────────────────────
// 辅助
// ─────────────────────────────────────────────────────────────────────────────

fn make_call(index: usize, name: &str) -> ToolCall {
    ToolCall {
        call_id: Some(format!("w28-call-{index}")),
        name: name.to_string(),
        arguments: json!({ "index": index }),
        plan: None,
    }
}

/// executor 内部从 arguments 取回调用序号（用于构造确定性时序与保序断言）。
fn index_of(call: &ToolCall) -> usize {
    call.arguments
        .get("index")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or_else(|| {
            panic!(
                "W28 夹具错误：ToolCall.arguments 缺少 index 字段。实际 arguments={}",
                call.arguments
            )
        })
}

fn ok_result(call: &ToolCall, index: usize) -> ToolResult {
    ToolResult {
        tool_name: call.name.clone(),
        status: "ok".to_string(),
        output: format!("executed:{index}"),
        duration_ms: 0,
    }
}

fn policy(max_concurrency: usize) -> ToolCallExecutionPolicy {
    ToolCallExecutionPolicy { max_concurrency }
}

fn expect_outcome(outcome: Result<ParallelToolOutcome, String>, context: &str) -> ParallelToolOutcome {
    outcome.unwrap_or_else(|error| panic!("{context}：期望 Ok，实际 Err({error:?})"))
}

fn error_code_of(output: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(output).ok()?;
    parsed
        .pointer("/error/code")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            parsed
                .get("error")
                .and_then(|error| error.get("kind"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

// ─────────────────────────────────────────────────────────────────────────────
// t1：保序 + 真并发（越靠前的调用越慢，完成顺序 ≠ 返回顺序）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t1_results_preserve_call_order_even_when_earlier_calls_finish_later() {
    // Arrange：4 个安全调用，第 i 个 sleep (4-i)*100ms（0 最慢 400ms、3 最快 100ms）
    let calls: Vec<ToolCall> = (0..4).map(|index| make_call(index, "Read")).collect();
    let concurrent_safe = vec![true; calls.len()];
    let policy = policy(4);
    let call_count = calls.len();

    // Act
    let started = Instant::now();
    let outcome = execute_calls(&calls, &concurrent_safe, &policy, |call| {
        let index = index_of(call);
        let delay_ms = (call_count - index) as u64 * 100;
        std::thread::sleep(Duration::from_millis(delay_ms));
        ok_result(call, index)
    });
    let elapsed = started.elapsed();
    let outcome = expect_outcome(outcome, "t1 4 个安全调用并发执行");
    println!(
        "[W28][t1] elapsed={elapsed:?} max_observed_concurrency={} results={:?}",
        outcome.max_observed_concurrency,
        outcome
            .results
            .iter()
            .map(|result| (result.status.as_str(), result.output.as_str()))
            .collect::<Vec<_>>()
    );

    // Assert 1：结果数量与 calls 一一对应
    assert_eq!(
        outcome.results.len(),
        calls.len(),
        "t1 失败：results.len()={} 应等于 calls.len()={}",
        outcome.results.len(),
        calls.len()
    );

    // Assert 2：保序（完成顺序是 3,2,1,0，返回顺序必须仍是 0,1,2,3）
    for (index, result) in outcome.results.iter().enumerate() {
        assert_eq!(
            result.status, "ok",
            "t1 失败：results[{index}]（calls[{index}]={}）status 应为 \"ok\"，实际 status=\"{}\"，output={}",
            calls[index].name, result.status, result.output
        );
        assert_eq!(
            result.output,
            format!("executed:{index}"),
            "t1 失败：保序被破坏 —— results[{index}] 的 output 应来自 calls[{index}]（期望 \"executed:{index}\"），\
             实际 \"{}\"。完成顺序应与返回顺序无关（串行也须保序，并发更须保序）。",
            result.output
        );
        assert_eq!(
            result.tool_name, calls[index].name,
            "t1 失败：results[{index}] 的 tool_name 应回显 calls[{index}].name=\"{}\"，实际=\"{}\"",
            calls[index].name, result.tool_name
        );
    }

    // Assert 3：真并发（4 路峰值 400ms；串行需 ~1000ms，故 700ms 阈值仍能区分串行）
    assert!(
        outcome.max_observed_concurrency >= 3 && outcome.max_observed_concurrency <= 4,
        "t1 失败：4 个安全调用在 max_concurrency=4 下峰值并发应为 3~4，实际={}（1 表示退化为串行）",
        outcome.max_observed_concurrency
    );
    assert!(
        elapsed < Duration::from_millis(700),
        "t1 失败：4 路并发最慢一路 400ms、串行约 1000ms，总耗时应明显小于 700ms，\
         实际耗时 {elapsed:?}（疑似串行执行）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t2：不安全调用必须串行且独占（峰值并发 == 1）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t2_unsafe_calls_run_serially_without_overlap() {
    // Arrange：3 个不安全调用（Write），各 60ms
    let calls: Vec<ToolCall> = (0..3).map(|index| make_call(index, "Write")).collect();
    let concurrent_safe = vec![false; calls.len()];
    let policy = policy(4);
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let overlap = AtomicBool::new(false);

    // Act
    let started = Instant::now();
    let outcome = execute_calls(&calls, &concurrent_safe, &policy, |call| {
        let index = index_of(call);
        let previous = active.fetch_add(1, Ordering::SeqCst);
        if previous != 0 {
            overlap.store(true, Ordering::SeqCst);
        }
        peak.fetch_max(previous + 1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
        active.fetch_sub(1, Ordering::SeqCst);
        ok_result(call, index)
    });
    let elapsed = started.elapsed();
    let outcome = expect_outcome(outcome, "t2 3 个不安全调用串行执行");
    println!(
        "[W28][t2] elapsed={elapsed:?} max_observed_concurrency={} observed_peak={} results={:?}",
        outcome.max_observed_concurrency,
        peak.load(Ordering::SeqCst),
        outcome.results.iter().map(|result| result.output.as_str()).collect::<Vec<_>>()
    );

    // Assert 1：结果保序且全部完成
    assert_eq!(
        outcome.results.len(),
        3,
        "t2 失败：results.len() 应为 3，实际 {}",
        outcome.results.len()
    );
    for (index, result) in outcome.results.iter().enumerate() {
        assert_eq!(
            result.output,
            format!("executed:{index}"),
            "t2 失败：results[{index}] 应为 \"executed:{index}\"，实际 \"{}\"",
            result.output
        );
        assert_eq!(
            result.status, "ok",
            "t2 失败：results[{index}].status 应为 \"ok\"，实际 \"{}\"",
            result.status
        );
    }

    // Assert 2：峰值并发必须为 1（不安全工具独占执行）
    assert_eq!(
        outcome.max_observed_concurrency, 1,
        "t2 失败：全部为不安全调用时峰值并发应为 1，实际={}（说明 unsafe 调用被并发执行）",
        outcome.max_observed_concurrency
    );
    assert!(
        !overlap.load(Ordering::SeqCst),
        "t2 失败：检测到 unsafe 调用执行期间有其它调用同时处于执行中（重叠违反独占语义）"
    );
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "t2 失败：executor 实测峰值并发应为 1，实际={}",
        peak.load(Ordering::SeqCst)
    );

    // Assert 3：串行耗时下界（3×60ms=180ms；>=150ms 留余量，防止「假串行」零耗时通过）
    assert!(
        elapsed >= Duration::from_millis(150),
        "t2 失败：3 个 60ms 的 unsafe 调用串行耗时至少 ~180ms，实际 {elapsed:?}（疑似未真正串行/未 sleep）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t3：混合负载 —— 安全调用可并发、不安全调用独占，全序正确
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t3_mixed_safe_and_unsafe_calls_respect_isolation_and_order() {
    // Arrange：safe×2 + unsafe×1 + safe×2（unsafe 位于 index 2）
    let calls: Vec<ToolCall> = (0..5)
        .map(|index| {
            let name = if index == 2 { "Write" } else { "Read" };
            make_call(index, name)
        })
        .collect();
    let concurrent_safe = vec![true, true, false, true, true];
    let policy = policy(4);
    let active = AtomicUsize::new(0);
    let unsafe_active = AtomicUsize::new(0);
    let violation = AtomicBool::new(false);

    // Act
    let outcome = execute_calls(&calls, &concurrent_safe, &policy, |call| {
        let index = index_of(call);
        let is_unsafe = index == 2;
        let previous_active = active.fetch_add(1, Ordering::SeqCst);
        if is_unsafe {
            unsafe_active.fetch_add(1, Ordering::SeqCst);
            // unsafe 进入时不得有任何他人正在执行
            if previous_active != 0 {
                violation.store(true, Ordering::SeqCst);
            }
        } else if unsafe_active.load(Ordering::SeqCst) != 0 {
            // safe 进入时不得与 unsafe 重叠
            violation.store(true, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(60));
        if is_unsafe {
            unsafe_active.fetch_sub(1, Ordering::SeqCst);
        }
        active.fetch_sub(1, Ordering::SeqCst);
        ok_result(call, index)
    });
    let outcome = expect_outcome(outcome, "t3 混合（safe/unsafe）调用执行");
    println!(
        "[W28][t3] max_observed_concurrency={} violation={} results={:?}",
        outcome.max_observed_concurrency,
        violation.load(Ordering::SeqCst),
        outcome.results.iter().map(|result| result.output.as_str()).collect::<Vec<_>>()
    );

    // Assert 1：全序正确
    assert_eq!(
        outcome.results.len(),
        5,
        "t3 失败：results.len() 应为 5，实际 {}",
        outcome.results.len()
    );
    for (index, result) in outcome.results.iter().enumerate() {
        assert_eq!(
            result.status, "ok",
            "t3 失败：results[{index}].status 应为 \"ok\"，实际 \"{}\"，output={}",
            result.status, result.output
        );
        assert_eq!(
            result.output,
            format!("executed:{index}"),
            "t3 失败：保序被破坏 —— results[{index}] 应为 \"executed:{index}\"，实际 \"{}\"",
            result.output
        );
    }

    // Assert 2：峰值并发不超过策略上限
    assert!(
        outcome.max_observed_concurrency <= policy.max_concurrency,
        "t3 失败：峰值并发 {} 超过 policy.max_concurrency={}",
        outcome.max_observed_concurrency,
        policy.max_concurrency
    );

    // Assert 3：unsafe 独占（执行期间不得有任何其它调用并发）
    assert!(
        !violation.load(Ordering::SeqCst),
        "t3 失败：混合负载下检测到 unsafe 调用与其它调用重叠执行（违反「unsafe 必须独占」语义）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t4：panic 隔离 —— 单个调用 panic 不拖垮整体
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t4_executor_panic_is_isolated_into_tool_panicked_error() {
    // Arrange：3 个调用，第 2 个（index 1）panic
    let calls: Vec<ToolCall> = (0..3).map(|index| make_call(index, "Read")).collect();
    let concurrent_safe = vec![true; calls.len()];
    let policy = policy(4);

    // Act
    let started = Instant::now();
    let outcome = execute_calls(&calls, &concurrent_safe, &policy, |call| {
        let index = index_of(call);
        if index == 1 {
            panic!("boom");
        }
        ok_result(call, index)
    });
    let elapsed = started.elapsed();
    let outcome = expect_outcome(outcome, "t4 panic 隔离（panicking 调用的错误应体现在 results 内）");
    println!(
        "[W28][t4] elapsed={elapsed:?} max_observed_concurrency={} results={:?}",
        outcome.max_observed_concurrency,
        outcome
            .results
            .iter()
            .map(|result| (result.status.as_str(), result.output.as_str()))
            .collect::<Vec<_>>()
    );

    // Assert 1：整体在 5s 内返回（不挂死）
    assert!(
        elapsed < Duration::from_secs(5),
        "t4 失败：executor panic 后整体应在 5s 内返回，实际耗时 {elapsed:?}"
    );

    // Assert 2：数量与结构完整
    assert_eq!(
        outcome.results.len(),
        3,
        "t4 失败：panic 隔离后 results.len() 应为 3，实际 {}（整体不应中断）",
        outcome.results.len()
    );

    // Assert 3：panic 的调用映射为 tool_panicked 错误
    let panicked = &outcome.results[1];
    assert_eq!(
        panicked.status, "error",
        "t4 失败：results[1].status 应为 \"error\"（panic 被隔离），实际 \"{}\"，output={}",
        panicked.status, panicked.output
    );
    assert_eq!(
        error_code_of(&panicked.output).as_deref(),
        Some("tool_panicked"),
        "t4 失败：results[1].output 的 error.code 应为 \"tool_panicked\"，实际 output={}",
        panicked.output
    );

    // Assert 4：其余调用正常完成且保序
    for index in [0usize, 2] {
        let result = &outcome.results[index];
        assert_eq!(
            result.status, "ok",
            "t4 失败：results[{index}]（非 panic 调用）应正常完成，实际 status=\"{}\"，output={}",
            result.status, result.output
        );
        assert_eq!(
            result.output,
            format!("executed:{index}"),
            "t4 失败：results[{index}] 应为 \"executed:{index}\"，实际 \"{}\"",
            result.output
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// t5：空输入 与 concurrent_safe 长度不匹配
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t5_empty_input_and_length_mismatch_contract() {
    // Arrange
    let empty_calls: Vec<ToolCall> = Vec::new();
    let empty_flags: Vec<bool> = Vec::new();

    // Act 1：空输入
    let empty_outcome = execute_calls(&empty_calls, &empty_flags, &policy(4), |call| {
        ok_result(call, index_of(call))
    });
    let empty_outcome = expect_outcome(empty_outcome, "t5 空输入");

    // Assert 1
    assert!(
        empty_outcome.results.is_empty(),
        "t5 失败：空输入时 results 应为空，实际 {} 项",
        empty_outcome.results.len()
    );
    assert_eq!(
        empty_outcome.max_observed_concurrency, 0,
        "t5 失败：空输入时 max_observed_concurrency 应为 0，实际 {}",
        empty_outcome.max_observed_concurrency
    );

    // Act 2：长度不匹配（flags 比 calls 短、比 calls 长）
    let calls: Vec<ToolCall> = (0..2).map(|index| make_call(index, "Read")).collect();
    let too_short = vec![true];
    let too_long = vec![true, true, true];
    let short_outcome = execute_calls(&calls, &too_short, &policy(4), |call| {
        ok_result(call, index_of(call))
    });
    let long_outcome = execute_calls(&calls, &too_long, &policy(4), |call| {
        ok_result(call, index_of(call))
    });

    // Assert 2：两个方向都必须 Err(String)
    assert!(
        short_outcome.is_err(),
        "t5 失败：concurrent_safe 长度不足（1 vs calls 2）必须返回 Err，实际 Ok（max_observed_concurrency={}）",
        short_outcome.as_ref().map(|outcome| outcome.max_observed_concurrency).unwrap_or(0)
    );
    assert!(
        long_outcome.is_err(),
        "t5 失败：concurrent_safe 长度超出（3 vs calls 2）必须返回 Err，实际 Ok（max_observed_concurrency={}）",
        long_outcome.as_ref().map(|outcome| outcome.max_observed_concurrency).unwrap_or(0)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t6：并发上限不得被突破
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t6_concurrency_is_capped_by_policy_max_concurrency() {
    // Arrange：6 个安全调用、max_concurrency=2、每个 80ms（串行 480ms，2 路约 240ms）
    let calls: Vec<ToolCall> = (0..6).map(|index| make_call(index, "Search")).collect();
    let concurrent_safe = vec![true; calls.len()];
    let policy = policy(2);
    let active = AtomicUsize::new(0);
    let observed_peak = AtomicUsize::new(0);

    // Act
    let started = Instant::now();
    let outcome = execute_calls(&calls, &concurrent_safe, &policy, |call| {
        let index = index_of(call);
        let previous = active.fetch_add(1, Ordering::SeqCst);
        observed_peak.fetch_max(previous + 1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(80));
        active.fetch_sub(1, Ordering::SeqCst);
        ok_result(call, index)
    });
    let elapsed = started.elapsed();
    let outcome = expect_outcome(outcome, "t6 并发上限（max_concurrency=2）");
    println!(
        "[W28][t6] elapsed={elapsed:?} max_observed_concurrency={} observed_peak={}",
        outcome.max_observed_concurrency,
        observed_peak.load(Ordering::SeqCst)
    );

    // Assert 1：结果完整且保序
    assert_eq!(
        outcome.results.len(),
        6,
        "t6 失败：results.len() 应为 6，实际 {}",
        outcome.results.len()
    );
    for (index, result) in outcome.results.iter().enumerate() {
        assert_eq!(
            result.output,
            format!("executed:{index}"),
            "t6 失败：results[{index}] 应为 \"executed:{index}\"，实际 \"{}\"",
            result.output
        );
    }

    // Assert 2：峰值不得突破上限
    assert!(
        outcome.max_observed_concurrency <= policy.max_concurrency,
        "t6 失败：峰值并发 {} 超过 policy.max_concurrency={}（上限被突破）",
        outcome.max_observed_concurrency,
        policy.max_concurrency
    );
    assert!(
        observed_peak.load(Ordering::SeqCst) <= policy.max_concurrency,
        "t6 失败：executor 侧实测峰值并发 {} 超过 policy.max_concurrency={}",
        observed_peak.load(Ordering::SeqCst),
        policy.max_concurrency
    );

    // Assert 3：耗时 < 6×80ms（串行下界 480ms；并行上限 2 时约 240ms）
    assert!(
        elapsed < Duration::from_millis(480),
        "t6 失败：max_concurrency=2 下 6 个 80ms 调用应约 240ms 完成（串行 480ms），实际 {elapsed:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t7：工具并发安全属性
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t7_tool_concurrent_safety_flags() {
    let safe_tools = ["Read", "workspace_read_file", "Search"];
    let unsafe_tools = ["Write", "workspace_write_file", "Run"];

    let mut violations: Vec<String> = Vec::new();

    for name in safe_tools {
        if !tool_is_concurrent_safe(name) {
            violations.push(format!(
                "`{name}`: tool_is_concurrent_safe=false（期望 true：只读工具可并发）"
            ));
        }
    }
    for name in unsafe_tools {
        if tool_is_concurrent_safe(name) {
            violations.push(format!(
                "`{name}`: tool_is_concurrent_safe=true（期望 false：写入/命令执行必须独占串行）"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "t7 失败：{} 项工具并发安全属性与 W28 冻结清单不一致。\n\
         期望 true：{safe_tools:?}\n期望 false：{unsafe_tools:?}\n实际违规：\n- {}",
        violations.len(),
        violations.join("\n- ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t8：策略默认值（契约冻结 max_concurrency = 4）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t8_default_policy_max_concurrency_is_four() {
    let default_policy = ToolCallExecutionPolicy::default();
    assert_eq!(
        default_policy.max_concurrency, 4,
        "t8 失败：ToolCallExecutionPolicy::default().max_concurrency 应为 4，实际 {}",
        default_policy.max_concurrency
    );
}

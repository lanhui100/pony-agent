//! 并发工具执行引擎（W28）。
//!
//! 背景：审计（`docs/audit/agent-core-gap-matrix.md` P0-1 / A01）指出回合引擎在 provider 层
//! 硬拒多工具调用（Anthropic 多 `tool_use` 直接 `Err`，OpenAI 静默丢弃多余调用），因此
//! 「同一轮并行读 5 个文件」这类高频范式只能退化成多轮往返。本模块提供可复用的并发执行底座：
//!
//! - 按 descriptor 的 `concurrent_safe` 属性分区：只读安全工具并发，其余**严格串行**；
//! - **保序聚合**：`results[i]` 永远对应 `calls[i]`，与完成先后无关；
//! - **panic 隔离**：单个调用的 panic 不会带崩整批，映射为 `tool_panicked` 错误结果；
//! - 并发上限可配（`max_concurrency`，`0` 视为 1）。
//!
//! 本波只交付引擎与工具并发属性；把 provider 的多调用接入本引擎需要改造回合循环
//! （`turn_sync`/`turn_stream` 的单 hop 结构），已单列为计划 W28b，见
//! `docs/audit/agent-core-improvement-plan.md`。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde_json::json;

use crate::agent::tools::{ToolCall, ToolResult};

/// 进程级并发硬上限：即使调用方传入更大值也不超过该上限，避免线程/句柄耗尽。
pub const MAX_CONCURRENCY_HARD_CAP: usize = 32;

/// 并发执行策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallExecutionPolicy {
    /// 允许的最大并发数；`0` 等价于 1（串行）。
    pub max_concurrency: usize,
}

impl Default for ToolCallExecutionPolicy {
    fn default() -> Self {
        Self { max_concurrency: 4 }
    }
}

impl ToolCallExecutionPolicy {
    /// 生效的并发上限（至少 1，且不超过 [`MAX_CONCURRENCY_HARD_CAP`]）。
    pub fn effective_concurrency(&self) -> usize {
        self.max_concurrency.clamp(1, MAX_CONCURRENCY_HARD_CAP)
    }
}

/// 一批调用的执行结果。
#[derive(Debug, Clone)]
pub struct ParallelToolOutcome {
    /// 与入参 `calls` 一一对应、保序的结果。
    pub results: Vec<ToolResult>,
    /// 实测峰值并发数（供断言与遥测）。
    pub max_observed_concurrency: usize,
}

/// 按并发安全属性执行一批工具调用。
///
/// 语义（契约）：
/// 1. `results.len() == calls.len()` 且保序；
/// 2. `concurrent_safe[i] == true` 的调用并发执行（上限 `policy.effective_concurrency()`），
///    `false` 的调用在并发阶段**全部完成后**串行执行——任何时刻都不会与其它调用重叠；
/// 3. executor panic 被隔离为该调用的 `tool_panicked` 错误结果；
/// 4. 空输入返回空结果、峰值并发 0；
/// 5. `concurrent_safe` 与 `calls` 长度不一致返回 `Err`。
pub fn execute_calls<F>(
    calls: &[ToolCall],
    concurrent_safe: &[bool],
    policy: &ToolCallExecutionPolicy,
    executor: F,
) -> Result<ParallelToolOutcome, String>
where
    F: Fn(&ToolCall) -> ToolResult + Sync,
{
    if calls.len() != concurrent_safe.len() {
        return Err(format!(
            "并发执行入参不一致：calls={} concurrent_safe={}。",
            calls.len(),
            concurrent_safe.len()
        ));
    }
    if calls.is_empty() {
        return Ok(ParallelToolOutcome {
            results: Vec::new(),
            max_observed_concurrency: 0,
        });
    }

    let results: Mutex<Vec<Option<ToolResult>>> = Mutex::new(vec![None; calls.len()]);
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let executor = &executor;

    let run_one = |index: usize| {
        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(current, Ordering::SeqCst);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            executor(&calls[index])
        }));
        active.fetch_sub(1, Ordering::SeqCst);
        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                panic_result(&calls[index], &detail)
            }
        };
        // 毒锁恢复：即便某个 executor 在持锁边缘 panic 导致锁中毒，也必须写回结果，
        // 不允许静默丢弃已完成调用（`into_inner` 取回底层数据）。
        let mut slot = results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slot[index] = Some(result);
    };

    // 阶段 1：并发安全调用，固定大小工作池拉取索引（避免 chunk 窗口的队头阻塞）。
    let safe_indices: Vec<usize> = (0..calls.len())
        .filter(|index| concurrent_safe[*index])
        .collect();
    let limit = policy.effective_concurrency();
    if !safe_indices.is_empty() {
        let workers = limit.min(safe_indices.len());
        let cursor = AtomicUsize::new(0);
        let next_index = || {
            let position = cursor.fetch_add(1, Ordering::SeqCst);
            safe_indices.get(position).copied()
        };
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let run_one = &run_one;
                let next_index = &next_index;
                scope.spawn(move || {
                    while let Some(index) = next_index() {
                        run_one(index);
                    }
                });
            }
        });
    }

    // 阶段 2：非并发安全调用严格串行（不与任何 safe 调用重叠）。
    for (index, is_safe) in concurrent_safe.iter().enumerate() {
        if !is_safe {
            run_one(index);
        }
    }

    let results = results
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            slot.unwrap_or_else(|| panic_result(&calls[index], "执行器未返回结果"))
        })
        .collect();

    Ok(ParallelToolOutcome {
        results,
        max_observed_concurrency: peak.load(Ordering::SeqCst),
    })
}

/// 把 `ToolRouter` 作为执行器，按工具名解析并发安全属性后执行一批调用。
pub fn execute_calls_with_router(
    router: &crate::agent::tools::ToolRouter,
    calls: &[ToolCall],
    policy: &ToolCallExecutionPolicy,
) -> Result<ParallelToolOutcome, String> {
    let safe: Vec<bool> = calls
        .iter()
        .map(|call| crate::agent::tools::tool_is_concurrent_safe(&call.name))
        .collect();
    execute_calls(calls, &safe, policy, |call| router.execute(call))
}

fn panic_result(call: &ToolCall, detail: &str) -> ToolResult {
    ToolResult {
        tool_name: call.name.clone(),
        status: "error".to_string(),
        output: json!({
            "ok": false,
            "tool": call.name,
            "error": {
                "code": "tool_panicked",
                "kind": "tool_panicked",
                "message": format!("工具 `{}` 执行时 panic：{detail}", call.name),
                "hint": "该调用已隔离，其余调用不受影响；请检查工具实现或改用等价工具。"
            }
        })
        .to_string(),
        duration_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            call_id: None,
            name: name.to_string(),
            arguments: json!({}),
            plan: None,
        }
    }

    fn ok_result(name: &str) -> ToolResult {
        ToolResult {
            tool_name: name.to_string(),
            status: "ok".to_string(),
            output: "{}".to_string(),
            duration_ms: 0,
        }
    }

    #[test]
    fn empty_input_yields_empty_outcome() {
        let outcome = execute_calls(&[], &[], &ToolCallExecutionPolicy::default(), |c| {
            ok_result(&c.name)
        })
        .expect("empty batch");
        assert!(outcome.results.is_empty());
        assert_eq!(outcome.max_observed_concurrency, 0);
    }

    #[test]
    fn length_mismatch_is_rejected() {
        let err = execute_calls(&[call("Read")], &[], &ToolCallExecutionPolicy::default(), |c| {
            ok_result(&c.name)
        })
        .expect_err("mismatch must be rejected");
        assert!(err.contains("不一致"), "{err}");
    }

    #[test]
    fn results_preserve_input_order() {
        let calls = vec![call("a"), call("b"), call("c")];
        let outcome = execute_calls(
            &calls,
            &[true, true, true],
            &ToolCallExecutionPolicy { max_concurrency: 3 },
            |c| {
                let mut result = ok_result(&c.name);
                result.output = json!({ "name": c.name }).to_string();
                result
            },
        )
        .expect("batch");
        let names: Vec<String> = outcome
            .results
            .iter()
            .map(|r| r.tool_name.clone())
            .collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn unsafe_calls_never_overlap() {
        let calls = vec![call("w1"), call("w2"), call("w3")];
        let outcome = execute_calls(
            &calls,
            &[false, false, false],
            &ToolCallExecutionPolicy::default(),
            |c| ok_result(&c.name),
        )
        .expect("batch");
        assert_eq!(outcome.max_observed_concurrency, 1);
    }

    #[test]
    fn concurrency_safety_resolves_aliases_and_defaults_unknown_to_serial() {
        use crate::agent::tools::tool_is_concurrent_safe;
        // 模型可见名（未进 canonical 表）必须与 descriptor 策略同源
        assert!(tool_is_concurrent_safe("ReadDocument"));
        assert!(tool_is_concurrent_safe("Read"));
        assert!(tool_is_concurrent_safe("workspace_read_file"));
        assert!(tool_is_concurrent_safe("Search"));
        // 写/执行类必须串行
        assert!(!tool_is_concurrent_safe("Write"));
        assert!(!tool_is_concurrent_safe("workspace_write_file"));
        assert!(!tool_is_concurrent_safe("Run"));
        // 未知/动态工具：默认串行（宁可慢不可并发误判）
        assert!(!tool_is_concurrent_safe("mcp__demo__echo"));
    }

    #[test]
    fn panics_are_isolated_per_call() {
        let calls = vec![call("ok1"), call("boom"), call("ok2")];
        let outcome = execute_calls(
            &calls,
            &[true, true, true],
            &ToolCallExecutionPolicy::default(),
            |c| {
                if c.name == "boom" {
                    panic!("boom");
                }
                ok_result(&c.name)
            },
        )
        .expect("batch");
        assert_eq!(outcome.results[0].status, "ok");
        assert_eq!(outcome.results[2].status, "ok");
        assert_eq!(outcome.results[1].status, "error");
        assert!(outcome.results[1].output.contains("tool_panicked"));
    }
}

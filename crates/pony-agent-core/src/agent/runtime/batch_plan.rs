//! D4（W28b / B018 后续）：批内执行段的**并发规划**与执行 —— 把 `parallel_tools` 引擎接到
//! `turn_sync.rs` / `turn_stream.rs` 的批执行段。
//!
//! 边界（设计 `multi-tool-call-execution.md` §3 / §7）：
//! - **开关默认关**（`PONY_AGENT_PARALLEL_TOOLS`，见 `limits::parallel_tools_enabled`）。
//!   关 ⇒ 规划出的每个段长恒为 1，批内仍是「预阶段 → 执行 → 后阶段」逐调用交织，与本波之前
//!   逐字一致；
//! - 开 ⇒ 批内**连续**的并发安全调用（`tool_is_concurrent_safe` 且非交互类）且该段 ≥2 个时，
//!   该段整体交给 [`parallel_tools::execute_calls`]；`Run` / `Write` / 未知工具（默认串行）与
//!   交互类工具（批终止符）**永不**进并发段，仍按逐调用顺序执行；
//! - 结果始终按批内**原序**回填（引擎本身保序，这里再按索引回收执行记录）；
//! - per-call 语义不因并发而丢：hop 记账 / 工具事件 / 熔断闸 / OTel span 都在预阶段与后阶段
//!   逐调用执行，执行阶段只负责「跑工具并回收三元组」。
//!
//! 为什么不把整批都丢给引擎：`execute_calls` 的串行阶段会把非安全调用挤到并发阶段之后，
//! 且一次性吞掉全部执行 —— 交互类工具（Ask）必须能在批内任意位置**立即**停批，逐调用的
//! 熔断/取消检查也必须能随时终止本轮。故只有「连续可并发段」进引擎，其余仍逐调用。

use super::*;

/// 一次工具调用的执行产物（与批内位置一一对应，供后阶段逐调用回填）。
pub(super) struct ToolCallOutcome {
    pub(super) result: ToolResult,
    pub(super) invocation_record: crate::agent::telemetry::CapabilityInvocationRecord,
    pub(super) capability_hook_trace_records: Vec<HookTraceRecord>,
}

/// 批内执行段：`calls` 是批内下标区间；`parallel` 为真表示整段交并发引擎执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BatchSegment {
    pub(super) calls: std::ops::Range<usize>,
    pub(super) parallel: bool,
}

/// 规划批内执行段。
///
/// - 开关关 ⇒ 每个调用一个串行段（逐调用交织，行为与本波之前逐字一致）；
/// - 开关开 ⇒ 连续的并发安全调用聚成一段：≥2 个走并发段，孤立的安全调用仍按串行段；
///   非并发安全 / 交互类调用各自成串行段（不与任何调用重叠）。
pub(super) fn plan_batch_segments(calls: &[ToolCall]) -> Vec<BatchSegment> {
    let parallel_enabled = parallel_tools_enabled();
    let mut segments: Vec<BatchSegment> = Vec::new();
    let mut index = 0usize;
    while index < calls.len() {
        if parallel_enabled && call_may_run_concurrently(&calls[index]) {
            let start = index;
            while index < calls.len() && call_may_run_concurrently(&calls[index]) {
                index += 1;
            }
            if index - start >= 2 {
                segments.push(BatchSegment {
                    calls: start..index,
                    parallel: true,
                });
                continue;
            }
            // 孤立的安全调用：按串行段执行（并发无收益，且保持逐调用交织语义）。
            index = start + 1;
            segments.push(BatchSegment {
                calls: start..start + 1,
                parallel: false,
            });
            continue;
        }
        segments.push(BatchSegment {
            calls: index..index + 1,
            parallel: false,
        });
        index += 1;
    }
    segments
}

/// 该调用是否**允许**进并发段：并发安全属性为真，且不是交互类（批终止符）。
fn call_may_run_concurrently(call: &ToolCall) -> bool {
    if crate::agent::tools::tool_kind_for_name(&call.name)
        == crate::agent::tools::ToolKind::Interactive
    {
        return false;
    }
    crate::agent::tools::tool_is_concurrent_safe(&call.name)
}

/// 用一个并发安全段走 `parallel_tools::execute_calls`，返回**按段内原序**排列的执行产物。
///
/// 三元组回收用 `Mutex<Vec<(usize, ToolCallOutcome)>>`：引擎的 `executor` 闭包只拿到 `&ToolCall`，
/// 这里用指针同一性把回调映射回段内下标（`execute_calls` 传入的正是 `calls[index]` 的引用），
/// 回收后按下标排序展开 —— 绝不依赖完成先后。
pub(super) fn execute_segment_in_parallel<F>(
    calls: &[ToolCall],
    exec_one: F,
) -> Result<Vec<ToolCallOutcome>, String>
where
    F: Fn(&ToolCall) -> ToolCallOutcome + Sync,
{
    if calls.is_empty() {
        return Ok(Vec::new());
    }
    let concurrent_safe = vec![true; calls.len()];
    // 段内并发上限：段长与 8 的较小值（引擎内部还会再夹到 HARDCAP 32）。取 8 而不是引擎默认 4，
    // 是因为「一次并行读 3~5 个文件」是常见形态；也不取段长，避免一个超长批瞬间拉起几十个线程。
    let policy = crate::agent::parallel_tools::ToolCallExecutionPolicy {
        max_concurrency: calls.len().min(8),
    };
    let collected: std::sync::Mutex<Vec<(usize, ToolCallOutcome)>> =
        std::sync::Mutex::new(Vec::with_capacity(calls.len()));
    let outcome = crate::agent::parallel_tools::execute_calls(
        calls,
        &concurrent_safe,
        &policy,
        |call| {
            let index = calls
                .iter()
                .position(|candidate| std::ptr::eq(candidate, call))
                .unwrap_or(0);
            let one = exec_one(call);
            let result = one.result.clone();
            collected
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((index, one));
            result
        },
    )?;
    runtime_log(format!(
        "turn:tool-parallel calls={} peak_concurrency={}",
        calls.len(),
        outcome.max_observed_concurrency
    ));
    let mut slots: Vec<Option<ToolCallOutcome>> = (0..calls.len()).map(|_| None).collect();
    let mut collected = collected
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    collected.sort_by_key(|(index, _)| *index);
    for (index, one) in collected {
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(one);
        }
    }
    Ok(slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| match slot {
            Some(one) => one,
            // panic 隔离：引擎捕获了该调用的 panic（executor 未返回）⇒ 用引擎给出的
            // `tool_panicked` 结果补位，执行记录退回同源的最小形态（只有工具名与默认权限事实）。
            None => ToolCallOutcome {
                result: outcome.results[index].clone(),
                invocation_record: fallback_invocation_record(&calls[index]),
                capability_hook_trace_records: Vec::new(),
            },
        })
        .collect())
}

/// panic 隔离路径的最小执行记录：与 `execute_registered_tool_call_with_context` 的非技能分支
/// 同源（工具名 + 默认权限事实），其余能力/来源字段为空。
pub(super) fn fallback_invocation_record(
    call: &ToolCall,
) -> crate::agent::telemetry::CapabilityInvocationRecord {
    crate::agent::telemetry::CapabilityInvocationRecord {
        tool_name: call.name.clone(),
        capability_id: None,
        source_id: None,
        source_kind: None,
        capability_kind: None,
        invocation_mode: None,
        failure_kind: None,
        requires_approval: None,
        host_mediated: None,
        permission_scope: None,
        permission_facts: Some(default_permission_facts_for_name(&call.name)),
        skill_id: None,
        skill_source_id: None,
        composed_capability_refs: None,
        composed_capability_kinds: None,
        failure_layer: None,
    }
}

impl AgentRuntime {
    /// 逐调用执行入口（批内串行段与并发段的执行体都走它）：治理/沙箱/预算路径与单调用完全一致。
    ///
    /// 上下文构造从 `turn_sync` / `turn_stream` 的两份内联代码收敛到这里：两处原本分别是
    /// `session_id.map(str::to_string)` 与 `input.session_id.clone()`，等价于同一个
    /// `Option<&str>` 入参 —— 行为逐字不变。
    pub(super) fn execute_tool_call_with_turn_context(
        &self,
        session_id: Option<&str>,
        governed_context: Option<&DispatchContext>,
        tool_call: &ToolCall,
    ) -> ToolCallOutcome {
        let execution_context = ToolExecutionContext {
            workspace_root: self.resolve_session_workspace_root(session_id),
            // L3 r2 A1：透传真实会话 id，触达注入按会话分桶（否则跨会话互相抑制）。
            session_id: session_id.map(str::to_string),
            // T2-1：turn 级治理事实随本次调用显式传入（并发 turn 之间不共享任何槽位）。
            dispatch_context: governed_context.cloned(),
            // wave-62 C3：治理事实里若带本轮令牌（stream 路径注入），工具层据此可被取消。
            cancellation: governed_context.and_then(|context| context.turn_cancellation.clone()),
            ..Default::default()
        };
        let (result, invocation_record, capability_hook_trace_records) =
            self.execute_registered_tool_call_with_context(tool_call, &execution_context);
        ToolCallOutcome {
            result,
            invocation_record,
            capability_hook_trace_records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            call_id: None,
            name: name.to_string(),
            arguments: json!({}),
            plan: None,
        }
    }

    /// 开关**关**：每个调用各成一段串行段（顺序语义与批执行逐字一致）。
    #[test]
    fn switch_off_plans_one_serial_segment_per_call() {
        let previous = set_parallel_tools_override(Some(false));
        let calls = vec![call("Read"), call("List"), call("Run")];
        let segments = plan_batch_segments(&calls);
        set_parallel_tools_override(previous);
        assert_eq!(
            segments,
            vec![
                BatchSegment { calls: 0..1, parallel: false },
                BatchSegment { calls: 1..2, parallel: false },
                BatchSegment { calls: 2..3, parallel: false },
            ]
        );
    }

    /// 开关**开**：连续的安全调用聚成并发段；`Run`（非安全）单独成串行段；孤立安全调用不并发。
    #[test]
    fn switch_on_plans_parallel_runs_and_keeps_serial_calls_apart() {
        let previous = set_parallel_tools_override(Some(true));
        let calls = vec![
            call("Read"),
            call("List"),
            call("Run"),
            call("Read"),
            call("Ask"),
            call("Read"),
        ];
        let segments = plan_batch_segments(&calls);
        set_parallel_tools_override(previous);
        assert_eq!(
            segments,
            vec![
                BatchSegment { calls: 0..2, parallel: true },
                BatchSegment { calls: 2..3, parallel: false },
                // 孤立安全调用不并发（段长 1）。
                BatchSegment { calls: 3..4, parallel: false },
                // 交互类工具是批终止符，永不进并发段。
                BatchSegment { calls: 4..5, parallel: false },
                BatchSegment { calls: 5..6, parallel: false },
            ]
        );
    }

    /// 开关取值解析：只有 `1/true/on/yes` 开，其余（含拼写错误）一律关。
    #[test]
    fn parallel_tools_flag_parsing_is_fail_safe() {
        for raw in ["1", "true", "TRUE", "on", " Yes "] {
            assert!(parse_parallel_tools_enabled(Some(raw)), "{raw} 必须开");
        }
        for raw in ["", "0", "false", "off", "no", "2", "enable", "tru"] {
            assert!(!parse_parallel_tools_enabled(Some(raw)), "{raw} 必须关");
        }
        assert!(!parse_parallel_tools_enabled(None), "未设置必须关");
    }
}

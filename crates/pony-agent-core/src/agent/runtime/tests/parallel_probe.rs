//! 并发接线用例的共享探针（自 `runtime/tests.rs` 外移，纯搬迁，语义不变）。
//!
//! 包含：`PONY_AGENT_PARALLEL_TOOLS` 用例级覆写守卫、有界集合点探针执行器与两档时间预算。

use super::*;

/// 并发探针的**有界集合点**预算：进入后等「峰值达到期望值」或预算耗尽。
///
/// 取代「固定 `sleep(60ms)` 制造重叠窗口」：固定窗口下「并发是否被观测到」取决于负载下的
/// 线程调度抖动（本机全量并行跑时偶红——第二路线程在 60ms 窗口内没被调度到，`peak` 就停在 1）；
/// 集合点只依赖「并发是否真的发生」，并发成立时毫秒级返回，不成立时预算耗尽后照常返回。
pub(super) const PARALLEL_PROBE_RENDEZVOUS_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

/// 串行负控的**观测窗口**：给「可能存在的并发对端」一段有界机会；窗口耗尽仍未出现第二路，
/// 即构成本用例的串行证据（有界、可复算，不是用固定 sleep 伪装出来的「看起来串行」）。
pub(super) const PARALLEL_PROBE_SERIAL_WINDOW: std::time::Duration = std::time::Duration::from_millis(200);

/// 用例级开关覆写守卫：panic 也能还原（并行跑的其他用例不受影响）。
pub(super) struct ParallelToolsFlag(Option<bool>);

impl Drop for ParallelToolsFlag {
    fn drop(&mut self) {
        set_parallel_tools_override(self.0);
    }
}

pub(super) fn with_parallel_tools(enabled: bool) -> ParallelToolsFlag {
    ParallelToolsFlag(set_parallel_tools_override(Some(enabled)))
}

/// 并发探针执行器：记录每次调用**进入时**的在途并发数，并累计实测峰值。
///
/// - `entries`：`(工具名, 进入时在途并发数)`，按真实进入顺序（并发下非确定，只作并发证据）；
/// - `peak`：实测峰值并发 —— `>= 2` 才证明并发**确实发生**（不是「两个都执行了」）；
/// - `expected_peak` + `wait_budget`：**有界集合点**（见 [`PARALLEL_PROBE_RENDEZVOUS_BUDGET`]）。
///   进入后最多等 `wait_budget`，直到实测峰值达到 `expected_peak`；并发成立时集合点在毫秒级
///   返回（与调度抖动无关），串行执行时窗口耗尽后照常返回且 `peak` 保持 1；
/// - `ask_user` 复用 D3a 的挂起标记形态（`control_outcome_pending`），其余调用返回 ok。
pub(super) struct ConcurrencyProbeToolExecutor {
    pub(super) entries: Arc<Mutex<Vec<(String, usize)>>>,
    pub(super) active: Arc<std::sync::atomic::AtomicUsize>,
    pub(super) peak: Arc<std::sync::atomic::AtomicUsize>,
    pub(super) expected_peak: usize,
    pub(super) wait_budget: std::time::Duration,
    /// 集合点的唤醒通道（`Mutex` + `Condvar`）；真正的条件是 `active`/`peak` 两个原子值。
    pub(super) rendezvous: Arc<(Mutex<()>, std::sync::Condvar)>,
}

impl ConcurrencyProbeToolExecutor {
    pub(super) fn new(expected_peak: usize, wait_budget: std::time::Duration) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            peak: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            expected_peak,
            wait_budget,
            rendezvous: Arc::new((Mutex::new(()), std::sync::Condvar::new())),
        }
    }

    pub(super) fn executed_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// 该工具每次调用「进入时」的在途并发数（1 = 独占执行）。
    pub(super) fn entry_concurrency_of(&self, name: &str) -> Vec<usize> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(executed, _)| executed == name)
            .map(|(_, active_at_entry)| *active_at_entry)
            .collect()
    }

    pub(super) fn peak(&self) -> usize {
        self.peak.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl crate::agent::tools::ToolExecutor for ConcurrencyProbeToolExecutor {
    fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
        let active_at_entry =
            self.active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.peak
            .fetch_max(active_at_entry, std::sync::atomic::Ordering::SeqCst);
        self.entries
            .lock()
            .unwrap()
            .push((call.name.clone(), active_at_entry));
        // 有界集合点：等「峰值达到期望值」或预算耗尽（取代固定 sleep 造出来的重叠窗口）。
        // 顺序要点：先登记 active/peak，再等待——并发对端只要进入就一定能让条件成立。
        let deadline = std::time::Instant::now() + self.wait_budget;
        let mut guard = self
            .rendezvous
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while self
            .peak
            .load(std::sync::atomic::Ordering::SeqCst)
            < self.expected_peak
        {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            guard = self
                .rendezvous
                .1
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        drop(guard);
        // 本调用已在途，唤醒可能在等它的对端。
        self.rendezvous.1.notify_all();
        self.active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

        let is_ask = crate::agent::tools::canonical_tool_name(&call.name)
            .map(|canonical| canonical == "ask_user")
            .unwrap_or(false);
        if is_ask {
            return crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "error".to_string(),
                output: json!({
                    "ok": false,
                    "error": {
                        "code": "control_outcome_pending",
                        "kind": "control_outcome_pending",
                        "message": "Ask 等待宿主应答。"
                    }
                })
                .to_string(),
                duration_ms: 0,
            };
        }
        crate::agent::tools::ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            // 结果自带「被执行的是哪个调用」的证据：并发下完成顺序不定，回填后仍须与调用一一对应。
            output: json!({ "executed": call.name, "arguments": call.arguments }).to_string(),
            duration_ms: 1,
        }
    }
}


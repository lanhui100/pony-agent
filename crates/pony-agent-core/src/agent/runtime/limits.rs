// AtomicUsize/Ordering 经 use super::* 从门面供养链解析（见 mod.rs 供养注记）
use super::*;

pub(super) const DEFAULT_MAX_TOOL_HOPS_PER_TURN: usize = 1024;

pub(super) const MAX_ALLOWED_TOOL_HOPS_PER_TURN: usize = 4096;

pub(super) const MAX_TOOL_HOPS_ENV: &str = "PONY_AGENT_MAX_TOOL_HOPS_PER_TURN";

pub const DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN: usize = 500;

pub const MAX_ALLOWED_TOOL_FOLLOWUPS_PER_TURN: usize = 2048;

pub(super) const MAX_TOOL_FOLLOWUPS_ENV: &str = "PONY_AGENT_MAX_TOOL_FOLLOWUPS_PER_TURN";

/// 同一 turn 内「同一工具 + 同一错误码」连续失败达到该次数时，终止 follow-up 并把
/// 真实错误直接呈现给用户，避免模型把整个 follow-up 预算浪费在注定失败的重复重试上。
pub(super) const DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN: usize = 3;

pub(super) const MAX_ALLOWED_CONSECUTIVE_TOOL_FAILURES_PER_TURN: usize = 16;

pub(super) const MAX_CONSECUTIVE_TOOL_FAILURES_ENV: &str =
    "PONY_AGENT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN";

/// 同一工具「连续失败」上限（P1-3）：只看工具名，不看入参与错误码。
///
/// 为什么需要第二道闸：既有指纹闸是 `(工具, 入参指纹, 错误码)` 三元组，模型只要换参数或
/// 换错误码就能绕过——实测 `JobOutput` 连续失败 5 次仍未被停（trace `dab6a42e…`）。
pub(super) const DEFAULT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES: usize = 4;

pub(super) const MAX_ALLOWED_CONSECUTIVE_SAME_TOOL_FAILURES: usize = 16;

pub(super) const MAX_CONSECUTIVE_SAME_TOOL_FAILURES_ENV: &str =
    "PONY_AGENT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES";

/// 同签名重复工具调用闸（集合 → 计数）：同一 turn 内「同一工具 + 同一归一化参数」允许执行的
/// 次数，第 N+1 次触发闸。默认 2 ⇒ 前 2 次放行、第 3 次熔断并恢复式收尾。
pub(super) const DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN: usize = 2;

pub(super) const MAX_ALLOWED_SAME_SIGNATURE_OCCURRENCES_PER_TURN: usize = 8;

pub(super) const MAX_SAME_SIGNATURE_OCCURRENCES_ENV: &str =
    "PONY_AGENT_MAX_SAME_TOOL_SIGNATURE_OCCURRENCES_PER_TURN";

/// 批内并发执行开关（D4 / W28b）：`PONY_AGENT_PARALLEL_TOOLS`。
///
/// **默认关**（fail-safe）：只有显式 `1/true/on/yes`（大小写不敏感、允许首尾空白）才开启；
/// 未设置、空串、任何其它取值（含拼写错误）一律按**关闭**处理 —— 并发是「可能改变工具间时序」
/// 的语义级变更，绝不能因为一个环境变量写错而被动开启。
pub(super) const PARALLEL_TOOLS_ENV: &str = "PONY_AGENT_PARALLEL_TOOLS";

pub(super) fn build_same_tool_consecutive_failure_error(
    limit: usize,
    tool_name: &str,
    output: &str,
) -> String {
    format!(
        "工具 `{tool_name}` 已连续失败 {limit} 次（无论参数/错误码是否相同），已停止继续 follow-up \
以避免在同一工具上打转；最后一次错误详情：{}。如属误判，可提高 PONY_AGENT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES。",
        preview_text(output, 200)
    )
}

// 测试覆写为什么用 thread_local 而不是进程级 Atomic：turn 在调用线程上同步执行，thread_local
// 既能让用例精确注入预算，又不会污染并行跑的其他用例（进程级覆写会让并发用例误触预算）。
#[cfg(test)]
thread_local! {
    static SAME_TOOL_FAILURE_OVERRIDE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SAME_SIGNATURE_OCCURRENCES_OVERRIDE: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    /// `Some(bool)` ⇒ 用例显式覆写并发开关（`None` = 未覆写，回落 env）。
    static PARALLEL_TOOLS_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn set_same_tool_failure_override(value: usize) -> usize {
    SAME_TOOL_FAILURE_OVERRIDE.with(|slot| slot.replace(value))
}

/// 测试专用：设置同签名重复调用闸的允许次数覆写，返回先前的覆写值（便于用例收尾还原）。
#[cfg(test)]
pub(super) fn set_same_signature_occurrences_override(value: usize) -> usize {
    SAME_SIGNATURE_OCCURRENCES_OVERRIDE.with(|slot| slot.replace(value))
}

/// 测试专用：设置并发开关覆写，返回先前的覆写值（便于用例收尾还原）。
#[cfg(test)]
pub(super) fn set_parallel_tools_override(value: Option<bool>) -> Option<bool> {
    PARALLEL_TOOLS_OVERRIDE.with(|slot| slot.replace(value))
}

/// 批内并发执行是否开启（`PONY_AGENT_PARALLEL_TOOLS`，**默认关**、fail-safe）。
pub(super) fn parallel_tools_enabled() -> bool {
    #[cfg(test)]
    {
        if let Some(override_value) = PARALLEL_TOOLS_OVERRIDE.with(|slot| slot.get()) {
            return override_value;
        }
    }

    static PARALLEL_TOOLS_ENABLED: OnceLock<bool> = OnceLock::new();
    *PARALLEL_TOOLS_ENABLED.get_or_init(|| {
        parse_parallel_tools_enabled(std::env::var(PARALLEL_TOOLS_ENV).ok().as_deref())
    })
}

/// 解析开关取值：仅 `1/true/on/yes`（大小写不敏感、允许首尾空白）为开；其余一律关。
pub(super) fn parse_parallel_tools_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|value| value.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("on") | Some("yes")
    )
}

pub(super) fn max_consecutive_same_tool_failures_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = SAME_TOOL_FAILURE_OVERRIDE.with(|slot| slot.get());
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_SAME_TOOL_FAILURES: OnceLock<usize> = OnceLock::new();
    *MAX_SAME_TOOL_FAILURES.get_or_init(|| {
        parse_max_consecutive_same_tool_failures(
            std::env::var(MAX_CONSECUTIVE_SAME_TOOL_FAILURES_ENV)
                .ok()
                .as_deref(),
        )
    })
}

pub(super) fn parse_max_consecutive_same_tool_failures(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_CONSECUTIVE_SAME_TOOL_FAILURES).contains(value))
        .unwrap_or(DEFAULT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES)
}

pub(super) fn max_same_signature_occurrences_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = SAME_SIGNATURE_OCCURRENCES_OVERRIDE.with(|slot| slot.get());
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_SAME_SIGNATURE_OCCURRENCES: OnceLock<usize> = OnceLock::new();
    *MAX_SAME_SIGNATURE_OCCURRENCES.get_or_init(|| {
        parse_max_same_signature_occurrences_per_turn(
            std::env::var(MAX_SAME_SIGNATURE_OCCURRENCES_ENV)
                .ok()
                .as_deref(),
        )
    })
}

pub(super) fn parse_max_same_signature_occurrences_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_SAME_SIGNATURE_OCCURRENCES_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN)
}

pub(super) fn build_consecutive_tool_failure_error(limit: usize, tool_name: &str, code: &str, output: &str) -> String {
    format!(
        "工具 `{tool_name}` 连续 {limit} 次以同一错误 `{code}` 失败，已停止继续 follow-up 以避免无效重试；最后一次错误详情：{}。如属误判，可提高 PONY_AGENT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN。",
        preview_text(output, 200)
    )
}


/// Unix-epoch milliseconds used for graph binding timestamps.
pub(super) fn runtime_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}


pub(super) fn build_tool_hop_limit_error(limit: usize) -> String {
    format!(
        "同一 turn 内连续工具调用超过 {} 次，已停止继续 follow-up 以避免进入无限循环；如属复杂任务，可提高 PONY_AGENT_MAX_TOOL_HOPS_PER_TURN。",
        limit
    )
}


pub(super) fn build_tool_followup_limit_error(limit: usize) -> String {
    format!(
        "同一 turn 内 follow-up 轮次超过 {} 次，已停止继续 follow-up 以避免重复探索；如属复杂任务，可提高 PONY_AGENT_MAX_TOOL_FOLLOWUPS_PER_TURN。",
        limit
    )
}


/// 同签名重复工具调用闸的文案（契约矩阵 C4）。
///
/// `occurrences` = 本次被拦的是「第几次」（= 已执行次数 + 1）；`limit` = 允许执行次数上限。
pub(super) fn build_duplicate_tool_call_error(
    tool_call: &ToolCall,
    occurrences: usize,
    limit: usize,
) -> String {
    format!(
        "工具 `{}` 在同一 turn 内已第 {occurrences} 次以完全相同的参数被调用\
（同签名判定口径：同名 + 同参数，仅忽略 `description` 与 key 顺序；同一 turn 内最多允许 {limit} 次），\
已停止继续 follow-up 以避免重复探索。若上次调用失败或结果已失效，请改变参数、先确认当前状态与已有证据，\
或直接基于已有证据作答。",
        tool_call.name
    )
}


pub(super) fn build_tool_execution_error(tool_name: &str, output: &str) -> String {
    format!(
        "工具 `{}` 执行失败：{}",
        tool_name,
        preview_text(output, 160)
    )
}


#[cfg(test)]
pub(super) fn tool_hop_limit_override_registry() -> &'static AtomicUsize {
    static OVERRIDE: OnceLock<AtomicUsize> = OnceLock::new();
    OVERRIDE.get_or_init(|| AtomicUsize::new(0))
}


pub(super) fn max_tool_hops_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = tool_hop_limit_override_registry().load(AtomicOrdering::SeqCst);
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_TOOL_HOPS: OnceLock<usize> = OnceLock::new();
    *MAX_TOOL_HOPS.get_or_init(|| {
        parse_max_tool_hops_per_turn(std::env::var(MAX_TOOL_HOPS_ENV).ok().as_deref())
    })
}


pub(super) fn parse_max_tool_hops_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_TOOL_HOPS_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_TOOL_HOPS_PER_TURN)
}


#[cfg(test)]
pub(super) fn tool_followup_limit_override_registry() -> &'static AtomicUsize {
    static OVERRIDE: OnceLock<AtomicUsize> = OnceLock::new();
    OVERRIDE.get_or_init(|| AtomicUsize::new(0))
}


pub fn max_tool_followups_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = tool_followup_limit_override_registry().load(AtomicOrdering::SeqCst);
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_TOOL_FOLLOWUPS: OnceLock<usize> = OnceLock::new();
    *MAX_TOOL_FOLLOWUPS.get_or_init(|| {
        parse_max_tool_followups_per_turn(std::env::var(MAX_TOOL_FOLLOWUPS_ENV).ok().as_deref())
    })
}


pub fn parse_max_tool_followups_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_TOOL_FOLLOWUPS_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN)
}


#[cfg(test)]
pub(super) fn consecutive_failure_limit_override_registry() -> &'static AtomicUsize {
    static OVERRIDE: OnceLock<AtomicUsize> = OnceLock::new();
    OVERRIDE.get_or_init(|| AtomicUsize::new(0))
}


pub(super) fn max_consecutive_tool_failures_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = consecutive_failure_limit_override_registry().load(AtomicOrdering::SeqCst);
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_CONSECUTIVE_FAILURES: OnceLock<usize> = OnceLock::new();
    *MAX_CONSECUTIVE_FAILURES.get_or_init(|| {
        parse_max_consecutive_tool_failures_per_turn(
            std::env::var(MAX_CONSECUTIVE_TOOL_FAILURES_ENV).ok().as_deref(),
        )
    })
}


pub(super) fn parse_max_consecutive_tool_failures_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_CONSECUTIVE_TOOL_FAILURES_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN)
}

// AtomicUsize/Ordering 经 use super::* 从门面供养链解析（见 mod.rs 供养注记）
use super::*;

pub(super) const DEFAULT_MAX_TOOL_HOPS_PER_TURN: usize = 1024;

pub(super) const MAX_ALLOWED_TOOL_HOPS_PER_TURN: usize = 4096;

pub(super) const MAX_TOOL_HOPS_ENV: &str = "PONY_AGENT_MAX_TOOL_HOPS_PER_TURN";

pub(super) const DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN: usize = 12;

pub(super) const MAX_ALLOWED_TOOL_FOLLOWUPS_PER_TURN: usize = 32;

pub(super) const MAX_TOOL_FOLLOWUPS_ENV: &str = "PONY_AGENT_MAX_TOOL_FOLLOWUPS_PER_TURN";

/// 同一 turn 内「同一工具 + 同一错误码」连续失败达到该次数时，终止 follow-up 并把
/// 真实错误直接呈现给用户，避免模型把整个 follow-up 预算浪费在注定失败的重复重试上。
pub(super) const DEFAULT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN: usize = 3;

pub(super) const MAX_ALLOWED_CONSECUTIVE_TOOL_FAILURES_PER_TURN: usize = 16;

pub(super) const MAX_CONSECUTIVE_TOOL_FAILURES_ENV: &str =
    "PONY_AGENT_MAX_CONSECUTIVE_TOOL_FAILURES_PER_TURN";

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


pub(super) fn build_duplicate_tool_call_error(tool_call: &ToolCall) -> String {
    format!(
        "工具 `{}` 在同一 turn 内重复调用了近似相同的参数，已停止继续 follow-up 以避免重复探索。",
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


pub(super) fn max_tool_followups_per_turn() -> usize {
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


pub(super) fn parse_max_tool_followups_per_turn(raw: Option<&str>) -> usize {
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

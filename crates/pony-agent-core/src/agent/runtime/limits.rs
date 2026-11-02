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

/// 单轮墙钟预算（秒，P1-3）：超预算后不再继续 follow-up，而是在下一个工具调用边界
/// 走「恢复式收尾」给出可读结论。默认 900s（15min）——真实长轮 trace `dab6a42e…`
/// 跑了 942s / 71 次调用；预算是兜底，不是主止损手段（主止损见同工具连续失败）。
pub(super) const DEFAULT_MAX_TURN_WALL_SECONDS: usize = 900;

pub(super) const MAX_ALLOWED_TURN_WALL_SECONDS: usize = 7_200;

pub(super) const MAX_TURN_WALL_SECONDS_ENV: &str = "PONY_AGENT_MAX_TURN_WALL_SECONDS";

/// 同一工具「连续失败」上限（P1-3）：只看工具名，不看入参与错误码。
///
/// 为什么需要第二道闸：既有指纹闸是 `(工具, 入参指纹, 错误码)` 三元组，模型只要换参数或
/// 换错误码就能绕过——实测 `JobOutput` 连续失败 5 次仍未被停（trace `dab6a42e…`）。
pub(super) const DEFAULT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES: usize = 4;

pub(super) const MAX_ALLOWED_CONSECUTIVE_SAME_TOOL_FAILURES: usize = 16;

pub(super) const MAX_CONSECUTIVE_SAME_TOOL_FAILURES_ENV: &str =
    "PONY_AGENT_MAX_CONSECUTIVE_SAME_TOOL_FAILURES";

pub(super) fn build_turn_wall_budget_error(limit_seconds: usize) -> String {
    format!(
        "本轮已运行超过 {limit_seconds} 秒（墙钟预算），已停止继续调用工具以避免长时间占用；\
请基于已经获得的证据给出结论，并明确仍未完成的部分。如确需更长时限，可提高 PONY_AGENT_MAX_TURN_WALL_SECONDS。"
    )
}

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
    static TURN_WALL_OVERRIDE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SAME_TOOL_FAILURE_OVERRIDE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn set_turn_wall_override(value: usize) -> usize {
    TURN_WALL_OVERRIDE.with(|slot| slot.replace(value))
}

#[cfg(test)]
pub(super) fn set_same_tool_failure_override(value: usize) -> usize {
    SAME_TOOL_FAILURE_OVERRIDE.with(|slot| slot.replace(value))
}

pub(super) fn max_turn_wall_seconds() -> usize {
    #[cfg(test)]
    {
        let override_limit = TURN_WALL_OVERRIDE.with(|slot| slot.get());
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_TURN_WALL_SECONDS: OnceLock<usize> = OnceLock::new();
    *MAX_TURN_WALL_SECONDS.get_or_init(|| {
        parse_max_turn_wall_seconds(std::env::var(MAX_TURN_WALL_SECONDS_ENV).ok().as_deref())
    })
}

pub(super) fn parse_max_turn_wall_seconds(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_TURN_WALL_SECONDS).contains(value))
        .unwrap_or(DEFAULT_MAX_TURN_WALL_SECONDS)
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

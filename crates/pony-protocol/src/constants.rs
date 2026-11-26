use std::time::Duration;

pub const DEFAULT_RUN_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_TOOL_TIMEOUT_MS: u64 = 10_000;
pub const MAX_FULL_READ_BYTES: u64 = 120_000;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 512 * 1024;
pub const MAX_SKILL_BODY_BYTES: usize = 262_144;
pub const ACP_DEFAULT_TURN_BUDGET: Duration = Duration::from_secs(120);
pub const SUBAGENT_MAX_CONCURRENT_CEILING: usize = 8;
pub const HOST_HISTORY_ROLE: &str = "host";
pub const DEFAULT_SESSION_TITLE: &str = "\u{65B0}\u{5BF9}\u{8BDD}";

pub fn default_session_title() -> String {
    DEFAULT_SESSION_TITLE.to_string()
}

pub fn normalize_subagent_max_concurrent(configured: Option<usize>) -> Option<usize> {
    configured.map(|value| value.clamp(1, SUBAGENT_MAX_CONCURRENT_CEILING))
}

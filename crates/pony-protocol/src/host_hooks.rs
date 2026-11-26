use std::sync::OnceLock;

type GitIdentityEnvFn = fn(Option<&str>) -> Vec<(String, String)>;
type SubagentWiredProbeFn = fn() -> bool;

static GIT_IDENTITY_ENV_PROVIDER: OnceLock<GitIdentityEnvFn> = OnceLock::new();
static SUBAGENT_WIRED_PROBE: OnceLock<SubagentWiredProbeFn> = OnceLock::new();

pub fn register_teammate_git_env_provider(f: GitIdentityEnvFn) {
    let _ = GIT_IDENTITY_ENV_PROVIDER.set(f);
}

pub fn git_identity_env_for_session(session_id: Option<&str>) -> Vec<(String, String)> {
    match GIT_IDENTITY_ENV_PROVIDER.get() {
        Some(f) => f(session_id),
        None => Vec::new(),
    }
}

pub fn git_identity_allowlist_for_session(session_id: Option<&str>) -> Vec<String> {
    git_identity_env_for_session(session_id)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect()
}

pub fn register_subagent_wired_probe(f: SubagentWiredProbeFn) {
    let _ = SUBAGENT_WIRED_PROBE.set(f);
}

pub fn is_subagent_runtime_wired() -> bool {
    SUBAGENT_WIRED_PROBE.get().map_or(false, |f| f())
}

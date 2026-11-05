//! W2 RED：team/subagent provider 继承与覆盖的接口契约（task-7）。
//!
//! 契约来源：`.dev-team/contracts/`（W2 provider inheritance）+ task-7 冻结桩提交 `d169a287`。
//! 本文件只断言**接口面**（`resolve_teammate_provider_selection`、`SpawnTeammateArgs` 的
//! `provider_id`/`model_id`、`ProviderManager::selection()`、`SubagentTurnEngine::
//! run_turn_with_selection` 默认委托），不含任何生产逻辑。
//!
//! 确定性约束：不读写全局 `SUBAGENT_ENGINE` OnceLock、不联网、不依赖外部配置/环境，
//! 每个用例只构造本地值。

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderReasoningEffort, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::orchestration::{
    resolve_teammate_provider_selection, SpawnTeammateArgs,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderManager, ProviderProtocol};
use pony_agent_core::agent::subagent_runtime::{SubagentRunOutcome, SubagentTurnEngine};

/// 宿主当前生效的 provider 选择（继承源）。全部为本地值，不触网、不读环境。
fn host_selection() -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "w2-host-requested".to_string(),
        provider_name: "w2-host-provider".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: "https://host.invalid/v1".to_string(),
        auth_type: ProviderAuthType::Bearer,
        api_key_env_var: "W2_HOST_API_KEY".to_string(),
        api_key: Some("w2-host-secret".to_string()),
        model: "w2-host-model".to_string(),
        temperature: 0.25,
        max_output_tokens: 2048,
        reasoning_effort: Some(ProviderReasoningEffort::Medium),
        reasoning_budget_tokens: Some(4096),
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(32_768),
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn describe(selection: &ResolvedProviderSelection) -> String {
    format!(
        "requested_name={} provider_name={} model={} base_url={}",
        selection.requested_name,
        selection.provider_name,
        selection.model,
        selection.base_url
    )
}

/// C1：宿主继承 —— `(Some(&host), None, None)` ⇒ Ok 且 provider/model 全部沿用宿主。
#[test]
fn c1_inherits_host_selection_when_no_override() {
    let host = host_selection();

    let resolved = resolve_teammate_provider_selection(Some(&host), None, None);

    let resolved = match resolved {
        Ok(resolved) => resolved,
        Err(err) => panic!("C1：无显式覆盖时必须继承宿主选择，实际 Err = {err}"),
    };
    eprintln!("[C1] resolved  = {}", describe(&resolved));
    eprintln!("[C1] host      = {}", describe(&host));

    assert_eq!(
        resolved.provider_name, host.provider_name,
        "C1：provider_name 必须继承宿主"
    );
    assert_eq!(
        resolved.requested_name, host.requested_name,
        "C1：requested_name 必须继承宿主"
    );
    assert_eq!(resolved.model, host.model, "C1：model 必须继承宿主");
}

/// C2：model 覆盖 —— 只覆盖 model，不改变 provider。
#[test]
fn c2_overrides_model_without_changing_provider() {
    let host = host_selection();

    let resolved = resolve_teammate_provider_selection(Some(&host), None, Some("custom-model-xyz"));

    let resolved = match resolved {
        Ok(resolved) => resolved,
        Err(err) => panic!("C2：显式 model 覆盖必须解析成功，实际 Err = {err}"),
    };
    eprintln!("[C2] resolved  = {}", describe(&resolved));
    eprintln!("[C2] host      = {}", describe(&host));

    assert_eq!(
        resolved.model, "custom-model-xyz",
        "C2：显式 model_id 必须覆盖宿主 model"
    );
    assert_eq!(
        resolved.provider_name, host.provider_name,
        "C2：仅覆盖 model 时 provider 必须保持不变"
    );
}

/// C3：无宿主且无显式覆盖 ⇒ Err（错误码 `missing_host_provider_selection`）。
#[test]
fn c3_missing_host_selection_is_rejected_with_stable_code() {
    let result = resolve_teammate_provider_selection(None, None, None);

    match result {
        Ok(resolved) => panic!(
            "C3：既无 host 也无显式 provider/model 时必须 Err，实际 Ok = {}",
            describe(&resolved)
        ),
        Err(err) => {
            eprintln!("[C3] err = {err}");
            assert!(
                err.contains("missing_host_provider_selection"),
                "C3：错误信息必须包含 `missing_host_provider_selection`，实际 = {err}"
            );
        }
    }
}

/// C4：显式未知 provider ⇒ Err（错误码 `unknown_provider`）。
#[test]
fn c4_unknown_provider_override_is_rejected_with_stable_code() {
    let host = host_selection();

    let result = resolve_teammate_provider_selection(
        Some(&host),
        Some("definitely-no-such-provider"),
        None,
    );

    match result {
        Ok(resolved) => panic!(
            "C4：未知 provider_id 必须 Err，实际 Ok = {}",
            describe(&resolved)
        ),
        Err(err) => {
            eprintln!("[C4] err = {err}");
            assert!(
                err.contains("unknown_provider"),
                "C4：错误信息必须包含 `unknown_provider`，实际 = {err}"
            );
        }
    }
}

/// C5：`SpawnTeammateArgs` 的 `provider_id`/`model_id` 参与 serde 解析，缺省为 None。
#[test]
fn c5_spawn_teammate_args_serde_carries_provider_overrides() {
    let with_overrides: SpawnTeammateArgs = serde_json::from_value(serde_json::json!({
        "name": "w2-worker",
        "role": "worker",
        "description": "w2 provider override probe",
        "prompt": "do the thing",
        "context": "fresh",
        "provider_id": "w2-provider-x",
        "model_id": "w2-model-y",
    }))
    .expect("C5：带 provider_id/model_id 的 JSON 必须可反序列化");

    eprintln!(
        "[C5] with_overrides provider_id={:?} model_id={:?}",
        with_overrides.provider_id, with_overrides.model_id
    );
    assert_eq!(
        with_overrides.provider_id.as_deref(),
        Some("w2-provider-x"),
        "C5：provider_id 必须被解析"
    );
    assert_eq!(
        with_overrides.model_id.as_deref(),
        Some("w2-model-y"),
        "C5：model_id 必须被解析"
    );

    let without_overrides: SpawnTeammateArgs = serde_json::from_value(serde_json::json!({
        "name": "w2-worker-2",
        "description": "w2 inherit probe",
        "prompt": "inherit everything",
    }))
    .expect("C5：缺省 provider_id/model_id 的 JSON 必须可反序列化");

    eprintln!(
        "[C5] without_overrides provider_id={:?} model_id={:?}",
        without_overrides.provider_id, without_overrides.model_id
    );
    assert!(
        without_overrides.provider_id.is_none(),
        "C5：缺省 provider_id 必须为 None（= 继承宿主）"
    );
    assert!(
        without_overrides.model_id.is_none(),
        "C5：缺省 model_id 必须为 None（= 继承宿主）"
    );

    // 序列化侧：None 不得凭空写出覆盖键（否则继承语义会被写坏）。
    let serialized = serde_json::to_value(&without_overrides).expect("C5：序列化必须成功");
    assert!(
        serialized.get("provider_id").is_none(),
        "C5：None 的 provider_id 不得被序列化，实际 = {serialized}"
    );
    assert!(
        serialized.get("model_id").is_none(),
        "C5：None 的 model_id 不得被序列化，实际 = {serialized}"
    );
}

/// C6：`ProviderManager::selection()` 原样返回注入的选择（只读真值源）。
#[test]
fn c6_provider_manager_selection_round_trips_injected_selection() {
    let selection = host_selection();

    let manager = ProviderManager::new(selection.clone());
    let observed = manager.selection();

    eprintln!("[C6] observed = {}", describe(observed));
    eprintln!("[C6] injected = {}", describe(&selection));

    assert_eq!(
        observed.requested_name, selection.requested_name,
        "C6：requested_name 必须与注入值一致"
    );
    assert_eq!(
        observed.provider_name, selection.provider_name,
        "C6：provider_name 必须与注入值一致"
    );
    assert_eq!(
        observed.model, selection.model,
        "C6：model 必须与注入值一致"
    );
    assert_eq!(
        observed.base_url, selection.base_url,
        "C6：base_url 必须与注入值一致"
    );
    assert_eq!(
        observed.protocol, selection.protocol,
        "C6：protocol 必须与注入值一致"
    );
    assert_eq!(
        observed.auth_type, selection.auth_type,
        "C6：auth_type 必须与注入值一致"
    );
    assert_eq!(
        observed.api_key, selection.api_key,
        "C6：api_key 必须与注入值一致（不得被清空/重解析）"
    );
    assert_eq!(
        observed.temperature, selection.temperature,
        "C6：temperature 必须与注入值一致"
    );
    assert_eq!(
        observed.max_output_tokens, selection.max_output_tokens,
        "C6：max_output_tokens 必须与注入值一致"
    );
}

/// 只覆写 `run_turn_in` 的计数引擎：用于验证 `run_turn_with_selection` 的默认委托路径。
struct CountingEngine {
    run_turn_calls: AtomicUsize,
    run_turn_in_calls: AtomicUsize,
    last_workspace_root_is_none: AtomicUsize,
}

impl CountingEngine {
    fn new() -> Self {
        Self {
            run_turn_calls: AtomicUsize::new(0),
            run_turn_in_calls: AtomicUsize::new(0),
            last_workspace_root_is_none: AtomicUsize::new(0),
        }
    }

    fn outcome(tag: &str) -> SubagentRunOutcome {
        SubagentRunOutcome {
            final_text: format!("{tag}"),
            session_id: format!("{tag}-session"),
        }
    }
}

impl SubagentTurnEngine for CountingEngine {
    fn run_turn(&self, _prompt: &str, _timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        self.run_turn_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Self::outcome("run_turn"))
    }

    fn run_turn_in(
        &self,
        _prompt: &str,
        _timeout_ms: u64,
        workspace_root: Option<&Path>,
    ) -> Result<SubagentRunOutcome, String> {
        self.run_turn_in_calls.fetch_add(1, Ordering::SeqCst);
        if workspace_root.is_none() {
            self.last_workspace_root_is_none
                .fetch_add(1, Ordering::SeqCst);
        }
        Ok(Self::outcome("run_turn_in"))
    }
}

/// C7：默认 `run_turn_with_selection(..., None)` 必须原样委托 `run_turn_in` **恰好一次**。
#[test]
fn c7_default_run_turn_with_selection_delegates_to_run_turn_in_once() {
    let engine = CountingEngine::new();

    let outcome = engine
        .run_turn_with_selection("w2 prompt", 1_000, None, None)
        .expect("C7：默认 run_turn_with_selection 必须成功委托");

    let run_turn_calls = engine.run_turn_calls.load(Ordering::SeqCst);
    let run_turn_in_calls = engine.run_turn_in_calls.load(Ordering::SeqCst);
    eprintln!(
        "[C7] run_turn_calls={run_turn_calls} run_turn_in_calls={run_turn_in_calls} final_text={} session_id={}",
        outcome.final_text, outcome.session_id
    );

    assert_eq!(
        run_turn_in_calls, 1,
        "C7：默认实现必须委托 run_turn_in 恰好一次"
    );
    assert_eq!(
        run_turn_calls, 0,
        "C7：默认实现不得绕过 run_turn_in 直接调用 run_turn"
    );
    assert_eq!(
        outcome.final_text, "run_turn_in",
        "C7：返回值必须来自 run_turn_in 的实现"
    );
    assert_eq!(
        engine.last_workspace_root_is_none.load(Ordering::SeqCst),
        1,
        "C7：workspace_root 必须原样透传（None ⇒ None）"
    );
}

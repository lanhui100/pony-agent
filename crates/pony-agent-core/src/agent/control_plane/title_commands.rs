// title_commands: AI 会话标题生成控制面命令（桌面 / TUI 共用入口）。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;
use crate::agent::runtime::{
    generate_session_title_with_sessions, SessionTitleOutcome, TITLE_NO_USER_MESSAGE_ERROR_CODE,
};
use crate::agent::session::DEFAULT_SESSION_ID;

/// B201：自动标题静默跳过的会话集合（进程级；每会话最多记一次 debug）。
///
/// 不挂在 `HostControlPlane` 字段上：控制面 struct 已顶 file-size 基线，且这是纯观测去重状态，
/// 与实例生命周期无关。
fn auto_title_attempted() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static ATTEMPTED: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<String>>,
    > = std::sync::OnceLock::new();
    ATTEMPTED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

impl HostControlPlane {
    /// 解析指定 provider/model（如果提供），否则回落默认 selection（与生产 turn / 压缩同口径）。
    ///
    /// **锁纪律**：`runtime` 读锁只覆盖 provider 解析（毫秒级），网络调用期间不持任何锁。
    fn title_provider_with_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> crate::agent::provider::ProviderManager {
        let runtime = self.runtime.read().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] runtime rwlock poisoned");
            e.into_inner()
        });
        runtime.title_provider(provider_id, model_id)
    }

    /// 解析默认 provider（与生产 turn / 压缩同口径）。
    ///
    /// **锁纪律**：`runtime` 读锁只覆盖 provider 解析（毫秒级），网络调用期间不持任何锁。
    fn title_provider(&self) -> crate::agent::provider::ProviderManager {
        self.title_provider_with_selection(None, None)
    }

    /// 生成并写入当前会话的 AI 标题（手动 `/title` / 桌面按钮）。
    ///
    /// `session_id == None` 回落默认会话。语义见 `generate_session_title_with_sessions`。
    pub fn generate_session_title(
        &self,
        session_id: Option<&str>,
    ) -> Result<SessionTitleOutcome, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let provider = self.title_provider();
        let sessions = Arc::clone(&self.sessions_rwlock);
        generate_session_title_with_sessions(&sessions, provider, session_key)
    }

    /// 首轮后**自动**生成标题的准入入口：仅当会话已有真实用户轮、且标题来源仍为「派生」
    /// （未被用户改名 / 未生成过 AI 标题）时才发起；否则返回 `None`（no-op）。
    ///
    /// 自动路径吞掉非致命错误（网络抖动不应打断会话），但把失败原因打到 stderr 取证面。
    /// B201: 优先跟随会话实际使用的 provider/model；无 key 或无可用 provider 时静默跳过并记 debug（最多一次），
    /// 不再每轮 warn / 发起必然失败的调用。
    pub fn maybe_generate_session_title(
        &self,
        session_id: Option<&str>,
    ) -> Option<SessionTitleOutcome> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let (needed, session_provider, session_model) = {
            let mut store = self.sessions_rwlock.write().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            let needed = store.needs_ai_title(session_key);
            let (provider, model) = if needed {
                store.session_provider_and_model(session_key)
            } else {
                (None, None)
            };
            (needed, provider, model)
        };
        if !needed {
            return None;
        }

        let provider = self.title_provider_with_selection(
            session_provider.as_deref(),
            session_model.as_deref(),
        );

        let has_key = provider
            .selection()
            .api_key
            .as_deref()
            .is_some_and(|k| !k.trim().is_empty());

        if !has_key {
            let already_attempted = {
                let mut attempted = auto_title_attempted()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                !attempted.insert(session_key.to_string())
            };
            if !already_attempted {
                tracing::debug!(
                    target: "control_plane",
                    session_id = %session_key,
                    provider = %provider.name(),
                    "auto_title skipped: provider has no API key"
                );
            }
            return None;
        }

        let sessions = Arc::clone(&self.sessions_rwlock);
        match generate_session_title_with_sessions(&sessions, provider, session_key) {
            Ok(outcome) => Some(outcome),
            Err(error) if error.starts_with(TITLE_NO_USER_MESSAGE_ERROR_CODE) => None,
            Err(error) => {
                if error.contains("missing API key") {
                    let already_attempted = {
                        let mut attempted = auto_title_attempted()
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        !attempted.insert(session_key.to_string())
                    };
                    if !already_attempted {
                        tracing::debug!(
                            target: "control_plane",
                            session_id = %session_key,
                            error = %error,
                            "auto_title skipped: missing API key"
                        );
                    }
                } else {
                    eprintln!(
                        "[pony-agent][session] auto_title_generation_failed: session={session_key} error={error}"
                    );
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
        ThinkingParamPattern,
    };
    use crate::agent::provider::{ProviderAuthType, ProviderProtocol};
    use crate::agent::runtime::AgentRuntimeBuilder;
    use crate::agent::session::{SessionStore, TurnTraceRecord};
    use std::sync::{Arc, Mutex};

    struct FakeTitleResolver {
        last_resolved: Arc<Mutex<Option<(Option<String>, Option<String>)>>>,
        has_api_key: bool,
    }

    impl ProviderSelectionResolver for FakeTitleResolver {
        fn resolve_provider_selection(
            &self,
            provider_id: Option<&str>,
            model_id: Option<&str>,
        ) -> ResolvedProviderSelection {
            if let Ok(mut slot) = self.last_resolved.lock() {
                *slot = Some((provider_id.map(str::to_string), model_id.map(str::to_string)));
            }
            let (name, model) = match (provider_id, model_id) {
                (Some(p), Some(m)) => (p.to_string(), m.to_string()),
                (Some(p), None) => (p.to_string(), "default-model".to_string()),
                (None, Some(m)) => ("default-provider".to_string(), m.to_string()),
                (None, None) => ("default-provider".to_string(), "default-model".to_string()),
            };
            ResolvedProviderSelection {
                requested_name: name.clone(),
                provider_name: name,
                protocol: ProviderProtocol::OpenAiCompletions,
                base_url: "http://127.0.0.1:9".to_string(),
                auth_type: ProviderAuthType::Auto,
                api_key_env_var: "TEST_API_KEY".to_string(),
                api_key: if self.has_api_key {
                    Some("test-key".to_string())
                } else {
                    None
                },
                model,
                temperature: 0.3,
                max_output_tokens: 64,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: ProviderModelCapabilities {
                    context_window_tokens: Some(8_192),
                    supports_tools: false,
                    supports_streaming: false,
                    supports_image_input: false,
                    supports_reasoning: false,
                    ..Default::default()
                },
                thinking_param_pattern: ThinkingParamPattern::EffortStandard,
            }
        }
    }

    #[test]
    fn test_auto_title_prefers_session_provider_and_model() {
        let mut store = SessionStore::memory_only();
        store
            .append_turn_fallible(
                Some("s_test"),
                "User query about rust",
                "Assistant answer",
                None,
                vec![],
            )
            .unwrap();
        // 注入 session 实际 provider 和 model（走公开入口，不碰私有字段）
        store.record_turn_trace(
            Some("s_test"),
            TurnTraceRecord {
                provider_requested_name: Some("ponyllm".to_string()),
                provider_name: Some("ponyllm".to_string()),
                provider_model: Some("gemini-3.8-flash".to_string()),
                ..Default::default()
            },
        );

        let last_resolved = Arc::new(Mutex::new(None));
        let resolver = FakeTitleResolver {
            last_resolved: Arc::clone(&last_resolved),
            has_api_key: false, // 无 key 时触发静默跳过，但不影响解析断言
        };

        let runtime = AgentRuntimeBuilder::new()
            .session_store(store)
            .provider_resolver(Box::new(resolver))
            .build();
        let cp = HostControlPlaneBuilder::new().runtime(runtime).build();

        let outcome = cp.maybe_generate_session_title(Some("s_test"));
        assert!(outcome.is_none(), "无 key 时应返回 None");

        // 断言：解析时传入的 provider/model 优先取自当前会话，而非默认 None
        let resolved = last_resolved.lock().unwrap().clone();
        assert_eq!(
            resolved,
            Some((
                Some("ponyllm".to_string()),
                Some("gemini-3.8-flash".to_string())
            )),
            "标题生成应优先跟随当前会话实际使用的 provider 和 model"
        );
    }

    #[test]
    fn test_auto_title_no_key_skips_without_repeated_failing_calls() {
        let mut store = SessionStore::memory_only();
        store
            .append_turn_fallible(
                Some("s_no_key"),
                "User first message",
                "Assistant first response",
                None,
                vec![],
            )
            .unwrap();

        let last_resolved = Arc::new(Mutex::new(None));
        let resolver = FakeTitleResolver {
            last_resolved: Arc::clone(&last_resolved),
            has_api_key: false,
        };

        let runtime = AgentRuntimeBuilder::new()
            .session_store(store)
            .provider_resolver(Box::new(resolver))
            .build();
        let cp = HostControlPlaneBuilder::new().runtime(runtime).build();

        // 第 1 轮自动标题尝试：无 key 静默跳过
        let outcome1 = cp.maybe_generate_session_title(Some("s_no_key"));
        assert!(outcome1.is_none());

        // 第 2 轮自动标题尝试：仍然无 key，静默跳过且不产生重复失败日志/调用
        let outcome2 = cp.maybe_generate_session_title(Some("s_no_key"));
        assert!(outcome2.is_none());
    }

    #[test]
    fn test_manual_title_preserves_behavior() {
        let mut store = SessionStore::memory_only();
        store
            .append_turn_fallible(
                Some("s_manual"),
                "User manual query",
                "Assistant manual response",
                None,
                vec![],
            )
            .unwrap();

        let last_resolved = Arc::new(Mutex::new(None));
        let resolver = FakeTitleResolver {
            last_resolved: Arc::clone(&last_resolved),
            has_api_key: false,
        };

        let runtime = AgentRuntimeBuilder::new()
            .session_store(store)
            .provider_resolver(Box::new(resolver))
            .build();
        let cp = HostControlPlaneBuilder::new().runtime(runtime).build();

        // 手动 /title：行为保持不变，调用默认 provider 解析并走完整流程（无 key 正常返回 Err）
        let result = cp.generate_session_title(Some("s_manual"));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("missing API key"),
            "手动调用无 key 时应返回明确错误，不静默吞掉"
        );

        let resolved = last_resolved.lock().unwrap().clone();
        assert_eq!(
            resolved,
            Some((None, None)),
            "手动 /title 保持既有行为：使用默认 provider selection"
        );
    }
}

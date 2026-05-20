use crate::agent::provider::{ProviderManager, ProviderMessage, ProviderRequest, ProviderRole};
use crate::agent::session::{SessionSnapshot, TurnHistoryMessage};

pub trait TurnContextBuilder: Send {
    fn build_request(
        &self,
        graph_name: &str,
        provider: &ProviderManager,
        user_message: &str,
        session: &SessionSnapshot,
    ) -> ProviderRequest;

    fn build_session_summary(
        &self,
        graph_name: &str,
        session: &SessionSnapshot,
        provider_name: &str,
        provider_mode: Option<&str>,
    ) -> String;
}

pub struct DefaultTurnContextBuilder;

impl TurnContextBuilder for DefaultTurnContextBuilder {
    fn build_request(
        &self,
        graph_name: &str,
        provider: &ProviderManager,
        user_message: &str,
        session: &SessionSnapshot,
    ) -> ProviderRequest {
        let mut messages = vec![
            ProviderMessage::system(
                "You are Pony Agent. Reply in Chinese and use tools when workspace inspection is needed.",
            ),
            ProviderMessage::developer(format!(
                "Session summary: {} / graph={} / session={}",
                session.summary,
                graph_name,
                session.conversation_id
            )),
        ];

        for message in session.history.iter().filter_map(to_provider_history_message) {
            messages.push(message);
        }
        messages.push(ProviderMessage::user(user_message.to_string()));

        ProviderRequest {
            model: provider.model().to_string(),
            input: messages,
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        }
    }

    fn build_session_summary(
        &self,
        graph_name: &str,
        session: &SessionSnapshot,
        provider_name: &str,
        provider_mode: Option<&str>,
    ) -> String {
        let focus = session
            .last_referenced_file
            .as_deref()
            .map(|path| format!(" / focus={}", path))
            .unwrap_or_default();
        format!(
            "{} / graph={} / session={} / turns={} / provider={} / mode={}{}",
            session.summary,
            graph_name,
            session.conversation_id,
            session.turn_count,
            provider_name,
            provider_mode.unwrap_or("unknown"),
            focus,
        )
    }
}

fn to_provider_history_message(message: &TurnHistoryMessage) -> Option<ProviderMessage> {
    let content = message.content.trim();
    if content.is_empty() {
        return None;
    }

    match message.role.as_str() {
        "user" => Some(ProviderMessage::user(content.to_string())),
        "assistant" => Some(ProviderMessage {
            role: ProviderRole::Developer,
            content: format!("Previous assistant reply: {}", content),
        }),
        _ => None,
    }
}

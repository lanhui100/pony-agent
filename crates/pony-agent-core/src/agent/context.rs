use crate::agent::capability_bridge::SkillDescriptor;
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::graph::GraphRun;
use crate::agent::input::TurnInputImage;
use crate::agent::provider::{
    ContextRefreshReason, ConversationCarryMode, PrefixMutationReason, ProviderManager,
    ProviderMessage, ProviderRequest, ProviderRequestObservation, ProviderRole,
};
use crate::agent::session::{
    AttachmentAsset, LongTermMemoryRecord, SessionSnapshot, TurnHistoryMessage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const BASE_SYSTEM_PROMPT: &str = r#"You are Pony Agent, an AI agent that collaborates with the user inside a shared workspace.

- Reply in Chinese unless the user explicitly requests another language.
- Understand the latest request and solve it directly.
- Verify environment-specific or changeable facts instead of guessing.
- Respect existing code, project conventions, and user changes.
- Keep progress updates concise during longer work.
- Do not use Markdown formatting in replies; output plain text only unless the user explicitly asks for Markdown.
- Be extremely concise by default to save tokens; provide detailed explanations only when requested or necessary.
- When calling a tool, always include a Chinese "description" field briefly explaining the purpose of this invocation (e.g. "读取配置文件 tauri.conf.json", "搜索 TokenManager 类"). This description is displayed to the user in the UI — without it, only a generic message appears.
- This base prompt must stay stable; environment facts, workspace instructions, memory, and temporary reminders are injected in later layers."#;
const SESSION_CONTEXT_HISTORY_LIMIT: usize = 12;
const SESSION_CONTEXT_ATTACHMENT_LIMIT: usize = 8;
const TRANSCRIPT_CONTEXT_MESSAGE_LIMIT: usize = 24;
const CODING_DOMAIN_PROFILE_PROMPT: &str = r#"Active domain profile: coding.

- Focus on code, debugging, architecture, tooling, and tests.
- Read surrounding code before changing non-trivial logic.
- Prefer minimal fixes that match existing patterns.
- Validate changes with targeted tests and builds when available.
- Keep edits scoped; do not silently fix unrelated issues."#;
const WORK_DOMAIN_PROFILE_PROMPT: &str = r#"Active domain profile: work.

- Focus on writing, analysis, planning, research, and operations.
- Gather evidence first; separate facts, inferences, and recommendations.
- Prefer structured, action-oriented outputs over technical over-detail.
- Cite or name sources when claims depend on external material.
- Keep plans phased, concrete, and easy to scan."#;
const MIN_CONTEXT_WINDOW_FOR_DOMAIN_PROFILE_TOKENS: u32 = 8_192;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnContext {
    pub user_message: String,
    #[serde(default)]
    pub images: Vec<TurnInputImage>,
    pub references_image: bool,
    pub workspace_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContext {
    pub conversation_id: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub recent_history: Vec<TurnHistoryMessage>,
    #[serde(default)]
    pub recent_attachment_assets: Vec<AttachmentAsset>,
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    pub run_id: Option<String>,
    pub goal: Option<String>,
    pub phase: Option<String>,
    pub active_turn_id: Option<String>,
    pub last_completed_turn_id: Option<String>,
    pub resume_count: Option<u32>,
    pub last_decision_summary: Option<String>,
    pub execution_checkpoint_status: Option<String>,
    pub execution_checkpoint_phase: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LongTermMemoryEntry {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LongTermMemory {
    pub status: String,
    pub summary: Option<String>,
    #[serde(default)]
    pub entries: Vec<LongTermMemoryEntry>,
}

impl Default for LongTermMemory {
    fn default() -> Self {
        Self {
            status: "empty".to_string(),
            summary: Some(
                "No long-term memory entries are stored for this session yet.".to_string(),
            ),
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptContext {
    #[serde(default)]
    pub provider_native_messages: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedContextState {
    pub turn_context: TurnContext,
    pub session_context: SessionContext,
    pub run_state: RunState,
    pub long_term_memory: LongTermMemory,
    pub transcript: TranscriptContext,
}

impl RetrievedContextState {
    pub fn planner_history(&self) -> &[TurnHistoryMessage] {
        &self.session_context.recent_history
    }
}

pub struct ContextStateQuery<'a> {
    pub user_message: &'a str,
    pub images: &'a [TurnInputImage],
    pub workspace_mode: Option<&'a str>,
    pub session: &'a SessionSnapshot,
    pub run: Option<&'a GraphRun>,
    pub checkpoint: Option<&'a ExecutionCheckpoint>,
}

pub trait ContextStateRetriever: Send {
    fn retrieve(&self, query: ContextStateQuery<'_>) -> RetrievedContextState;
}

pub struct DefaultContextStateRetriever;

pub trait TurnContextBuilder: Send + Sync {
    fn retrieve_context_state(
        &self,
        user_message: &str,
        images: &[TurnInputImage],
        workspace_mode: Option<&str>,
        session: &SessionSnapshot,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState;

    fn build_request(
        &self,
        graph_name: &str,
        provider: &ProviderManager,
        retrieved: &RetrievedContextState,
        planner_skills: &[SkillDescriptor],
    ) -> ProviderRequest;

    fn build_session_summary(
        &self,
        graph_name: &str,
        retrieved: &RetrievedContextState,
        provider_name: &str,
        provider_mode: Option<&str>,
    ) -> String;
}

pub struct DefaultTurnContextBuilder;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayeredTurnContext {
    #[serde(default)]
    pub tool_context: Vec<String>,
    #[serde(default)]
    pub base_system_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub runtime_fact_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub domain_profile_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub project_instruction_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub memory_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub conversation_carry_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub volatile_context_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub volatile_input_messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub volatile_input_observation_text: String,
    #[serde(default)]
    pub native_base_system_messages: Vec<Value>,
    #[serde(default)]
    pub native_runtime_fact_messages: Vec<Value>,
    #[serde(default)]
    pub native_domain_profile_messages: Vec<Value>,
    #[serde(default)]
    pub native_project_instruction_messages: Vec<Value>,
    #[serde(default)]
    pub native_memory_messages: Vec<Value>,
    #[serde(default)]
    pub native_conversation_carry_messages: Vec<Value>,
    #[serde(default)]
    pub native_volatile_context_messages: Vec<Value>,
    #[serde(default)]
    pub native_volatile_input_messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_refresh_reason: Option<ContextRefreshReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instruction_scope_sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_carry_mode: Option<ConversationCarryMode>,
}

struct SemistableContextSection {
    note: String,
}

impl TurnContextBuilder for DefaultTurnContextBuilder {
    fn retrieve_context_state(
        &self,
        user_message: &str,
        images: &[TurnInputImage],
        workspace_mode: Option<&str>,
        session: &SessionSnapshot,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState {
        DefaultContextStateRetriever.retrieve(ContextStateQuery {
            user_message,
            images,
            workspace_mode,
            session,
            run,
            checkpoint,
        })
    }

    fn build_request(
        &self,
        graph_name: &str,
        provider: &ProviderManager,
        retrieved: &RetrievedContextState,
        planner_skills: &[SkillDescriptor],
    ) -> ProviderRequest {
        let layered_context =
            build_layered_turn_context(graph_name, provider, retrieved, planner_skills);
        let messages = flatten_layered_input_messages(&layered_context);
        let native_messages = flatten_layered_native_messages(&layered_context);
        let observation = build_request_observation_from_layered_context(&layered_context);

        ProviderRequest {
            model: provider.model().to_string(),
            input: messages,
            images: retrieved.turn_context.images.clone(),
            native_messages,
            observation,
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        }
    }

    fn build_session_summary(
        &self,
        graph_name: &str,
        retrieved: &RetrievedContextState,
        provider_name: &str,
        provider_mode: Option<&str>,
    ) -> String {
        let focus = retrieved
            .session_context
            .last_referenced_file
            .as_deref()
            .map(|path| format!(" / focus={}", path))
            .unwrap_or_default();
        format!(
            "{} / graph={} / session={} / turns={} / provider={} / mode={}{}",
            retrieved.session_context.summary,
            graph_name,
            retrieved.session_context.conversation_id,
            retrieved.session_context.turn_count,
            provider_name,
            provider_mode.unwrap_or("unknown"),
            focus,
        )
    }
}

fn build_layered_turn_context(
    graph_name: &str,
    provider: &ProviderManager,
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
) -> LayeredTurnContext {
    let current_user_message = ProviderMessage::user(retrieved.turn_context.user_message.clone());
    let base_system_messages = vec![ProviderMessage::system(BASE_SYSTEM_PROMPT)];
    let runtime_fact_messages = vec![ProviderMessage::developer(provider_capability_note(
        provider,
    ))];
    let domain_profile_prompt = infer_domain_profile_prompt(graph_name, &retrieved.turn_context);
    let domain_profile_messages = build_domain_profile_messages(provider, domain_profile_prompt);
    let instruction_scope_sources = applicable_instruction_sources(retrieved);
    let input_budget_tokens = input_budget_tokens(provider);
    let raw_history = retrieved
        .session_context
        .recent_history
        .iter()
        .filter_map(to_provider_history_message)
        .collect::<Vec<_>>();
    let image_note = image_capability_note(provider, retrieved);
    let base_semistable_context = provider_semistable_context_note(graph_name, retrieved);
    let memory_messages = build_memory_messages(retrieved);
    let mut reserved_messages = vec![
        base_system_messages[0].clone(),
        runtime_fact_messages[0].clone(),
    ];
    reserved_messages.extend(domain_profile_messages.clone());
    reserved_messages.push(ProviderMessage::developer(
        base_semistable_context.note.clone(),
    ));
    reserved_messages.extend(memory_messages.clone());
    reserved_messages.extend(build_volatile_context_messages(
        retrieved,
        planner_skills,
        image_note.as_deref(),
        None,
    ));
    reserved_messages.push(current_user_message.clone());
    let (history_messages, history_truncated_count) =
        truncate_history_messages(raw_history, &reserved_messages, input_budget_tokens);
    let history_truncation_note = truncation_note(history_truncated_count, "history messages");
    let project_instruction_messages = vec![ProviderMessage::developer(
        base_semistable_context.note.clone(),
    )];
    let conversation_carry_messages = history_messages;
    let volatile_context_messages = build_volatile_context_messages(
        retrieved,
        planner_skills,
        image_note.as_deref(),
        history_truncation_note.as_deref(),
    );
    let volatile_input_messages = vec![current_user_message];
    let volatile_input_observation_text = render_turn_input_for_observation(
        &retrieved.turn_context.user_message,
        &retrieved.turn_context.images,
    );
    let prefix_mutation_reasons = collect_prefix_mutation_reasons(
        retrieved,
        planner_skills,
        image_note.as_deref(),
        history_truncation_note.as_deref(),
        provider.requires_provider_native_tool_flow(),
    );
    let mut native_base_system_messages = Vec::new();
    let mut native_runtime_fact_messages = Vec::new();
    let mut native_domain_profile_messages = Vec::new();
    let mut native_project_instruction_messages = Vec::new();
    let mut native_memory_messages = Vec::new();
    let mut native_conversation_carry_messages = Vec::new();
    let mut native_volatile_context_messages = Vec::new();
    let mut native_volatile_input_messages = Vec::new();
    let conversation_carry_mode = if provider.requires_provider_native_tool_flow() {
        let native_memory = build_native_memory_messages(retrieved);
        let mut reserved_native_messages = vec![
            json!({
                "role": "system",
                "content": BASE_SYSTEM_PROMPT,
            }),
            json!({
                "role": "system",
                "content": provider_capability_note(provider),
            }),
            json!({
                "role": "system",
                "content": base_semistable_context.note.clone(),
            }),
        ];
        reserved_native_messages.splice(
            2..2,
            domain_profile_messages.iter().map(|message| {
                json!({
                    "role": "system",
                    "content": message.content.clone(),
                })
            }),
        );
        reserved_native_messages.extend(native_memory.clone());
        reserved_native_messages.extend(build_native_volatile_context_messages(
            retrieved,
            planner_skills,
            image_note.as_deref(),
            None,
        ));
        reserved_native_messages.push(json!({
            "role": "user",
            "content": retrieved.turn_context.user_message.clone()
        }));
        let (native_transcript, native_truncated_count) = truncate_native_messages(
            retrieved.transcript.provider_native_messages.clone(),
            &reserved_native_messages,
            input_budget_tokens,
        );
        let native_transcript_note = truncation_note(
            native_truncated_count,
            "provider-native transcript messages",
        );
        native_base_system_messages = vec![json!({
            "role": "system",
            "content": BASE_SYSTEM_PROMPT,
        })];
        native_runtime_fact_messages = vec![json!({
            "role": "system",
            "content": provider_capability_note(provider),
        })];
        native_domain_profile_messages = domain_profile_messages
            .iter()
            .map(|message| {
                json!({
                    "role": "system",
                    "content": message.content.clone(),
                })
            })
            .collect();
        native_project_instruction_messages = vec![json!({
            "role": "system",
            "content": base_semistable_context.note.clone(),
        })];
        native_memory_messages = native_memory;
        native_conversation_carry_messages = native_transcript;
        native_volatile_context_messages = build_native_volatile_context_messages(
            retrieved,
            planner_skills,
            image_note.as_deref(),
            native_transcript_note.as_deref(),
        );
        native_volatile_input_messages = vec![json!({
            "role": "user",
            "content": if retrieved.turn_context.images.is_empty() {
                Value::String(retrieved.turn_context.user_message.clone())
            } else {
                Value::Array(openai_user_content_blocks(
                    &retrieved.turn_context.user_message,
                    &retrieved.turn_context.images,
                ))
            }
        })];
        ConversationCarryMode::ProviderNativeTranscriptReplay
    } else {
        ConversationCarryMode::FullReplay
    };

    LayeredTurnContext {
        tool_context: planner_skills
            .iter()
            .map(|skill| skill.label.clone())
            .collect(),
        base_system_messages,
        runtime_fact_messages,
        domain_profile_messages,
        project_instruction_messages,
        memory_messages,
        conversation_carry_messages,
        volatile_context_messages,
        volatile_input_messages,
        volatile_input_observation_text,
        native_base_system_messages,
        native_runtime_fact_messages,
        native_domain_profile_messages,
        native_project_instruction_messages,
        native_memory_messages,
        native_conversation_carry_messages,
        native_volatile_context_messages,
        native_volatile_input_messages,
        prefix_mutation_reasons: dedupe_prefix_mutation_reasons(prefix_mutation_reasons),
        context_refresh_reason: derive_context_refresh_reason(retrieved, planner_skills),
        instruction_scope_sources,
        conversation_carry_mode: Some(conversation_carry_mode),
    }
}

fn flatten_layered_input_messages(context: &LayeredTurnContext) -> Vec<ProviderMessage> {
    let mut messages = Vec::new();
    messages.extend(context.base_system_messages.clone());
    messages.extend(context.runtime_fact_messages.clone());
    messages.extend(context.domain_profile_messages.clone());
    messages.extend(context.project_instruction_messages.clone());
    messages.extend(context.conversation_carry_messages.clone());
    messages.extend(context.memory_messages.clone());
    messages.extend(context.volatile_context_messages.clone());
    messages.extend(context.volatile_input_messages.clone());
    messages
}

fn flatten_layered_native_messages(context: &LayeredTurnContext) -> Vec<Value> {
    let mut messages = Vec::new();
    messages.extend(context.native_base_system_messages.clone());
    messages.extend(context.native_runtime_fact_messages.clone());
    messages.extend(context.native_domain_profile_messages.clone());
    messages.extend(context.native_project_instruction_messages.clone());
    messages.extend(context.native_conversation_carry_messages.clone());
    messages.extend(context.native_memory_messages.clone());
    messages.extend(context.native_volatile_context_messages.clone());
    messages.extend(context.native_volatile_input_messages.clone());
    messages
}

fn build_request_observation_from_layered_context(
    context: &LayeredTurnContext,
) -> ProviderRequestObservation {
    let stable_prefix_text = if !context.native_base_system_messages.is_empty()
        || !context.native_runtime_fact_messages.is_empty()
        || !context.native_domain_profile_messages.is_empty()
    {
        render_native_messages_for_observation(
            &[
                context.native_base_system_messages.clone(),
                context.native_runtime_fact_messages.clone(),
                context.native_domain_profile_messages.clone(),
            ]
            .concat(),
        )
    } else {
        render_provider_messages_for_observation(
            &[
                context.base_system_messages.clone(),
                context.runtime_fact_messages.clone(),
                context.domain_profile_messages.clone(),
            ]
            .concat(),
        )
    };
    let semi_stable_context_text = if !context.native_project_instruction_messages.is_empty()
        || !context.native_conversation_carry_messages.is_empty()
    {
        join_non_empty_sections(&[
            render_native_messages_for_observation(&context.native_project_instruction_messages),
            render_native_messages_for_observation(&context.native_conversation_carry_messages),
            render_native_messages_for_observation(&context.native_memory_messages),
            render_native_messages_for_observation(&context.native_volatile_context_messages),
        ])
    } else {
        join_non_empty_sections(&[
            render_provider_messages_for_observation(&context.project_instruction_messages),
            render_provider_messages_for_observation(&context.conversation_carry_messages),
            render_provider_messages_for_observation(&context.memory_messages),
            render_provider_messages_for_observation(&context.volatile_context_messages),
        ])
    };
    let volatile_input_text = if !context.native_volatile_input_messages.is_empty() {
        render_native_messages_for_observation(&context.native_volatile_input_messages)
    } else {
        context.volatile_input_observation_text.clone()
    };

    ProviderRequestObservation {
        stable_prefix_text,
        semi_stable_context_text,
        volatile_input_text,
        prefix_mutation_reasons: context.prefix_mutation_reasons.clone(),
        context_refresh_reason: context.context_refresh_reason.clone(),
        instruction_scope_sources: context.instruction_scope_sources.clone(),
        conversation_carry_mode: context.conversation_carry_mode.clone(),
    }
}

fn infer_domain_profile_prompt(_graph_name: &str, turn_context: &TurnContext) -> &'static str {
    if let Some(workspace_mode) = turn_context.workspace_mode.as_deref() {
        match workspace_mode {
            "work" => return WORK_DOMAIN_PROFILE_PROMPT,
            "coding" => return CODING_DOMAIN_PROFILE_PROMPT,
            other => {
                eprintln!(
                    "[pony-agent][context] unknown workspace_mode={other}; falling back to coding profile"
                );
            }
        }
    }

    CODING_DOMAIN_PROFILE_PROMPT
}

fn build_domain_profile_messages(
    provider: &ProviderManager,
    domain_profile_prompt: &'static str,
) -> Vec<ProviderMessage> {
    if matches!(
        provider.context_window_tokens(),
        Some(tokens) if tokens < MIN_CONTEXT_WINDOW_FOR_DOMAIN_PROFILE_TOKENS
    ) {
        return Vec::new();
    }

    vec![ProviderMessage::developer(domain_profile_prompt)]
}

impl ContextStateRetriever for DefaultContextStateRetriever {
    fn retrieve(&self, query: ContextStateQuery<'_>) -> RetrievedContextState {
        let recent_history =
            recent_history_slice(&query.session.history, SESSION_CONTEXT_HISTORY_LIMIT);
        let recent_attachment_assets = recent_attachment_assets(
            &query.session.attachment_assets,
            SESSION_CONTEXT_ATTACHMENT_LIMIT,
        );
        let transcript = TranscriptContext {
            provider_native_messages: recent_transcript_slice(
                &query.session.provider_native_transcript,
                TRANSCRIPT_CONTEXT_MESSAGE_LIMIT,
            ),
        };
        let references_image = references_image_in_context(
            query.user_message,
            &recent_history,
            query.session.last_referenced_file.as_deref(),
        );

        RetrievedContextState {
            turn_context: TurnContext {
                user_message: query.user_message.to_string(),
                images: query.images.to_vec(),
                references_image,
                workspace_mode: query.workspace_mode.map(str::to_string),
            },
            session_context: SessionContext {
                conversation_id: query.session.conversation_id.clone(),
                title: query.session.title.clone(),
                summary: query.session.summary.clone(),
                recent_history,
                recent_attachment_assets,
                turn_count: query.session.turn_count,
                last_referenced_file: query.session.last_referenced_file.clone(),
            },
            run_state: build_run_state(query.run, query.checkpoint),
            long_term_memory: build_long_term_memory(&query.session.long_term_memory_entries),
            transcript,
        }
    }
}

fn openai_user_content_blocks(user_message: &str, images: &[TurnInputImage]) -> Vec<Value> {
    let mut blocks = Vec::new();
    if !user_message.trim().is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": user_message
        }));
    }

    blocks.extend(images.iter().map(|image| {
        json!({
            "type": "image_url",
            "image_url": {
                "url": image.data_url
            }
        })
    }));
    blocks
}

fn provider_capability_note(provider: &ProviderManager) -> String {
    format!(
        "Capability profile: contextWindowTokens={} / supportsImageInput={}. Only rely on the visible request context.",
        provider
            .context_window_tokens()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        provider.supports_image_input()
    )
}

fn provider_semistable_context_note(
    graph_name: &str,
    retrieved: &RetrievedContextState,
) -> SemistableContextSection {
    SemistableContextSection {
        note: format!(
            "Session label: graph={} / session={}",
            graph_name, retrieved.session_context.conversation_id
        ),
    }
}

fn build_volatile_context_messages(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
    image_note: Option<&str>,
    truncation_note: Option<&str>,
) -> Vec<ProviderMessage> {
    let note = volatile_context_note(retrieved, planner_skills, image_note, truncation_note);
    if note.is_empty() {
        Vec::new()
    } else {
        vec![ProviderMessage::developer(note)]
    }
}

fn build_native_volatile_context_messages(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
    image_note: Option<&str>,
    truncation_note: Option<&str>,
) -> Vec<Value> {
    let note = volatile_context_note(retrieved, planner_skills, image_note, truncation_note);
    if note.is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "role": "system",
            "content": note,
        })]
    }
}

fn volatile_context_note(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
    image_note: Option<&str>,
    truncation_note: Option<&str>,
) -> String {
    let mut notes = Vec::new();

    if let Some(path) = retrieved.session_context.last_referenced_file.as_deref() {
        notes.push(format!("Focus file: {}.", path));
    }

    if let Some(goal) = retrieved.run_state.goal.as_deref() {
        notes.push(format!("Run goal: {}.", goal));
    }

    if !planner_skills.is_empty() {
        notes.push(render_planner_skills_note(planner_skills));
    }

    if let Some(note) = image_note {
        notes.push(note.to_string());
    }

    if let Some(note) = truncation_note {
        notes.push(note.to_string());
    }

    notes.join(" ")
}

fn build_memory_messages(retrieved: &RetrievedContextState) -> Vec<ProviderMessage> {
    if retrieved.long_term_memory.entries.is_empty() {
        return Vec::new();
    }

    let summary = retrieved
        .long_term_memory
        .summary
        .as_deref()
        .unwrap_or("Long-term memory entries are attached to this retrieval snapshot.");
    vec![ProviderMessage::developer(format!(
        "Long-term memory status: {}. {}",
        retrieved.long_term_memory.status, summary
    ))]
}

fn build_native_memory_messages(retrieved: &RetrievedContextState) -> Vec<Value> {
    if retrieved.long_term_memory.entries.is_empty() {
        return Vec::new();
    }

    let summary = retrieved
        .long_term_memory
        .summary
        .as_deref()
        .unwrap_or("Long-term memory entries are attached to this retrieval snapshot.");
    vec![json!({
        "role": "system",
        "content": format!(
            "Long-term memory status: {}. {}",
            retrieved.long_term_memory.status, summary
        )
    })]
}

fn derive_context_refresh_reason(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
) -> Option<ContextRefreshReason> {
    if retrieved.session_context.turn_count <= 1 {
        return Some(ContextRefreshReason::InitialBuild);
    }

    if !planner_skills.is_empty() {
        return Some(ContextRefreshReason::PlannerSkillsChanged);
    }

    if !retrieved.long_term_memory.entries.is_empty() {
        return Some(ContextRefreshReason::LongTermMemoryChanged);
    }

    if retrieved.run_state.goal.is_some() {
        return Some(ContextRefreshReason::RunGoalChanged);
    }

    None
}

fn applicable_instruction_sources(retrieved: &RetrievedContextState) -> Vec<String> {
    let mut sources = vec!["thread://base-system".to_string()];
    if let Some(path) = retrieved.session_context.last_referenced_file.as_deref() {
        sources.push(format!("workspace://{}", normalize_instruction_path(path)));
    }
    dedupe_instruction_sources(sources)
}

fn normalize_instruction_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn dedupe_instruction_sources(sources: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for source in sources {
        if !deduped.contains(&source) {
            deduped.push(source);
        }
    }
    deduped
}

fn collect_prefix_mutation_reasons(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
    image_note: Option<&str>,
    history_truncation_note: Option<&str>,
    provider_native_tool_flow: bool,
) -> Vec<PrefixMutationReason> {
    let _ = (
        retrieved,
        planner_skills,
        image_note,
        history_truncation_note,
        provider_native_tool_flow,
    );
    Vec::new()
}

fn render_planner_skills_note(planner_skills: &[SkillDescriptor]) -> String {
    let preview = planner_skills
        .iter()
        .take(6)
        .map(|skill| {
            format!(
                "{} [{}] approval={} host_mediated={} scope={} kinds={}",
                skill.label,
                skill.input_schema_summary,
                skill.requires_approval,
                skill.host_mediated,
                skill.permission_scope,
                skill
                    .composed_capability_kinds
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect::<Vec<_>>()
                    .join("+")
            )
        })
        .collect::<Vec<_>>()
        .join(" ; ");
    if planner_skills.len() > 6 {
        format!(
            "Available executable skills (showing 6/{}): {}.",
            planner_skills.len(),
            preview
        )
    } else {
        format!("Available executable skills: {}.", preview)
    }
}

fn dedupe_prefix_mutation_reasons(reasons: Vec<PrefixMutationReason>) -> Vec<PrefixMutationReason> {
    let mut deduped = Vec::new();
    for reason in reasons {
        if !deduped.contains(&reason) {
            deduped.push(reason);
        }
    }
    deduped
}

fn image_capability_note(
    provider: &ProviderManager,
    retrieved: &RetrievedContextState,
) -> Option<String> {
    let references_image = retrieved.turn_context.references_image;
    let has_recent_image_attachments = retrieved
        .session_context
        .recent_history
        .iter()
        .rev()
        .take(4)
        .any(|message| !message.attachments.is_empty());

    if provider.supports_image_input() {
        if references_image {
            return Some(if has_recent_image_attachments {
                "The user appears to reference image content. This model is image-capable, and the runtime may reattach recent session images for this turn. Only claim visual inspection when actual image payloads are present in the request."
                    .to_string()
            } else {
                "The user appears to reference image content. This model is marked as image-capable, but this runtime only sends text unless image payloads are explicitly attached upstream. Do not claim to see pixels from a filename, path, or screenshot mention alone."
                    .to_string()
            });
        }

        return Some(
            "This model is marked as image-capable. Only claim visual inspection when actual image content is present in the request; otherwise treat image mentions as text-only references."
                .to_string(),
        );
    }

    if references_image {
        return Some(
            "The user appears to reference image content, but supportsImageInput=false for this model. Do not pretend to inspect screenshots or image files; ask for OCR, textual details, or an image-capable model when visual inspection is required."
                .to_string(),
        );
    }

    Some(
        "supportsImageInput=false for this model. If image inspection becomes necessary, ask for OCR or textual details instead of claiming visual access."
            .to_string(),
    )
}

fn render_provider_messages_for_observation(messages: &[ProviderMessage]) -> String {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            format!(
                "[{}] {}\n{}",
                index,
                match message.role {
                    ProviderRole::System => "system",
                    ProviderRole::Developer => "developer",
                    ProviderRole::User => "user",
                },
                message.content.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_native_messages_for_observation(messages: &[Value]) -> String {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let content = match message.get("content") {
                Some(Value::String(text)) => text.trim().to_string(),
                Some(other) => {
                    serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string())
                }
                None => message.to_string(),
            };
            format!("[{}] {}\n{}", index, role, content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_turn_input_for_observation(user_message: &str, images: &[TurnInputImage]) -> String {
    let mut sections = vec![format!("[0] user\n{}", user_message.trim())];

    if !images.is_empty() {
        sections.push(format!(
            "[images]\n{}",
            images
                .iter()
                .enumerate()
                .map(|(index, image)| {
                    format!(
                        "[{}] mimeType={} / name={} / dataUrlPresent={}",
                        index,
                        image.mime_type,
                        image.name.as_deref().unwrap_or("unnamed"),
                        !image.data_url.is_empty()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    sections.join("\n\n")
}

fn join_non_empty_sections(sections: &[String]) -> String {
    sections
        .iter()
        .filter_map(|section| {
            let trimmed = section.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn references_image_in_context(
    user_message: &str,
    history: &[TurnHistoryMessage],
    last_referenced_file: Option<&str>,
) -> bool {
    text_mentions_image(user_message)
        || history
            .iter()
            .rev()
            .take(4)
            .any(|message| !message.attachments.is_empty())
        || last_referenced_file
            .map(path_looks_like_image)
            .unwrap_or(false)
        || history
            .iter()
            .rev()
            .take(4)
            .any(|message| text_mentions_image(&message.content))
}

fn build_run_state(run: Option<&GraphRun>, checkpoint: Option<&ExecutionCheckpoint>) -> RunState {
    RunState {
        run_id: run.map(|item| item.id.clone()),
        goal: run.map(|item| item.goal.clone()),
        phase: run.map(|item| graph_run_phase_label(item)),
        active_turn_id: run.and_then(|item| item.active_turn_id.clone()),
        last_completed_turn_id: run.and_then(|item| item.last_completed_turn_id.clone()),
        resume_count: run.map(|item| item.resume_count),
        last_decision_summary: run
            .and_then(|item| item.last_decision.as_ref())
            .map(|item| item.summary.clone()),
        execution_checkpoint_status: checkpoint.map(|item| item.status.clone()),
        execution_checkpoint_phase: checkpoint.map(|item| item.phase.clone()),
    }
}

fn build_long_term_memory(entries: &[LongTermMemoryRecord]) -> LongTermMemory {
    if entries.is_empty() {
        return LongTermMemory::default();
    }

    let mut sorted_entries = entries.to_vec();
    // Keep prompt-adjacent memory facts deterministic so retrieval order does not drift with writes.
    sorted_entries.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.content.cmp(&right.content))
            .then_with(|| left.source.cmp(&right.source))
    });

    LongTermMemory {
        status: "available".to_string(),
        summary: Some(format!(
            "Retrieved {} long-term memory entr{} for this session.",
            sorted_entries.len(),
            if sorted_entries.len() == 1 {
                "y"
            } else {
                "ies"
            }
        )),
        entries: sorted_entries
            .iter()
            .map(|entry| LongTermMemoryEntry {
                kind: entry.kind.clone(),
                content: entry.content.clone(),
                source: entry.source.clone(),
                updated_at_ms: entry.updated_at_ms,
            })
            .collect(),
    }
}

fn graph_run_phase_label(run: &GraphRun) -> String {
    match run.phase {
        crate::agent::graph::GraphRunPhase::Ready => "ready",
        crate::agent::graph::GraphRunPhase::Running => "running",
        crate::agent::graph::GraphRunPhase::WaitingUser => "waiting_user",
        crate::agent::graph::GraphRunPhase::Paused => "paused",
        crate::agent::graph::GraphRunPhase::Completed => "completed",
        crate::agent::graph::GraphRunPhase::Failed => "failed",
        crate::agent::graph::GraphRunPhase::Cancelled => "cancelled",
    }
    .to_string()
}

fn recent_history_slice(history: &[TurnHistoryMessage], limit: usize) -> Vec<TurnHistoryMessage> {
    let start = history.len().saturating_sub(limit);
    trim_turn_history_to_turn_boundary(history[start..].to_vec())
}

fn trim_turn_history_to_turn_boundary(
    mut history: Vec<TurnHistoryMessage>,
) -> Vec<TurnHistoryMessage> {
    match history.iter().position(|message| message.role == "user") {
        Some(0) => history,
        Some(index) => {
            history.drain(0..index);
            history
        }
        None => Vec::new(),
    }
}

fn recent_attachment_assets(assets: &[AttachmentAsset], limit: usize) -> Vec<AttachmentAsset> {
    let mut recent = assets.to_vec();
    recent.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
    recent.truncate(limit);
    recent
}

fn recent_transcript_slice(transcript: &[Value], limit: usize) -> Vec<Value> {
    let start = transcript.len().saturating_sub(limit);
    trim_native_to_turn_boundary(transcript[start..].to_vec())
}

fn text_mentions_image(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let keywords = [
        "image",
        "screenshot",
        "photo",
        "picture",
        "vision",
        "ocr",
        ".png",
        ".jpg",
        ".jpeg",
        ".gif",
        ".webp",
        ".bmp",
        ".svg",
        "图片",
        "截图",
        "照片",
        "看图",
        "识图",
    ];

    keywords.iter().any(|keyword| lower.contains(keyword))
}

fn path_looks_like_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn input_budget_tokens(provider: &ProviderManager) -> Option<usize> {
    let context_window = provider.context_window_tokens()?;
    let output_reserve = provider
        .max_output_tokens()
        .min(context_window.saturating_sub(32));
    let safety_margin = (context_window / 20).clamp(32, 2048);
    let input_budget = context_window
        .saturating_sub(output_reserve)
        .saturating_sub(safety_margin)
        .max(32);
    Some(input_budget as usize)
}

fn truncate_history_messages(
    history: Vec<ProviderMessage>,
    reserved_messages: &[ProviderMessage],
    input_budget_tokens: Option<usize>,
) -> (Vec<ProviderMessage>, usize) {
    let Some(input_budget_tokens) = input_budget_tokens else {
        return (history, 0);
    };
    let reserved_tokens = reserved_messages
        .iter()
        .map(estimate_provider_message_tokens)
        .sum::<usize>();

    if reserved_tokens >= input_budget_tokens {
        return (Vec::new(), history.len());
    }

    let original_len = history.len();
    let mut remaining_budget = input_budget_tokens - reserved_tokens;
    let mut kept = Vec::new();

    for message in history.into_iter().rev() {
        let message_tokens = estimate_provider_message_tokens(&message);
        if message_tokens <= remaining_budget {
            remaining_budget -= message_tokens;
            kept.push(message);
        }
    }

    kept.reverse();
    let kept = trim_history_to_turn_boundary(kept);
    let truncated_count = original_len.saturating_sub(kept.len());

    (kept, truncated_count)
}

fn truncate_native_messages(
    transcript: Vec<Value>,
    reserved_messages: &[Value],
    input_budget_tokens: Option<usize>,
) -> (Vec<Value>, usize) {
    let Some(input_budget_tokens) = input_budget_tokens else {
        return (transcript, 0);
    };
    let reserved_tokens = reserved_messages
        .iter()
        .map(estimate_native_message_tokens)
        .sum::<usize>();

    if reserved_tokens >= input_budget_tokens {
        return (Vec::new(), transcript.len());
    }

    let transcript = trim_native_to_turn_boundary(transcript);
    let original_len = transcript.len();
    let mut remaining_budget = input_budget_tokens - reserved_tokens;
    let mut kept_turns = Vec::new();

    for turn in split_native_turns(transcript).into_iter().rev() {
        let turn_tokens = estimate_native_turn_tokens(&turn);
        if turn_tokens > remaining_budget {
            break;
        }

        remaining_budget -= turn_tokens;
        kept_turns.push(turn);
    }

    kept_turns.reverse();
    let kept = kept_turns.into_iter().flatten().collect::<Vec<_>>();
    let truncated_count = original_len.saturating_sub(kept.len());

    (kept, truncated_count)
}

fn trim_history_to_turn_boundary(mut history: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    match history
        .iter()
        .position(|message| matches!(message.role, ProviderRole::User))
    {
        Some(0) => history,
        Some(index) => {
            history.drain(0..index);
            history
        }
        None => Vec::new(),
    }
}

fn trim_native_to_turn_boundary(mut transcript: Vec<Value>) -> Vec<Value> {
    match transcript.iter().position(is_native_user_message) {
        Some(0) => transcript,
        Some(index) => {
            transcript.drain(0..index);
            transcript
        }
        None => Vec::new(),
    }
}

fn split_native_turns(transcript: Vec<Value>) -> Vec<Vec<Value>> {
    let mut turns = Vec::new();
    let mut current_turn = Vec::new();

    for message in transcript {
        if is_native_user_message(&message) {
            if !current_turn.is_empty() {
                turns.push(current_turn);
            }
            current_turn = vec![message];
            continue;
        }

        if !current_turn.is_empty() {
            current_turn.push(message);
        }
    }

    if !current_turn.is_empty() {
        turns.push(current_turn);
    }

    turns
}

fn estimate_native_turn_tokens(turn: &[Value]) -> usize {
    turn.iter().map(estimate_native_message_tokens).sum()
}

fn is_native_user_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
}

fn truncation_note(truncated_count: usize, label: &str) -> Option<String> {
    if truncated_count == 0 {
        None
    } else {
        Some(format!(
            "Older context was truncated to fit the provider window; {} earlier {} were omitted. Ask for a recap instead of assuming missing details.",
            truncated_count, label
        ))
    }
}

fn estimate_provider_message_tokens(message: &ProviderMessage) -> usize {
    4 + estimate_text_tokens(&message.content)
}

fn estimate_native_message_tokens(message: &Value) -> usize {
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        return 4 + estimate_text_tokens(content);
    }

    if let Some(content) = message.get("content").and_then(Value::as_array) {
        let content_tokens = content
            .iter()
            .map(estimate_native_content_block_tokens)
            .sum::<usize>();
        return 4 + content_tokens;
    }

    4 + estimate_text_tokens(&message.to_string())
}

fn estimate_native_content_block_tokens(block: &Value) -> usize {
    if block.get("type").and_then(Value::as_str) == Some("image")
        || block.get("type").and_then(Value::as_str) == Some("image_url")
    {
        return 256;
    }

    if let Some(text) = block.get("text").and_then(Value::as_str) {
        return estimate_text_tokens(text);
    }

    if let Some(text) = block
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
    {
        return estimate_text_tokens(text);
    }

    estimate_text_tokens(&block.to_string())
}

fn estimate_text_tokens(text: &str) -> usize {
    let char_count = text.chars().count();
    if char_count == 0 {
        0
    } else {
        char_count.div_ceil(4)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::capability_bridge::{CapabilityKind, SkillDescriptor, SkillSourceKind};
    use crate::agent::config::{
        ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
    };
    use crate::agent::graph::{
        GraphDecision, GraphDecisionKind, GraphDecisionReason, GraphRun, GraphRunPhase,
    };
    use crate::agent::provider::ProviderAuthType;
    use crate::agent::provider::ProviderProtocol;

    fn provider_manager(
        model: &str,
        max_output_tokens: u32,
        capabilities: ProviderModelCapabilities,
    ) -> ProviderManager {
        ProviderManager::new(ResolvedProviderSelection {
            requested_name: "test-provider".to_string(),
            provider_name: "test-provider".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://example.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "TEST_PROVIDER_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: model.to_string(),
            temperature: 0.2,
            max_output_tokens,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities,
            thinking_param_pattern: ThinkingParamPattern::None,
        })
    }

    fn sample_planner_skill(label: &str) -> SkillDescriptor {
        SkillDescriptor {
            skill_id: format!("skill:{label}"),
            source_id: "host-skills".to_string(),
            source_kind: SkillSourceKind::Host,
            label: label.to_string(),
            description: "demo skill".to_string(),
            input_schema_summary: "{}".to_string(),
            safety_class: "skill".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["skill".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
            composed_capability_refs: vec!["builtin:time_now".to_string()],
            composed_capability_kinds: vec![CapabilityKind::Tool],
            executable_in_v1: true,
        }
    }

    fn session_snapshot(
        history: Vec<TurnHistoryMessage>,
        provider_native_transcript: Vec<Value>,
        last_referenced_file: Option<&str>,
    ) -> SessionSnapshot {
        SessionSnapshot {
            conversation_id: "session-1".to_string(),
            title: "New Chat".to_string(),
            summary: "Investigating provider behavior".to_string(),
            history,
            attachment_assets: Vec::new(),
            provider_native_transcript,
            turn_trace_history: Vec::new(),
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            history_state_audit_summary: crate::agent::session::HistoryStateAuditSummary::default(),
            run_control_audit_summary:
                crate::agent::session::build_missing_run_control_audit_summary(),
            turn_count: 3,
            last_referenced_file: last_referenced_file.map(str::to_string),
            updated_at_ms: 0,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: Default::default(),
            resolved_node_id: None,
            latest_node_id: None,
        }
    }

    fn sample_run() -> GraphRun {
        GraphRun {
            id: "run-1".to_string(),
            goal: "逐步梳理 context/state 边界".to_string(),
            session_id: Some("session-1".to_string()),
            phase: GraphRunPhase::WaitingUser,
            active_turn_id: Some("turn-active".to_string()),
            last_completed_turn_id: Some("turn-0".to_string()),
            steps: Vec::new(),
            stop_reason: None,
            last_handoff: None,
            resume_count: 2,
            last_decision: Some(GraphDecision {
                kind: GraphDecisionKind::WaitUser,
                reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                summary: "等待用户继续输入".to_string(),
                target_phase: GraphRunPhase::WaitingUser,
            }),
            control_boundary_evidence: Vec::new(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn sample_checkpoint() -> ExecutionCheckpoint {
        ExecutionCheckpoint {
            contract_version: "execution-checkpoint-v1".to_string(),
            turn_id: "turn-active".to_string(),
            session_id: Some("session-1".to_string()),
            run_id: None,
            checkpoint_kind: "runtime_control".to_string(),
            recovery_mode: "replay_required".to_string(),
            projected_runtime_phase: "ready".to_string(),
            submission_command: None,
            resumable: false,
            replayable: false,
            status: "completed".to_string(),
            phase: "ready".to_string(),
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            completed_hops: 0,
            max_hops: 8,
            active_tool_name: None,
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
            persisted_effect_evidence: Vec::new(),
            error: None,
            started_at_ms: 0,
            updated_at_ms: 0,
            stop_requested_at_ms: None,
        }
    }

    #[test]
    fn retrieve_context_state_separates_turn_session_run_and_memory_layers() {
        let builder = DefaultTurnContextBuilder;
        let session = session_snapshot(
            vec![
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "orphan assistant".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "最近一次用户问题".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "最近一次助手回答".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            vec![
                json!({ "role": "assistant", "content": "ignored orphan assistant" }),
                json!({ "role": "user", "content": "recent native user" }),
                json!({ "role": "assistant", "content": "recent native assistant" }),
            ],
            Some("artifacts/diagram.png"),
        );

        let retrieved = builder.retrieve_context_state(
            "请继续看这张图",
            &[TurnInputImage {
                data_url: "data:image/png;base64,AAAA".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("diagram.png".to_string()),
            }],
            None,
            &session,
            Some(&sample_run()),
            Some(&sample_checkpoint()),
        );

        assert_eq!(retrieved.turn_context.user_message, "请继续看这张图");
        assert!(retrieved.turn_context.references_image);
        assert_eq!(retrieved.session_context.recent_history.len(), 2);
        assert_eq!(retrieved.session_context.recent_history[0].role, "user");
        assert_eq!(retrieved.run_state.run_id.as_deref(), Some("run-1"));
        assert_eq!(retrieved.run_state.phase.as_deref(), Some("waiting_user"));
        assert_eq!(
            retrieved.run_state.execution_checkpoint_status.as_deref(),
            Some("completed")
        );
        assert_eq!(retrieved.long_term_memory.status, "empty");
        assert_eq!(retrieved.transcript.provider_native_messages.len(), 2);
    }

    #[test]
    fn retrieve_context_state_reads_long_term_memory_from_session_snapshot() {
        let builder = DefaultTurnContextBuilder;
        let mut session = session_snapshot(Vec::new(), Vec::new(), None);
        session.long_term_memory_entries = vec![
            LongTermMemoryRecord {
                kind: "user_preference".to_string(),
                content: "Reply in Chinese.".to_string(),
                source: "explicit_user_message".to_string(),
                updated_at_ms: 10,
            },
            LongTermMemoryRecord {
                kind: "project_fact".to_string(),
                content: "PA-018 is a prerequisite for downstream task execution.".to_string(),
                source: "explicit_user_message".to_string(),
                updated_at_ms: 20,
            },
        ];

        let retrieved = builder.retrieve_context_state("continue", &[], None, &session, None, None);

        assert_eq!(retrieved.long_term_memory.status, "available");
        assert_eq!(retrieved.long_term_memory.entries.len(), 2);
        assert!(retrieved
            .long_term_memory
            .entries
            .iter()
            .any(
                |entry| entry.kind == "user_preference" && entry.source == "explicit_user_message"
            ));
    }

    #[test]
    fn build_request_records_layered_observation_for_normalized_input() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let mut session = session_snapshot(
            vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "recent history question".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "recent history answer".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            Vec::new(),
            Some("artifacts/diagram.png"),
        );
        session.long_term_memory_entries = vec![LongTermMemoryRecord {
            kind: "user_preference".to_string(),
            content: "Reply in Chinese.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms: 10,
        }];

        let retrieved = builder.retrieve_context_state(
            "Please inspect the latest diagram screenshot",
            &[TurnInputImage {
                data_url: "data:image/png;base64,AAAA".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("diagram.png".to_string()),
            }],
            None,
            &session,
            Some(&sample_run()),
            Some(&sample_checkpoint()),
        );
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        assert!(request
            .observation
            .stable_prefix_text
            .contains(BASE_SYSTEM_PROMPT));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Do not use Markdown formatting in replies"));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Be extremely concise by default"));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Capability profile:"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Session label: graph=graph-a / session="));
        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Investigating provider behavior"));
        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Please inspect the latest diagram screenshot"));

        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Run goal:"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Long-term memory status: available."));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("recent history question"));
        assert!(request
            .observation
            .instruction_scope_sources
            .iter()
            .any(|source| source.starts_with("thread://")));
        assert!(request.observation.prefix_mutation_reasons.is_empty());

        assert!(request
            .observation
            .volatile_input_text
            .contains("Please inspect the latest diagram screenshot"));
        assert!(request
            .observation
            .volatile_input_text
            .contains("name=diagram.png"));
        assert!(!request
            .observation
            .volatile_input_text
            .contains("recent history question"));
    }

    #[test]
    fn build_request_records_layered_observation_for_native_transcript() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            Vec::new(),
            vec![
                json!({ "role": "user", "content": "native user context" }),
                json!({ "role": "assistant", "content": "native assistant context" }),
            ],
            None,
        );

        let retrieved = builder.retrieve_context_state(
            "current volatile request",
            &[],
            None,
            &session,
            Some(&sample_run()),
            Some(&sample_checkpoint()),
        );
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        assert!(request
            .observation
            .stable_prefix_text
            .contains("supportsImageInput=false"));
        assert!(!request
            .observation
            .stable_prefix_text
            .contains("current volatile request"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("native user context"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("native assistant context"));
        assert!(request.observation.conversation_carry_mode.is_some());
        assert!(request
            .observation
            .volatile_input_text
            .contains("current volatile request"));
        assert!(!request
            .observation
            .semi_stable_context_text
            .contains("current volatile request"));
        assert!(request.observation.prefix_mutation_reasons.is_empty());
    }

    #[test]
    fn build_request_native_observation_keeps_truncation_note_when_transcript_is_trimmed() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            32,
            ProviderModelCapabilities {
                context_window_tokens: Some(64),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            Vec::new(),
            vec![
                json!({ "role": "user", "content": "native user context ".repeat(40) }),
                json!({ "role": "assistant", "content": "native assistant context ".repeat(40) }),
            ],
            None,
        );

        let retrieved = builder.retrieve_context_state(
            "current volatile request",
            &[],
            None,
            &session,
            None,
            None,
        );
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        assert!(request
            .observation
            .semi_stable_context_text
            .contains("provider-native transcript messages"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Older context was truncated to fit the provider window"));
        assert!(request.observation.prefix_mutation_reasons.is_empty());
    }

    #[test]
    fn build_request_places_memory_layers_in_both_normalized_and_native_requests() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let mut session = session_snapshot(Vec::new(), Vec::new(), None);
        session.long_term_memory_entries = vec![LongTermMemoryRecord {
            kind: "user_preference".to_string(),
            content: "Reply in Chinese.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms: 10,
        }];
        session.provider_native_transcript = vec![json!({
            "role": "user",
            "content": "native user"
        })];

        let retrieved = builder.retrieve_context_state("continue", &[], None, &session, None, None);
        let normalized_request = builder.build_request("graph-a", &provider, &retrieved, &[]);
        assert!(normalized_request
            .observation
            .semi_stable_context_text
            .contains("Long-term memory status: available."));

        let reasoning_provider = provider_manager(
            "gpt-5.4",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let native_request = builder.build_request("graph-a", &reasoning_provider, &retrieved, &[]);
        assert!(
            !native_request.native_messages.is_empty(),
            "native request should keep memory layer in native messages"
        );
        assert!(serde_json::to_string(&native_request.native_messages)
            .expect("serialize native messages")
            .contains("Long-term memory status: available."));
    }

    #[test]
    fn build_request_refresh_reason_is_none_for_plain_followup_without_special_state() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            vec![TurnHistoryMessage {
                role: "user".to_string(),
                content: "previous turn".to_string(),
                attachments: Vec::new(),
                ..Default::default()
            }],
            Vec::new(),
            None,
        );
        let retrieved = builder.retrieve_context_state("continue", &[], None, &session, None, None);
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        assert!(request.observation.context_refresh_reason.is_none());
    }

    #[test]
    fn build_request_keeps_image_and_truncation_notes_out_of_stable_prefix() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            32,
            ProviderModelCapabilities {
                context_window_tokens: Some(260),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "old turn ".repeat(60),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "old assistant ".repeat(60),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            Vec::new(),
            Some("artifacts/screenshot.png"),
        );

        let retrieved = builder.retrieve_context_state(
            "Please inspect this screenshot",
            &[],
            None,
            &session,
            None,
            None,
        );
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Do not pretend to inspect screenshots"));
        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Older context was truncated to fit the provider window"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Do not pretend to inspect screenshots"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Older context was truncated to fit the provider window"));
        assert!(request.observation.prefix_mutation_reasons.is_empty());
    }

    #[test]
    fn build_request_truncates_older_history_to_context_window() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            32,
            ProviderModelCapabilities {
                context_window_tokens: Some(1700),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "old turn ".repeat(60),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "old assistant ".repeat(60),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "recent question about provider behavior".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "recent assistant answer with concise summary".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            Vec::new(),
            None,
        );

        let retrieved = builder.retrieve_context_state("continue", &[], None, &session, None, None);
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);
        let joined = request
            .input
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Active domain profile: coding."));
        assert!(joined.contains("recent question about provider behavior"));
        assert!(joined.contains("old turn") || joined.contains("Older context was truncated"));
    }

    #[test]
    fn build_request_adds_image_limitation_for_non_vision_model() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let session = session_snapshot(Vec::new(), Vec::new(), Some("artifacts/screenshot.png"));

        let retrieved = builder.retrieve_context_state(
            "Please inspect this screenshot and explain the error",
            &[],
            None,
            &session,
            None,
            None,
        );
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);
        let developer_text = request
            .input
            .iter()
            .find(|message| matches!(message.role, ProviderRole::Developer))
            .map(|message| message.content.clone())
            .expect("developer message should exist");

        assert!(developer_text.contains("supportsImageInput=false"));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Do not pretend to inspect screenshots"));
    }

    #[test]
    fn build_request_truncates_native_transcript_for_reasoning_provider() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            Vec::new(),
            vec![
                json!({ "role": "user", "content": "old user ".repeat(60) }),
                json!({ "role": "assistant", "content": "old assistant ".repeat(60) }),
                json!({ "role": "user", "content": "recent user asks for summary" }),
                json!({ "role": "assistant", "content": "recent assistant summary" }),
            ],
            None,
        );

        let retrieved = builder.retrieve_context_state("continue", &[], None, &session, None, None);
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);
        let serialized = serde_json::to_string(&request.native_messages)
            .expect("native messages should serialize");

        assert!(serialized.contains("recent user asks for summary"));
        assert!(
            serialized.contains("old user") || serialized.contains("Older context was truncated")
        );
    }

    #[test]
    fn build_request_keeps_native_tool_roundtrip_on_turn_boundary() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            Vec::new(),
            vec![
                json!({ "role": "user", "content": "old user asks for capabilities" }),
                json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [
                        {
                            "id": "call_tool",
                            "type": "function",
                            "function": {
                                "name": "workspace_path_info",
                                "arguments": "{\"path\":\"capabilities\"}"
                            }
                        }
                    ]
                }),
                json!({
                    "role": "tool",
                    "tool_call_id": "call_tool",
                    "content": "large tool result ".repeat(120)
                }),
                json!({ "role": "assistant", "content": "old assistant follow-up" }),
                json!({ "role": "user", "content": "recent user asks for summary" }),
                json!({ "role": "assistant", "content": "recent assistant summary" }),
            ],
            None,
        );

        let retrieved = builder.retrieve_context_state("continue", &[], None, &session, None, None);
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);
        let serialized = serde_json::to_string(&request.native_messages)
            .expect("native messages should serialize");

        assert!(serialized.contains("recent user asks for summary"));
        assert!(serialized.contains("recent assistant summary"));
    }

    #[test]
    fn build_request_skips_domain_profile_for_small_context_window() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(4096),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(Vec::new(), Vec::new(), None);

        let retrieved = builder.retrieve_context_state(
            "Please write a planning memo for the next milestone",
            &[],
            Some("work"),
            &session,
            None,
            None,
        );
        let request = builder.build_request("research-workflow", &provider, &retrieved, &[]);

        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Active domain profile: work."));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Reply in Chinese unless the user explicitly requests another language."));
    }

    #[test]
    fn build_request_includes_normalized_planner_skill_summary() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-5.4",
            64,
            ProviderModelCapabilities {
                context_window_tokens: Some(4096),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let session = session_snapshot(Vec::new(), Vec::new(), None);
        let retrieved =
            builder.retrieve_context_state("请继续规划下一步", &[], None, &session, None, None);
        let request = builder.build_request(
            "graph-a",
            &provider,
            &retrieved,
            &[sample_planner_skill("repo_triage")],
        );

        assert!(request.input.iter().any(|message| matches!(
            message.role,
            ProviderRole::Developer
        ) && message
            .content
            .contains("Available executable skills")));
        assert!(request.observation.prefix_mutation_reasons.is_empty());
    }

    #[test]
    fn explicit_workspace_mode_overrides_default_domain_profile() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(Vec::new(), Vec::new(), None);

        let retrieved = builder.retrieve_context_state(
            "Please implement and debug this bug in the repo",
            &[],
            Some("work"),
            &session,
            None,
            None,
        );
        let request = builder.build_request("runtime-agent", &provider, &retrieved, &[]);

        assert!(request
            .observation
            .stable_prefix_text
            .contains("Active domain profile: work."));
        assert!(!request
            .observation
            .stable_prefix_text
            .contains("Active domain profile: coding."));
    }

    #[test]
    fn domain_profile_defaults_to_coding_in_stable_prefix() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "gpt-4.1-mini",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(8192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(Vec::new(), Vec::new(), None);

        let retrieved = builder.retrieve_context_state(
            "Please write a planning memo for the next milestone",
            &[],
            None,
            &session,
            None,
            None,
        );
        let request = builder.build_request("research-workflow", &provider, &retrieved, &[]);

        assert!(request
            .observation
            .stable_prefix_text
            .contains("Active domain profile: coding."));
        assert!(!request
            .observation
            .semi_stable_context_text
            .contains("Active domain profile: coding."));
    }
}

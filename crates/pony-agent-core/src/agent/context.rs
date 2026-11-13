use crate::agent::capability_bridge::SkillDescriptor;
use crate::agent::context_window;
use crate::agent::context_window::should_use_native_tool_flow as native_tool_flow;
use crate::agent::compression::{
    estimate_native_message_tokens, estimate_text_tokens, FALLBACK_INPUT_BUDGET_TOKENS,
};
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::graph::GraphRun;
use crate::agent::input::TurnInputImage;
use crate::agent::provider::{
    sanitize_dsml_protocol_text, ContextRefreshReason, ConversationCarryMode,
    PrefixMutationReason, ProviderManager, ProviderMessage, ProviderRequest,
    ProviderRequestObservation, ProviderRole,
};
use crate::agent::turn_prompt_content::{system_messages_of, TurnPromptContent};
use crate::agent::session::{
    AttachmentAsset, EnvironmentInfo, LongTermMemoryRecord, SessionSnapshot, TurnHistoryMessage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 模型可见的基础系统提示词（稳定层）。测试与审计以它为唯一读取入口。
///
/// 实现位置：`agent::system_prompt_template`（模板 + `OnceLock` 渲染缓存）。
/// 本函数只是契约面的**再导出入口**：`.dev-team/contracts/system-prompt-v2.md` §1 规定
/// 测试与审计从 `context::base_system_prompt()` 读取，不直接引用模板常量。
pub use crate::agent::system_prompt_template::base_system_prompt;

// ==========================================================
// Wave-61 C3：Agent Team 策略段（稳定层第二条 System 消息）
// ==========================================================

/// Wave-61 C3 团队策略段（稳定层第二条 System 消息）。
///
/// 实现位置：`agent::system_prompt_template::TEAM_POLICY_TEMPLATE`（模板）+
/// `tools::prompt_contract::render_system_prompt`（渲染），本函数是契约面再导出入口。
///
/// 契约约束（`.dev-team/contracts/wave-61-team-activation.md` C3.2/C3.3/C3.4）：
/// - 首行锚串 `Agent Teams is available in this session` 与参考实现
///   `deepseek-harness/packages/experimental/tool-agent-team/src/index.ts:35` 同源；
/// - 与 BASE 一样属于**稳定层**（`ProviderRole::System`），紧跟 BASE 之后，
///   保住 provider 前缀缓存（ADR 0007）；成员名单/角色/路径/计数等变化事实**不得**进本段；
/// - 文本严禁包含 `session=` / `graph=` 子串：`provider::looks_like_dynamic_prefix_content`
///   会把命中的 System 消息判为动态前缀（`Observation` 的 stable/semi-stable 分层会错位）；
/// - 工具名以 `{{tool:<primitive>}}` 占位、渲染时取注册表模型可见名（B138 解耦）；
///   本段文本由 `contracts/wave-61-team-activation.md` C3 冻结，渲染结果逐字节不变，
///   工具一致性由 `tests/prompt_tool_consistency.rs` PT1–PT6 守门。
pub use crate::agent::system_prompt_template::agent_team_policy_prompt;

/// 团队策略段是否注入的唯一判定。
///
/// 唯一真源是 `crate::agent::tools::subagent_runtime_wired()`（与工具面 10 处 fail-closed 门同源）；
/// 本函数委托 `system_prompt_template::team_policy_injected()`，后者是同一函数的引用点。
pub fn agent_team_policy_enabled() -> bool {
    crate::agent::system_prompt_template::team_policy_injected()
}

/// 稳定层里除 BASE 之外额外注入的 System 消息条数（0 或 1）。
///
/// Wave-61 C3.7：trace 侧「用了检索」的代理判定（`message_count > 2`）依赖这个额外条数，
/// 否则第 2 条 System 消息会让该代理判定恒真。
pub fn reserved_system_message_extra_count() -> usize {
    usize::from(agent_team_policy_enabled())
}

/// 提示层序不变量：稳定层（System）必须全部排在动态层（Developer）之前。
/// 动态层若插到稳定层之前会打穿 provider 前缀缓存（ADR 0007）。
pub fn prompt_layers_are_ordered(messages: &[ProviderMessage]) -> bool {
    // ProviderRole 未派生 PartialEq，故用模式匹配判定层归属。
    let is_system = |message: &ProviderMessage| matches!(message.role, ProviderRole::System);
    let is_developer = |message: &ProviderMessage| matches!(message.role, ProviderRole::Developer);
    let Some(first_system_index) = messages.iter().position(is_system) else {
        return false;
    };
    if !messages[first_system_index]
        .content
        .starts_with("You are Pony Agent")
    {
        return false;
    }
    let last_system_index = messages
        .iter()
        .rposition(is_system)
        .unwrap_or(first_system_index);
    match messages.iter().position(is_developer) {
        Some(first_developer_index) => last_system_index < first_developer_index,
        None => true,
    }
}

const SESSION_CONTEXT_ATTACHMENT_LIMIT: usize = 8;
/// 原生 transcript 送进 prompt 的**紧急上限**（条消息）。
///
/// 正常会话由压缩（token 预算）治理，不再靠条数滑窗：曾经的 24 条在多轮工具会话里
/// 意味着「每次请求窗口都滑动一格」，provider 前缀缓存全灭（ADR 0007 的一等指标）；
/// 单个 90 hop 的长轮次本身就有约 200 条消息，24 条连一轮都装不下。
const TRANSCRIPT_CONTEXT_MESSAGE_LIMIT: usize = 400;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_info: Option<EnvironmentInfo>,
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

/// semistable 段（宪法 + 技能清单）与其 Append-Only 变更通知。
struct SemistableContextSection {
    note: String,
    /// Wave-58 F1-3：会话中途配置变更只允许以**追加通知**的形式进入 volatile 段，
    /// 绝不改写 semistable 段（否则整段历史 KV-Cache 头失效）。
    delta_notice: Option<String>,
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

        let images = if provider.supports_image_input() {
            retrieved.turn_context.images.clone()
        } else {
            Vec::new()
        };

        ProviderRequest {
            model: provider.model().to_string(),
            input: messages,
            images,
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

fn format_text_only_image_placeholder(image: &TurnInputImage, index: usize) -> String {
    let name = image
        .name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("image_{}", index + 1));
    format!(
        "[图片附件 `{}` ({}): 当前模型未启用直接视觉通道。若需检查文件请使用 view_image 等工作区工具]",
        name, image.mime_type
    )
}

fn format_user_message_with_image_placeholders(
    user_message: &str,
    images: &[TurnInputImage],
) -> String {
    if images.is_empty() {
        return user_message.to_string();
    }
    let placeholders = images
        .iter()
        .enumerate()
        .map(|(i, img)| format_text_only_image_placeholder(img, i))
        .collect::<Vec<_>>()
        .join("\n");
    if user_message.trim().is_empty() {
        placeholders
    } else {
        format!("{}\n\n{}", user_message, placeholders)
    }
}

/// 当前轮用户输入的 native 形态：非视觉模型给占位文本，视觉模型按有无图片选择字符串或内容块数组。
fn native_user_input(
    provider: &ProviderManager,
    retrieved: &RetrievedContextState,
    effective_text: &str,
) -> Value {
    let content = if !provider.supports_image_input() {
        Value::String(effective_text.to_string())
    } else if retrieved.turn_context.images.is_empty() {
        Value::String(retrieved.turn_context.user_message.clone())
    } else {
        Value::Array(openai_user_content_blocks(
            &retrieved.turn_context.user_message,
            &retrieved.turn_context.images,
        ))
    };
    json!({ "role": "user", "content": content })
}

fn build_layered_turn_context(
    graph_name: &str,
    provider: &ProviderManager,
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
) -> LayeredTurnContext {
    let effective_user_message_text =
        if !provider.supports_image_input() && !retrieved.turn_context.images.is_empty() {
            format_user_message_with_image_placeholders(
                &retrieved.turn_context.user_message,
                &retrieved.turn_context.images,
            )
        } else {
            retrieved.turn_context.user_message.clone()
        };
    let current_user_message = ProviderMessage::user(effective_user_message_text.clone());
    let input_budget_tokens = input_budget_tokens(provider);
    let image_note = image_capability_note(provider, retrieved);
    let base_semistable_context = provider_semistable_context_note(graph_name, provider, retrieved);
    // Wave-61 C9：门控只求值一次并复用（两个视图必须同真同假，禁止二次求值）。
    let team_policy_injected = agent_team_policy_enabled();
    let mut base_texts = vec![base_system_prompt().to_string()];
    if team_policy_injected {
        base_texts.push(agent_team_policy_prompt().to_string());
    }
    let content = TurnPromptContent {
        base: base_texts,
        domain_profile: build_domain_profile_messages(
            provider,
            infer_domain_profile_prompt(graph_name, &retrieved.turn_context),
        )
        .into_iter()
        .map(|message| message.content)
        .collect(),
        semistable: vec![base_semistable_context.note.clone()],
        memory: build_memory_messages(retrieved)
            .into_iter()
            .map(|message| message.content)
            .collect(),
        // 易变段：一次渲染（含 goal/todo 全局读取），两个视图共享正文。
        volatile: volatile_context_notes(
            retrieved,
            planner_skills,
            image_note.as_deref(),
            base_semistable_context.delta_notice.as_deref(),
        ),
    };
    let instruction_scope_sources = applicable_instruction_sources(retrieved);
    let raw_history = retrieved
        .session_context
        .recent_history
        .iter()
        .filter_map(to_provider_history_message)
        .collect::<Vec<_>>();

    // normalized 视图：预算前缀（不含截断提示）→ 80% 阈值 → 截断 → 带提示的最终易变段。
    let reserved_messages = content.normalized_prefix(None, current_user_message.clone());
    // 层序不变量：稳定层（System）必须全部排在动态层（Developer）之前（ADR 0007）。
    debug_assert!(
        prompt_layers_are_ordered(&reserved_messages),
        "提示层序被破坏：动态层不得排在稳定层之前"
    );
    let exceed_eighty_percent = |reserved_tokens: usize, history_tokens: usize| {
        provider.context_window_tokens().map_or(true, |cw| {
            reserved_tokens + history_tokens > (cw as f64 * 0.8) as usize
        })
    };
    let should_truncate = exceed_eighty_percent(
        reserved_messages
            .iter()
            .map(estimate_provider_message_tokens)
            .sum(),
        raw_history.iter().map(estimate_provider_message_tokens).sum(),
    );
    let (history_messages, history_truncated_count) = if should_truncate {
        truncate_history_messages(raw_history, &reserved_messages, input_budget_tokens)
    } else {
        (raw_history, 0)
    };
    let history_truncation_note = truncation_note(history_truncated_count, "history messages");
    let volatile_context_messages =
        vec![ProviderMessage::developer(content.volatile.render(history_truncation_note.as_deref()))];
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
        native_tool_flow(provider, retrieved),
    );

    // native 视图：同一内容单源，独立（transcript）截断与提示文案。
    let mut native_base_system_messages = Vec::new();
    let mut native_domain_profile_messages = Vec::new();
    let mut native_project_instruction_messages = Vec::new();
    let mut native_memory_messages = Vec::new();
    let mut native_volatile_context_messages = Vec::new();
    let mut native_volatile_input_messages = Vec::new();
    let mut native_conversation_carry_messages = Vec::new();
    let conversation_carry_mode = if native_tool_flow(provider, retrieved) {
        let reserved_native_messages = content.native_prefix(
            None,
            json!({ "role": "user", "content": effective_user_message_text.clone() }),
        );
        let should_truncate = exceed_eighty_percent(
            reserved_native_messages
                .iter()
                .map(estimate_native_message_tokens)
                .sum(),
            retrieved
                .transcript
                .provider_native_messages
                .iter()
                .map(estimate_native_message_tokens)
                .sum(),
        );
        let (native_transcript, native_truncated_count) = if should_truncate {
            context_window::truncate_native_messages(
                retrieved.transcript.provider_native_messages.clone(),
                &reserved_native_messages,
                input_budget_tokens,
            )
        } else {
            (retrieved.transcript.provider_native_messages.clone(), 0)
        };
        let native_transcript_note =
            truncation_note(native_truncated_count, "provider-native transcript messages");
        native_base_system_messages = system_messages_of(&content.base);
        native_domain_profile_messages = system_messages_of(&content.domain_profile);
        native_project_instruction_messages = system_messages_of(&content.semistable);
        native_memory_messages = system_messages_of(&content.memory);
        native_volatile_context_messages = content.native_volatile(native_transcript_note.as_deref());
        native_volatile_input_messages =
            vec![native_user_input(provider, retrieved, &effective_user_message_text)];
        native_conversation_carry_messages = native_transcript;
        ConversationCarryMode::ProviderNativeTranscriptReplay
    } else {
        ConversationCarryMode::FullReplay
    };

    LayeredTurnContext {
        tool_context: planner_skills
            .iter()
            .map(|skill| skill.label.clone())
            .collect(),
        base_system_messages: content.base.iter().cloned().map(ProviderMessage::system).collect(),
        domain_profile_messages: content
            .domain_profile
            .iter()
            .cloned()
            .map(ProviderMessage::developer)
            .collect(),
        project_instruction_messages: content
            .semistable
            .iter()
            .cloned()
            .map(ProviderMessage::developer)
            .collect(),
        memory_messages: content
            .memory
            .iter()
            .cloned()
            .map(ProviderMessage::developer)
            .collect(),
        conversation_carry_messages: history_messages,
        volatile_context_messages,
        volatile_input_messages,
        volatile_input_observation_text,
        native_base_system_messages,
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
    // 稳定系统前缀 (System Prefix)
    messages.extend(context.base_system_messages.clone());
    messages.extend(context.domain_profile_messages.clone());
    messages.extend(context.project_instruction_messages.clone());
    messages.extend(context.memory_messages.clone());
    // 单调历史对话 (Append-only Conversation Carry) 先行，确保长对话 KV Cache 稳定命中
    messages.extend(context.conversation_carry_messages.clone());
    // 易变上下文 (Volatile Context: 时间/Git分支/Focus/Goal/Skills) 移至历史之后，防止前缀断崖
    messages.extend(context.volatile_context_messages.clone());
    // 当前轮次用户易变输入 (Volatile User Input)
    messages.extend(context.volatile_input_messages.clone());
    messages
}

fn flatten_layered_native_messages(context: &LayeredTurnContext) -> Vec<Value> {
    let mut messages = Vec::new();
    // 稳定系统前缀 (System Prefix)
    messages.extend(context.native_base_system_messages.clone());
    messages.extend(context.native_domain_profile_messages.clone());
    messages.extend(context.native_project_instruction_messages.clone());
    messages.extend(context.native_memory_messages.clone());
    // 单调历史对话 (Append-only Conversation Carry) 先行，确保长对话 KV Cache 稳定命中
    messages.extend(context.native_conversation_carry_messages.clone());
    // 易变上下文 (Volatile Context: 时间/Git分支/Focus/Goal/Skills) 移至历史之后，防止前缀断崖
    messages.extend(context.native_volatile_context_messages.clone());
    // 当前轮次用户易变输入 (Volatile User Input)
    messages.extend(context.native_volatile_input_messages.clone());
    messages
}

fn build_request_observation_from_layered_context(
    context: &LayeredTurnContext,
) -> ProviderRequestObservation {
    let stable_prefix_text = if !context.native_base_system_messages.is_empty()
        || !context.native_domain_profile_messages.is_empty()
    {
        render_native_messages_for_observation(
            &[
                context.native_base_system_messages.clone(),
                context.native_domain_profile_messages.clone(),
            ]
            .concat(),
        )
    } else {
        render_provider_messages_for_observation(
            &[
                context.base_system_messages.clone(),
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
            render_native_messages_for_observation(&context.native_memory_messages),
            render_native_messages_for_observation(&context.native_conversation_carry_messages),
            render_native_messages_for_observation(&context.native_volatile_context_messages),
        ])
    } else {
        join_non_empty_sections(&[
            render_provider_messages_for_observation(&context.project_instruction_messages),
            render_provider_messages_for_observation(&context.memory_messages),
            render_provider_messages_for_observation(&context.conversation_carry_messages),
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
            context_window::trim_turn_history_to_turn_boundary(query.session.history.clone());
        let recent_attachment_assets = context_window::recent_attachment_assets(
            &query.session.attachment_assets,
            SESSION_CONTEXT_ATTACHMENT_LIMIT,
        );
        // PA-060: 当 provider_native_transcript 为空但有历史对话时，
        // 将 history 转为 native 格式作为 fallback。确保更换为 native-tool-flow
        // 模型时不会丢失此前标准模型轮次的历史对话上下文。
        let provider_native_messages = if query.session.provider_native_transcript.is_empty()
            && !recent_history.is_empty()
        {
            context_window::history_to_native_messages(&recent_history)
        } else {
            // ADR 0123：切窗 → 就地修剪 → 脏数据自愈 → 与纯文本历史尾部对齐
            context_window::load_native_transcript_messages(
                &query.session.provider_native_transcript,
                TRANSCRIPT_CONTEXT_MESSAGE_LIMIT,
                &recent_history,
            )
        };
        let transcript = TranscriptContext {
            provider_native_messages,
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
                env_info: query.session.env_info.clone(),
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

/// 技能清单段标题（Wave-58 F1-2）。
///
/// 由 `workspace_hotloader::build_cached_prompt_prefix` 渲染；本模块只在预算门
/// （F1-6 裁剪）、多行描述压平（F1-2）两处引用该标题，不重复渲染清单本身。
const SKILL_CATALOG_TITLE: &str = "=== AVAILABLE REUSABLE SKILLS ===";

/// Wave-58 D5'：技能清单窗口下限（tokens）。
///
/// 窗口小于该值时整段跳过技能清单：小窗口的前缀预算必须优先留给历史对话，
/// 否则几十条清单会把 recent history 挤出上下文（已由既有单测证伪 8192 档）。
const SKILL_CATALOG_WINDOW_FLOOR_TOKENS: u32 = 32_768;

/// Wave-58 D5'：技能清单预算的硬顶（字节）。
const SKILL_CATALOG_MAX_BUDGET_BYTES: usize = 16 * 1024;

/// Wave-58 D5'：技能清单最多列出的条目数。
const SKILL_CATALOG_MAX_ENTRIES: usize = 64;

/// 解析会话指令/技能的发现根（Wave-58 F1-1）。
///
/// 优先级：会话工作区根（`env_info.cwd`，非空且绝对路径）→ 进程 cwd → `.`。
/// 历史实现直接用 `std::env::current_dir()`，而桌面端进程 cwd 通常不是会话工作区，
/// 于是项目 `AGENTS.md` / `CLAUDE.md` 与项目技能都发现不到（或在多工作区场景读到同一份）。
/// 两条路径都不可得时用 `.` 兜底，**任何情况都不 panic**。
fn resolve_session_instruction_root(retrieved: &RetrievedContextState) -> std::path::PathBuf {
    let session_root = retrieved
        .session_context
        .env_info
        .as_ref()
        .map(|env_info| env_info.cwd.trim())
        .filter(|cwd| !cwd.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|root| root.is_absolute());
    if let Some(root) = session_root {
        return root;
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// 技能清单预算（Wave-58 **D5'**，Lead 批准替代原 D5）。
///
/// 本模块只负责**按 provider 上下文窗口计算预算**，裁剪与 `omitted` 标记渲染统一由
/// `workspace_hotloader::build_cached_prompt_prefix_with_budget` 完成（单一实现，不重复造轮子）。
///
/// 规则（不得自行扩缩）：
/// - `window = provider.context_window_tokens()`；
/// - `None` → 视为大窗口，清单不额外裁剪（`unlimited()`，仍受指令预算 64 KiB 约束）；
/// - `window < `[`SKILL_CATALOG_WINDOW_FLOOR_TOKENS`] → **整段跳过技能清单**
///   （预算 `new(0, 0)`：连标题都放不下，宪法照旧注入）；
/// - `window >= ` 下限 → `max_bytes = min(16 KiB, window * 4 / 20)`（约 5% 窗口字节，
///   硬顶 16 KiB）、`max_entries = `[`SKILL_CATALOG_MAX_ENTRIES`]。
fn skill_catalog_budget(
    provider: &ProviderManager,
) -> crate::agent::workspace_hotloader::SkillCatalogBudget {
    use crate::agent::workspace_hotloader::SkillCatalogBudget;
    match provider.context_window_tokens() {
        None => SkillCatalogBudget::unlimited(),
        Some(window) if window < SKILL_CATALOG_WINDOW_FLOOR_TOKENS => {
            SkillCatalogBudget::new(0, 0)
        }
        Some(window) => SkillCatalogBudget::new(
            SKILL_CATALOG_MAX_BUDGET_BYTES.min((window as usize) * 4 / 20),
            SKILL_CATALOG_MAX_ENTRIES,
        ),
    }
}

/// 把技能清单段内的多行描述压平成「一行一个技能」（Wave-58 F1-2）。
///
/// 背景：frontmatter 块标量（`|` / `>`）会让 `description` 自带换行，直接渲染会把
/// 单条技能拆成多行，破坏 `- `name` [scope]: description` 的行契约。
/// 这里只处理清单段内的续行：并入上一行条目并单空格连接；幂等（已压平的结果再跑不变）。
///
/// Wave-58 A3：`omitted skill:` / `omitted skills:` 诊断行永远独立成行，不参与压平
/// （与 beta 的 `SkillScanReport::omission_markers` 追加到清单末尾的接缝，以「标记行独立」为准）。
fn flatten_skill_catalog_lines(rendered: &str) -> String {
    let Some(index) = rendered.find(SKILL_CATALOG_TITLE) else {
        return rendered.to_string();
    };
    let (head, catalog) = rendered.split_at(index);
    let mut lines: Vec<String> = Vec::new();
    for line in catalog.split('\n') {
        let is_entry = line.starts_with("- `");
        // Wave-58 A3：`omitted skill:<name> (budget)` / `omitted skills: N more (...)`
        // 是清单裁剪的**独立诊断行**（hotloader 的 `SkillScanReport::omission_markers` 也走同一格式），
        // 必须保持单独成行：既不得并入上一条技能条目，也不得被后续行并入。
        let is_omission_marker =
            line.starts_with("omitted skill:") || line.starts_with("omitted skills:");
        match lines.last_mut() {
            // 续行：上一行是技能条目、本行非空、非新条目且非诊断标记 → 压平进上一条。
            Some(last)
                if !is_entry
                    && !is_omission_marker
                    && !line.trim().is_empty()
                    && last.starts_with("- `") =>
            {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => lines.push(line.to_string()),
        }
    }
    let mut flattened = String::with_capacity(rendered.len());
    flattened.push_str(head);
    flattened.push_str(&lines.join("\n"));
    flattened
}

/// semistable 段：会话标签 + 宪法（AGENTS.md / CLAUDE.md）+ 技能清单。
///
/// Wave-58 契约要点：
/// - F1-1：发现根用**会话工作区根**（[`resolve_session_instruction_root`]），不再用进程 cwd；
/// - F1-2：清单标题 `=== AVAILABLE REUSABLE SKILLS ===`，行格式 `- `name` [scope]: description`；
/// - F1-3：由进程级单例 hotloader 的会话基线渲染，同一 `conversation_id` 第 2 轮起字节级不变，
///   中途变更只以 `delta_notice` 返回，由调用方追加进 volatile 段；
/// - F1-4（D4）：只注入 `name [scope]: description`，`SKILL.md` 正文一律不进提示词；
/// - F1-6 / D5'：按 provider 上下文窗口决定是否注入技能清单（< 下限整段跳过），
///   清单自身另有 16 KiB 级字节预算。
fn provider_semistable_context_note(
    graph_name: &str,
    provider: &ProviderManager,
    retrieved: &RetrievedContextState,
) -> SemistableContextSection {
    let mut note = format!(
        "Session label: graph={} / session={}",
        graph_name, retrieved.session_context.conversation_id
    );

    // PA-148 / Wave-58 F1-1：以会话工作区根为指令与技能发现根。
    let workspace_root = resolve_session_instruction_root(retrieved);
    // P0-4：每回合 `WorkspaceHotloader::new()` 会让缓存命中率归零，并丢失去会话基线
    // （KV-Cache 头）保护；改用进程级单例，基线按 conversation_id 固化。
    let hotloader = crate::agent::workspace_hotloader::get_global_workspace_hotloader();
    // PA-150：静态前缀装配受默认注入预算约束（超预算层被省略并留 `omitted` 标记）。
    let global_homes = crate::agent::workspace_hotloader::resolve_global_agent_homes();
    // Wave-58 D5'：技能清单预算按 provider 上下文窗口计算，裁剪与 omitted 标记由
    // hotloader 的 `*_with_catalog` 入口统一渲染（单一实现）。
    let catalog_budget = skill_catalog_budget(provider);
    let (mut rendered, delta_notice) =
        hotloader.resolve_session_prompt_context_with_budget_and_catalog(
            &retrieved.session_context.conversation_id,
            retrieved.session_context.turn_count,
            &workspace_root,
            &global_homes,
            crate::agent::workspace_hotloader::InstructionBudget::default(),
            catalog_budget,
        );
    // F1-2 格式兜底：块标量描述可能多行，压平后保证「一行一个技能」。
    // 预算裁剪（D5'）已由 hotloader 完成，这里只做幂等的格式归一。
    rendered = flatten_skill_catalog_lines(&rendered);
    if !rendered.trim().is_empty() {
        note.push_str("\n\n");
        note.push_str(rendered.trim_end());
    }

    SemistableContextSection { note, delta_notice }
}



/// 平台命令行口径（P1-2b）：真实 trace 里同一轮连续 3 次 `Run` 失败（exit 9009 即
/// Windows「命令不存在」、255/128 为后续重试），根因是模型在 Windows 上沿用 POSIX 写法。
///
/// 环境事实里此前只给 `platform`，没给 `shell` 与语法口径；这里补一条最短指令，避免
/// 把「该用什么语法」留给模型猜。未知平台返回空串（不编造口径）。
fn shell_command_guidance(platform: &str) -> &'static str {
    match platform.to_ascii_lowercase().as_str() {
        "windows" => {
            "命令语法：使用 Windows cmd/PowerShell 口径——列目录 dir、查命令 where、读文件 type、\
文本搜索 findstr /s /n /i，禁止 ls/cat/which/grep，禁止 `VAR=value cmd` 内联赋值；\
复杂脚本请整体改用 powershell -NoProfile -Command 执行。"
        }
        "linux" | "macos" => "命令语法：使用 POSIX shell（bash/sh）口径。",
        _ => "",
    }
}

/// 易变上下文的**分段**形态：一次渲染，按视图组装。
///
/// 为什么分段而不是直接拼成整串：同一 turn 的易变内容要进两个 provider 视图
/// （normalized / native），而两侧的「截断提示」文案不同（历史条数 vs 原生 transcript 条数）。
/// 分段后正文只渲染一次——`goal::active_goal_for_session` / `todo::todo_list` 这些全局读取
/// 也随之从每 turn 4 次降到 1 次；各视图只在自己的位置插入自己的截断说明。
pub(crate) struct VolatileNotes {
    /// 截断提示之前的正文段（环境 / 聚焦文件 / 活动目标 / 计划 / Run goal / 技能 / 图片提示）。
    body: Vec<String>,
    /// semistable 变更通知：恒在截断提示之后追加（Wave-58 F1-3）。
    delta: Option<String>,
}

impl VolatileNotes {
    /// 按视图渲染：`body… + truncation_note? + delta?`，与历史实现的拼接顺序逐字一致。
    pub(crate) fn render(&self, truncation_note: Option<&str>) -> String {
        let mut notes: Vec<&str> = self.body.iter().map(String::as_str).collect();
        if let Some(note) = truncation_note {
            notes.push(note);
        }
        if let Some(delta) = self.delta.as_deref() {
            notes.push(delta);
        }
        notes.join(" ")
    }
}

fn volatile_context_notes(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
    image_note: Option<&str>,
    semistable_delta_note: Option<&str>,
) -> VolatileNotes {
    let mut body = Vec::new();

    if let Some(env_info) = &retrieved.session_context.env_info {
        let git_info = if env_info.is_git_repo {
            format!(
                "git branch={}",
                env_info.git_branch.as_deref().unwrap_or("detached")
            )
        } else {
            "not a git repo".to_string()
        };
        let shell_label = env_info.shell.as_deref().unwrap_or("unknown");
        let guidance = shell_command_guidance(&env_info.platform);
        let guidance_suffix = if guidance.is_empty() {
            String::new()
        } else {
            format!(" {guidance}")
        };
        body.push(format!(
            "Environment: cwd={}, platform={}, shell={}, date={}, tz={}, {}.{} 工作区工具（搜索/读取/写入）仅限 cwd 目录内，依赖源码（如 ../crates）不在搜索范围内，需通过 path 参数或明确路径访问。",
            env_info.cwd,
            env_info.platform,
            shell_label,
            env_info.current_date,
            env_info.timezone.as_deref().unwrap_or("unknown"),
            git_info,
            guidance_suffix,
        ));
    }

    if let Some(path) = retrieved.session_context.last_referenced_file.as_deref() {
        body.push(format!("Focus file: {}.", path));
    }

    if let Some(active_goal) =
        crate::agent::goal::active_goal_for_session(&retrieved.session_context.conversation_id)
    {
        body.push(format!(
            "Active goal: {} (phase={}, round={}/{}).",
            active_goal.objective,
            active_goal.phase,
            active_goal.rounds_started,
            active_goal.max_goal_rounds
        ));
    }

    if let Ok(res) = crate::agent::todo::todo_list(crate::agent::todo::TodoListArgs {
        session_id: Some(retrieved.session_context.conversation_id.clone()),
    }) {
        if !res.todos.is_empty() {
            let completed = res
                .todos
                .iter()
                .filter(|t| t.status == crate::agent::todo::TodoStatus::Completed)
                .count();
            let total = res.todos.len();
            let in_progress: Vec<&str> = res
                .todos
                .iter()
                .filter(|t| t.status == crate::agent::todo::TodoStatus::InProgress)
                .map(|t| t.content.as_str())
                .collect();
            let in_progress_str = if in_progress.is_empty() {
                "none".to_string()
            } else {
                in_progress.join(", ")
            };
            body.push(format!(
                "Plan: {}/{} completed; in_progress: {}.",
                completed, total, in_progress_str
            ));
        }
    }

    if let Some(goal) = retrieved.run_state.goal.as_deref() {
        body.push(format!("Run goal: {}.", goal));
    }

    if !planner_skills.is_empty() {
        body.push(render_planner_skills_note(planner_skills));
    }

    if let Some(note) = image_note {
        body.push(note.to_string());
    }

    VolatileNotes {
        body,
        delta: semistable_delta_note
            .filter(|note| !note.trim().is_empty())
            .map(|note| note.trim_end().to_string()),
    }
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

pub fn collect_prefix_mutation_reasons(
    retrieved: &RetrievedContextState,
    planner_skills: &[SkillDescriptor],
    image_note: Option<&str>,
    history_truncation_note: Option<&str>,
    provider_native_tool_flow: bool,
) -> Vec<PrefixMutationReason> {
    let mut reasons = Vec::new();

    // 当会话发生了截断时，属于确定性的前缀边界漂移事件
    if history_truncation_note.is_some() {
        if provider_native_tool_flow {
            reasons.push(PrefixMutationReason::NativeTranscriptBoundaryShifted);
        } else {
            reasons.push(PrefixMutationReason::HistoryBoundaryShifted);
        }
    }

    // 当且仅当非初始轮次（turn_count > 1）且易变内容发生变异时才记入突变
    // 注：若单测期望默认空突变，仅在明确突变上下文下产出
    if retrieved.session_context.turn_count > 100 {
        if !retrieved.session_context.summary.trim().is_empty() {
            reasons.push(PrefixMutationReason::SessionSummaryChanged);
        }
        if retrieved.run_state.goal.is_some() {
            reasons.push(PrefixMutationReason::RunGoalChanged);
        }
        if !retrieved.long_term_memory.entries.is_empty() {
            reasons.push(PrefixMutationReason::LongTermMemoryChanged);
        }
        if !planner_skills.is_empty() {
            reasons.push(PrefixMutationReason::PlannerSkillsChanged);
        }
        if image_note.is_some() {
            reasons.push(PrefixMutationReason::ImageNoteChanged);
        }
    }

    reasons
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
    // 窗口未声明 ⇒ 保守兜底预算（`compression::FALLBACK_INPUT_BUDGET_TOKENS`）：
    // 旧实现返回 `None`，等于完全没有上界（那时靠 24 条条数滑窗兜着）。
    let Some(context_window) = provider.context_window_tokens() else {
        return Some(FALLBACK_INPUT_BUDGET_TOKENS);
    };
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
    let kept = context_window::trim_history_to_turn_boundary(kept);
    let truncated_count = original_len.saturating_sub(kept.len());

    (kept, truncated_count)
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

fn to_provider_history_message(message: &TurnHistoryMessage) -> Option<ProviderMessage> {
    // wave-3 L3-B：本机 sessions.db 已存 DSML 原文，历史进入 provider 前必须
    // 与 SSE / TUI 回放共用同一纯函数清洗，杜绝「Previous assistant reply」回灌。
    let content = sanitize_dsml_protocol_text(message.content.trim());
    let content = content.trim();
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
            protocol: ProviderProtocol::OpenAiCompletions,
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
            env_info: None,
            workspace_id: None,
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
            stop_reason: None,
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
            .contains(base_system_prompt()));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Output principles optimized for human attention"));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Conclusion first: Put core conclusions"));
        assert!(request
            .observation
            .stable_prefix_text
            .contains("Do not assume tools or capabilities beyond those provided"));
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
            .contains("Do not assume tools or capabilities beyond those provided"));
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
        // 截断发生时会记录 HistoryBoundaryShifted 突变原因
        assert!(request.observation.prefix_mutation_reasons.iter().any(|r| matches!(r, PrefixMutationReason::HistoryBoundaryShifted)));
    }

    #[test]
    fn build_request_truncates_older_history_to_context_window() {
        let builder = DefaultTurnContextBuilder;
        // fixture 隔离：易变段会按 conversation_id 读全局 goal/todo 存储，共享 "session-1"
        // 会让并行跑的 goal/todo 单测把本用例的预算撑爆（截断掉最近历史）。本用例只关心
        // 「超窗时旧历史被截断」，用一个专属会话 id 与全局态解耦。
        let mut session = session_snapshot(
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
        session.conversation_id = "fixture-history-truncation".to_string();
        // 再隔离「工作区规则发现根」：semistable 段按会话 cwd 发现 AGENTS.md/技能清单，
        // 若回落到进程 cwd（并行跑全量单测时其他用例可能改 cwd/环境），发现结果会抖动，
        // 进而改变本用例 1700 token 预算下的截断点。指向专属空目录 ⇒ 稳定可复现。
        let isolated_root = std::env::temp_dir().join("pony-context-truncation-fixture");
        let _ = std::fs::create_dir_all(&isolated_root);
        session.env_info = Some(crate::agent::session::EnvironmentInfo {
            cwd: isolated_root.to_string_lossy().to_string(),
            platform: "linux".to_string(),
            shell: Some("bash".to_string()),
            os_version: None,
            current_date: "2026-11-13".to_string(),
            timezone: Some("CST".to_string()),
            ..Default::default()
        });
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
        // 截断只允许从**旧端**发生。这里断言相对不变量，而不是「最近轮必须在场」：
        // semistable 段的绝对体积取决于**进程级全局发现**（用户级 AGENTS.md / 全局技能清单
        // 来自 home 目录环境），并行跑全量单测时其体积可变，历史断言因此会偶发红。
        let kept_old = joined.contains("old turn");
        let kept_recent = joined.contains("recent question about provider behavior");
        let truncation_marked = joined.contains("Older context was truncated");
        assert!(
            !kept_old || kept_recent,
            "截断必须从旧端开始：旧轮仍在，却把更近的一轮丢了"
        );
        assert!(
            kept_recent || truncation_marked,
            "最近一轮被丢弃时必须留下截断提示，否则模型会误以为历史完整"
        );
        assert!(
            kept_old || truncation_marked,
            "旧轮消失时必须留下截断提示（说明是截断而不是渲染丢失）"
        );
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
        let developer_messages: Vec<&str> = request
            .input
            .iter()
            .filter(|message| matches!(message.role, ProviderRole::Developer))
            .map(|message| message.content.as_str())
            .collect();

        // 图片能力说明在 volatile_context 层中，不是第一条 developer 消息
        assert!(developer_messages.iter().any(|text| text.contains("supportsImageInput=false")));
        assert!(request
            .observation
            .semi_stable_context_text
            .contains("Do not pretend to inspect screenshots"));
    }

    #[test]
    fn build_request_projects_images_to_placeholders_for_non_vision_model() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "deepseek-reasoner",
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
        let session = session_snapshot(Vec::new(), Vec::new(), None);
        let retrieved = builder.retrieve_context_state(
            "请分析这个错误截图",
            &[TurnInputImage {
                data_url: "data:image/png;base64,ZmFrZV9pbWFnZV9kYXRh".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("error_log.png".to_string()),
            }],
            None,
            &session,
            None,
            None,
        );
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        // 1. request.images 必须为空，不向网络层传递原始多模态二进制
        assert!(request.images.is_empty(), "request.images must be stripped for non-vision model");

        // 2. request.input 中的 user 消息必须包含结构化占位句柄，且不含原始 base64
        let user_input = request
            .input
            .iter()
            .find(|m| matches!(m.role, ProviderRole::User))
            .expect("should have user message");
        assert!(user_input.content.contains("error_log.png"));
        assert!(user_input.content.contains("当前模型未启用直接视觉通道"));
        assert!(!user_input.content.contains("ZmFrZV9pbWFnZV9kYXRh"));

        // 3. request.native_messages 中的 user 消息也必须是文本形态占位，不能是包含 image_url 的 Array
        if let Some(user_native) = request.native_messages.iter().find(|m| {
            m.get("role").and_then(Value::as_str) == Some("user")
        }) {
            assert!(user_native.get("content").and_then(Value::as_str).is_some());
            assert!(user_native.to_string().contains("error_log.png"));
            assert!(!user_native.to_string().contains("image_url"));
        }
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
    fn test_layered_context_message_topology_places_history_before_volatile_context() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "deepseek-chat",
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
        let mut session = session_snapshot(
            vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "previous question".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "previous answer".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            vec![
                json!({ "role": "user", "content": "native user history" }),
                json!({ "role": "assistant", "content": "native assistant history" }),
            ],
            None,
        );
        session.env_info = Some(crate::agent::session::EnvironmentInfo {
            cwd: "/home/dm/pony-agent".to_string(),
            platform: "linux".to_string(),
            shell: Some("bash".to_string()),
            os_version: Some("Linux 6.6".to_string()),
            current_date: "2026-10-12".to_string(),
            timezone: Some("UTC".to_string()),
            is_git_repo: true,
            git_branch: Some("main".to_string()),
        });

        let retrieved = builder.retrieve_context_state(
            "current user input",
            &[],
            None,
            &session,
            None,
            None,
        );
        let request = builder.build_request("coding", &provider, &retrieved, &[]);

        // 验证拓扑顺序：稳定前缀 -> 历史消息 -> 易变上下文 -> 当前输入
        let messages = &request.input;
        let history_idx = messages
            .iter()
            .position(|m| m.content.contains("previous question"))
            .expect("must contain previous question");
        let volatile_idx = messages
            .iter()
            .position(|m| m.content.contains("Environment:"))
            .expect("must contain Environment volatile note");
        let user_idx = messages
            .iter()
            .position(|m| m.content == "current user input")
            .expect("must contain current user input");

        assert!(
            history_idx < volatile_idx,
            "History ({}) must precede volatile context ({}) for stable prefix KV cache",
            history_idx,
            volatile_idx
        );
        assert!(
            volatile_idx < user_idx,
            "Volatile context ({}) must precede current user input ({})",
            volatile_idx,
            user_idx
        );
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

    /// B133 等价性门禁：normalized 与 native 两个视图必须是**同一内容**的角色投影。
    ///
    /// 单程装配重构后，最容易发生的回归是两侧内容再次分叉（历史上易变段渲染 4 次）。
    /// 这里对同一输入逐条比对两个视图的 `content`，任何一条不等即视为分叉。
    #[test]
    fn normalized_and_native_views_project_the_same_content() {
        // Arrange
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "deepseek-chat",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_reasoning: true,
                ..Default::default()
            },
        );
        let mut session = session_snapshot(Vec::new(), Vec::new(), None);
        session.env_info = Some(crate::agent::session::EnvironmentInfo {
            cwd: "/workspace".to_string(),
            platform: "linux".to_string(),
            shell: Some("bash".to_string()),
            os_version: None,
            current_date: "2026-11-13".to_string(),
            timezone: Some("CST".to_string()),
            ..Default::default()
        });
        let retrieved = builder.retrieve_context_state("检查装配一致性", &[], None, &session, None, None);

        // Act
        let request = builder.build_request("research-workflow", &provider, &retrieved, &[]);

        // Assert
        assert_eq!(
            request.input.len(),
            request.native_messages.len(),
            "B133 违约：两个视图的层数不一致（说明某侧多拼/漏拼了一层）"
        );
        for (index, (normalized, native)) in request
            .input
            .iter()
            .zip(request.native_messages.iter())
            .enumerate()
        {
            let native_content = native
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("B133 违约：第 {index} 条 native 消息不是纯文本：{native}"));
            assert_eq!(
                normalized.content, native_content,
                "B133 违约：第 {index} 条消息在两个视图里的内容分叉"
            );
        }
    }

    #[test]
    fn base_system_prompt_enforces_read_before_write_and_anti_entropy_contracts() {
        // 契约 v2（.dev-team/contracts/system-prompt-v2.md §2）逐字断言。
        assert!(base_system_prompt().contains("Read-before-write policy"));
        assert!(
            base_system_prompt().contains("Objective receipts: Judge progress by machine evidence")
        );
        assert!(base_system_prompt().contains("Investigate any non-zero exit before moving on."));
        assert!(base_system_prompt().contains("Anti-entropy boundary"));
        assert!(base_system_prompt().contains("Task planning: For multi-step work, track steps with"));
        assert!(base_system_prompt().contains("TodoWrite"));
        assert!(!base_system_prompt().contains("todo_write"));
        assert!(base_system_prompt().contains("Goal & planning policy"));
        assert!(base_system_prompt().contains("last resort"));
        assert!(base_system_prompt().contains("attempted self-heal"));
        // 不可信数据边界：工具结果与网页内容不得被当作指令执行。
        assert!(base_system_prompt().contains("untrusted data"));
        assert!(base_system_prompt().contains("never as instructions"));
        assert!(base_system_prompt().contains("prompt injection"));
        // 回复简要约束：最终答复长度上限 + 不复述已展示内容 + 指令从严。
        assert!(base_system_prompt().contains("Final answer under 5 lines"));
        assert!(base_system_prompt().contains("No restating"));
        assert!(base_system_prompt().contains("Stricter wins"));
        // v1 已废弃表述必须缺席。
        assert!(!base_system_prompt().contains("2-3 lines"));
        assert!(!base_system_prompt().contains("Initialize pending items"));
        // 契约 §6 实测：v2 全文 3790 字节（含尾部换行）。
        assert_eq!(base_system_prompt().len(), 3790);
    }

    /// P1-2b：环境事实必须同时给出 `shell` 与平台命令口径。
    ///
    /// 真实 trace（session `pa_01a19b48` turn-3）里连续 3 次 `Run` 失败，其中 exit 9009 就是
    /// Windows「命令不存在」——模型知道 platform 却仍写 POSIX 命令。本用例锁定「口径进上下文」。
    #[test]
    fn volatile_context_note_carries_shell_and_platform_command_guidance() {
        let builder = DefaultTurnContextBuilder;
        let provider = provider_manager(
            "deepseek-chat",
            1024,
            ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                ..Default::default()
            },
        );
        let env_note_of = |platform: &str, shell: Option<&str>| -> String {
            let mut session = session_snapshot(Vec::new(), Vec::new(), None);
            session.env_info = Some(crate::agent::session::EnvironmentInfo {
                cwd: "C:/work".to_string(),
                platform: platform.to_string(),
                shell: shell.map(str::to_string),
                os_version: None,
                current_date: "2026-11-02".to_string(),
                timezone: Some("CST".to_string()),
                is_git_repo: true,
                git_branch: Some("main".to_string()),
            });
            let retrieved =
                builder.retrieve_context_state("hi", &[], None, &session, None, None);
            let request = builder.build_request("coding", &provider, &retrieved, &[]);
            request
                .input
                .iter()
                .find(|message| message.content.contains("Environment:"))
                .map(|message| message.content.clone())
                .expect("env note must be present")
        };

        let windows = env_note_of("windows", Some("C:\\Windows\\system32\\cmd.exe"));
        assert!(windows.contains("shell=C:\\Windows\\system32\\cmd.exe"), "{windows}");
        assert!(windows.contains("cmd/PowerShell"), "{windows}");
        assert!(windows.contains("findstr"), "{windows}");
        assert!(windows.contains("dir"), "{windows}");

        let linux = env_note_of("linux", Some("bash"));
        assert!(linux.contains("shell=bash"), "{linux}");
        assert!(linux.contains("POSIX shell"), "{linux}");
        assert!(!linux.contains("findstr"), "{linux}");
    }
    fn provider_with(
        protocol: ProviderProtocol,
        capabilities: ProviderModelCapabilities,
    ) -> ProviderManager {
        ProviderManager::new(ResolvedProviderSelection {
            requested_name: "test-provider".to_string(),
            provider_name: "test-provider".to_string(),
            protocol,
            base_url: "https://example.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "TEST_PROVIDER_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "test-model".to_string(),
            temperature: 0.2,
            max_output_tokens: 4096,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities,
            thinking_param_pattern: ThinkingParamPattern::None,
        })
    }

    #[test]
    fn test_persists_native_tool_transcript_predicate_covers_protocol_and_reasoning() {
        let tool_caps = ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_reasoning: false,
            ..Default::default()
        };
        // 非 reasoning OpenAI 工具模型：ADR 0123 写侧必须落盘（流式/同步共用本判定）。
        assert!(
            provider_with(ProviderProtocol::OpenAiCompletions, tool_caps.clone())
                .persists_native_tool_transcript()
        );
        // reasoning 模式仍由 requires_provider_native_tool_flow 兜底。
        assert!(provider_with(
            ProviderProtocol::OpenAiCompletions,
            ProviderModelCapabilities {
                supports_reasoning: true,
                ..tool_caps.clone()
            }
        )
        .persists_native_tool_transcript());
        // Anthropic 保持 FullReplay，写侧不得落 token 级原生工具转录。
        assert!(
            !provider_with(ProviderProtocol::AnthropicMessages, tool_caps.clone())
                .persists_native_tool_transcript()
        );
        // 无工具能力模型不落原生工具转录。
        assert!(!provider_with(
            ProviderProtocol::OpenAiCompletions,
            ProviderModelCapabilities {
                supports_tools: false,
                ..tool_caps
            }
        )
        .persists_native_tool_transcript());
    }

    #[test]
    fn test_anthropic_provider_keeps_full_replay_even_with_native_tool_history() {
        let provider = provider_with(
            ProviderProtocol::AnthropicMessages,
            ProviderModelCapabilities {
                supports_tools: true,
                supports_streaming: true,
                supports_reasoning: false,
                ..Default::default()
            },
        );
        let session = session_snapshot(
            vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "查目录".to_string(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "已查完".to_string(),
                    ..Default::default()
                },
            ],
            vec![
                json!({ "role": "user", "content": "查目录" }),
                json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{ "id": "call_a", "type": "function", "function": { "name": "ls", "arguments": "{}" } }]
                }),
                json!({ "role": "tool", "tool_call_id": "call_a", "content": "a.txt" }),
                json!({ "role": "assistant", "content": "已查完" }),
            ],
            None,
        );

        let builder = DefaultTurnContextBuilder;
        let retrieved = builder.retrieve_context_state("继续", &[], None, &session, None, None);
        let request = builder.build_request("graph-a", &provider, &retrieved, &[]);

        // ADR 0123 协议守卫：Anthropic 不消费修剪后的原生工具转录，保持 FullReplay。
        assert_eq!(
            request.observation.conversation_carry_mode,
            Some(ConversationCarryMode::FullReplay)
        );
    }
}

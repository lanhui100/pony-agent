import type { ProviderReasoningEffort } from "@/types/provider";

export type QueueDeliveryMode = "queue" | "steer";
export type ToolAuthorizationMode = "read_only" | "workspace_write" | "full_access";

export type QueuedMessageItem = {
  id: string;
  sessionId: string;
  content: string;
  mode: QueueDeliveryMode;
  createdAt: number;
};

export type HealthPayload = {
  appName: string;
  appVersion: string;
  runtime: string;
  graphEngine: string;
  graphContractVersion: string;
};

export type RuntimePhase =
  | "idle"
  | "connecting"
  | "ready"
  | "completed"
  | "cancelled"
  /** wave-60 §C7/A2：停止请求已受理（**进行中态，非终态**）。终态仍只有
   * completed/failed/cancelled 三选一；`stopping` 必须有兜底退出（终态/watchdog）。 */
  | "stopping"
  | "calling_model"
  | "calling_tool"
  | "failed"
  | "waiting_user"
  | "suspended";

export type TurnLifecyclePhase =
  | "created"
  | "preparing"
  | "building_context"
  | "calling_model"
  | "streaming_response"
  | "executing_tool"
  | "tool_result_integrating"
  | "checkpointing"
  | "suspended"
  | "waiting_user"
  | "completed"
  | "failed"
  | "cancelled";

export type TurnLifecycleEventType =
  | "turn.created"
  | "turn.phase_changed"
  | "turn.context_built"
  | "turn.model_call_started"
  | "turn.first_token"
  | "turn.output_delta"
  | "turn.output_end"
  | "turn.tool_call_started"
  | "turn.tool_call_completed"
  | "turn.trace_updated"
  | "turn.checkpoint_persisted"
  | "turn.completed"
  | "turn.failed"
  | "turn.cancelled"
  | "turn.suspended";

export type AttachmentReference = {
  id: string;
  assetId?: string | null;
  name?: string | null;
  mimeType: string;
  relativePath?: string | null;
  sizeBytes?: number | null;
  createdAtMs?: number | null;
};

export type AttachmentAssetStatus =
  | "active"
  | "missing_payload"
  | "expired"
  | "reclaimable";

export type AttachmentAsset = {
  id: string;
  sessionId: string;
  name?: string | null;
  mimeType: string;
  relativePath: string;
  sizeBytes: number;
  createdAtMs: number;
  status?: AttachmentAssetStatus;
  referenceCount?: number;
  lastReferencedAtMs?: number | null;
  expiresAtMs?: number | null;
};

export type AttachmentAssetFilter = {
  sessionId?: string | null;
  mimeType?: string | null;
  nameContains?: string | null;
  createdAfterMs?: number | null;
  createdBeforeMs?: number | null;
  statuses?: AttachmentAssetStatus[];
  limit?: number | null;
};

export type AttachmentMeta = AttachmentReference;

export type ToolActivity = {
  id: string;
  name: string;
  canonicalToolName?: string | null;
  displayNameZh?: string | null;
  status: "planned" | "running" | "done" | "error";
  description: string;
  argumentsText?: string | null;
  resultText?: string | null;
  durationSeconds?: number | null;
  parentActivityId?: string | null;
  artifacts?: Array<Record<string, unknown>> | null;
  error?: Record<string, unknown> | null;
  capabilityInvocation?: CapabilityInvocationRecord | null;
};

export type ToolPermissionFacts = {
  requiresApproval?: boolean | null;
  permissionScope?: string | null;
  hostMediated?: boolean | null;
  permissionProfile?: string | null;
  approvalMode?: string | null;
  decisionSource?: string | null;
};

export type ToolDisplayMetadata = {
  displayNameZh?: string | null;
};

export type AvailableTool = {
  name: string;
  canonicalToolName: string;
  executionPrimitive: string;
  description: string;
  kind: string;
  exposure: string;
  displayMetadata: ToolDisplayMetadata;
  permissionFacts: ToolPermissionFacts;
  inputSchema: {
    type?: string;
    properties?: Record<string, { type?: string; description?: string }>;
    required?: string[];
    additionalProperties?: boolean;
  };
};

export type CapabilitySourceKind = "builtin" | "mcp";
export type CapabilityAvailability = "available" | "degraded" | "unreachable" | "disabled";
export type CapabilityKind = "tool" | "resource" | "prompt_template";
export type CapabilityInvocationMode = "direct_tool_call" | "read_only_fetch" | "prompt_expansion";
export type CapabilityFailureKind =
  | "source_unavailable"
  | "permission_denied"
  | "out_of_scope"
  | "malformed_response"
  | "invocation_failed"
  | "capability_not_found";
export type SkillSourceKind = "host" | "mcp";
export type SkillFailureLayer =
  | "skill_resolution"
  | "source_unavailable"
  | "permission_denied"
  | "malformed_composition"
  | "unsupported_composition"
  | "underlying_capability_execution";

export type SourceIngressObservation = {
  boundary: string;
  summary: string;
  candidateIds: string[];
  observedAtMs: number;
};

export type CapabilityInvocationRecord = {
  toolName: string;
  capabilityId?: string | null;
  sourceId?: string | null;
  sourceKind?: CapabilitySourceKind | null;
  capabilityKind?: CapabilityKind | null;
  invocationMode?: CapabilityInvocationMode | null;
  failureKind?: CapabilityFailureKind | null;
  requiresApproval?: boolean | null;
  hostMediated?: boolean | null;
  permissionScope?: string | null;
  permissionFacts?: ToolPermissionFacts | null;
  skillId?: string | null;
  skillSourceId?: string | null;
  composedCapabilityRefs?: string[] | null;
  composedCapabilityKinds?: CapabilityKind[] | null;
  failureLayer?: SkillFailureLayer | null;
};

export type CapabilitySourceView = {
  sourceId: string;
  sourceKind: CapabilitySourceKind;
  displayName: string;
  transportKind: string;
  serverIdentity: string;
  availability: CapabilityAvailability;
  declaredCapabilities: CapabilityKind[];
  permissionProfile: string;
  updatedAtMs: number;
  lastIngressObservation?: SourceIngressObservation | null;
};

export type CapabilityView = {
  capabilityId: string;
  sourceId: string;
  sourceKind: CapabilitySourceKind;
  kind: CapabilityKind;
  label: string;
  canonicalToolName?: string | null;
  displayNameZh?: string | null;
  description: string;
  invocationMode: CapabilityInvocationMode;
  inputSchemaSummary: string;
  safetyClass: string;
  visibility: string;
  observabilityTags: string[];
  requiresApproval: boolean;
  hostMediated: boolean;
  permissionScope: string;
  permissionFacts?: ToolPermissionFacts | null;
};

export type SkillSourceView = {
  sourceId: string;
  sourceKind: SkillSourceKind;
  displayName: string;
  availability: CapabilityAvailability;
  transportKind: string;
  serverIdentity: string;
  updatedAtMs: number;
  lastIngressObservation?: SourceIngressObservation | null;
};

export type SkillDescriptor = {
  skillId: string;
  sourceId: string;
  sourceKind: SkillSourceKind;
  label: string;
  description: string;
  inputSchemaSummary: string;
  safetyClass: string;
  visibility: string;
  observabilityTags: string[];
  requiresApproval: boolean;
  hostMediated: boolean;
  permissionScope: string;
  composedCapabilityRefs: string[];
  composedCapabilityKinds: CapabilityKind[];
  executableInV1: boolean;
};

export type TraceStep = {
  id: string;
  label: string;
  /** `skipped`：该步骤本轮未被触发（如无工具调用的 turn 的 `step-call-tool`）。 */
  state: "completed" | "active" | "pending" | "error" | "cancelled" | "skipped";
};

export type TraceTimelineEntryKind =
  | "input"
  | "prepare_retrieval"
  | "build_context"
  | "call_model"
  | "call_tool"
  | "return_result"
  | "context"
  | "model"
  | "tool"
  | "return";

export type TraceTimelineEntry = {
  id: string;
  kind: TraceTimelineEntryKind;
  label: string;
  state: TraceStep["state"];
  sequence: number;
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  /** PA-094：大字段外置引用（`bco:<turn_id>:<seq>`）。事件折叠重建的
   *  build_context 条目只带引用（全量 payload 按需加载）。 */
  buildContextObservationRef?: string | null;
  toolActivities?: ToolActivity[];
  text?: string | null;
  reasoningContent?: string | null;
  fallbackReason?: string | null;
  error?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  durationMs?: number | null;
  durationSeconds?: number | null;
  turnDurationMs?: number | null;
  /**
   * B016：该 hop 是否由 Run 到期 promote 成后台作业（加性字段，仅 `true` 时写入）。
   *
   * 只做**可见性**：`state` 仍为 `completed`，不引入新状态（V10 冻结快照依赖既有枚举）。
   * 取值统一来自 `parsePromotedToolResult(tool.resultText)`（B011 的唯一解析实现）。
   */
  promoted?: boolean;
};

export type PrefixMutationReason =
  | "session_summary_changed"
  | "run_goal_changed"
  | "long_term_memory_changed"
  | "planner_skills_changed"
  | "image_note_changed"
  | "truncation_note_changed"
  | "history_boundary_shifted"
  | "native_transcript_boundary_shifted"
  | "tool_surface_dynamic";

export type ContextRefreshReason =
  | "initial_build"
  | "session_summary_changed"
  | "run_goal_changed"
  | "long_term_memory_changed"
  | "planner_skills_changed"
  | "image_note_changed"
  | "truncation_note_changed"
  | "history_boundary_shifted"
  | "native_transcript_boundary_shifted"
  | "workspace_scope_changed";

export type ConversationCarryMode =
  | "full_replay"
  | "provider_native_transcript_replay"
  | "provider_continuation_preferred"
  | "provider_continuation_fallback_replay"
  | "compacted_replay";

export type ProviderRequestKind = "initial_request" | "tool_followup";
export type ProviderLatencyKind = "provider_stream" | "buffered_response" | "unknown";

/**
 * 速度口径标记（`.dev-team/contract-matrix-token-speed-v2.md` I10）：描述**测的是什么**。
 * - `stream_decode`：解码窗口口径 `Σ(out−1) / Σgen`（可比）；
 * - `end_to_end`：端到端口径 `Σout / Σdur`（含排队与 prefill，不可与前直接比较）；
 * - `none`：无值哨兵（wire 上缺席 ⇔ 无值，禁止 0 占位）。
 */
export type TurnSpeedBasis = "stream_decode" | "end_to_end" | "none";

export type ProviderCallCacheRecord = {
  requestKind: ProviderRequestKind;
  providerSource?: string | null;
  providerMode?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  cacheHitSource?: string | null;
  cacheMissInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  latencyKind?: ProviderLatencyKind | null;
  prefixMutationReasons?: PrefixMutationReason[];
  /** wave-54 契约 C3（.dev-team/contract-matrix-wave54.md）：usage 来源
   *  `provider` | `estimated_chars`；缺失表示未知（不得静默当真值）。 */
  usageOrigin?: string;
  /** wave-54 契约 C4：上下文占用归一值（anthropic 系 = input + cache_read，
   *  其余协议 = input）；缺失表示无值，UI 显示 `—`。 */
  contextInputTokens?: number;
};

export type BuildContextObservation = {
  requestFormat: string;
  messageCount: number;
  imageCount: number;
  toolCount: number;
  temperature: number;
  maxOutputTokens: number;
  stablePrefixText: string;
  semiStableContextText: string;
  volatileInputText: string;
  prefixMutationReasons?: PrefixMutationReason[];
  contextRefreshReason?: ContextRefreshReason | null;
  instructionScopeSources?: string[];
  conversationCarryMode?: ConversationCarryMode | null;
  requestMessagesText: string;
  toolDefinitionsText: string;
};

export type TurnInputImage = {
  dataUrl: string;
  mimeType: string;
  name?: string | null;
};

export type WorkspaceStateRef = {
  kind: "none" | "git_commit" | "patch_set" | "host_snapshot";
  locator?: string | null;
  rollbackCapable?: boolean;
};

export type HistoryNodeKind =
  | "turn_committed"
  | "turn_cancelled"
  | "run_paused"
  | "checkpoint"
  | "manual_snapshot";

export type HistoryNode = {
  nodeId: string;
  sessionId: string;
  parentNodeId?: string | null;
  branchId: string;
  forkedFromNodeId?: string | null;
  kind: HistoryNodeKind;
  turnId?: string | null;
  transcriptRef?: string | null;
  runRef?: string | null;
  workspaceRef?: WorkspaceStateRef | null;
  summary: string;
  title?: string | null;
  history?: TurnHistoryMessage[];
  turnTraceHistory?: TurnTraceRecord[];
  /** PA-088：持久化层轻量 trace 引用（节点不再内嵌完整 trace 时用于物化）。 */
  turnTraceRefs?: TurnTraceRef[];
  turnCount?: number | null;
  lastReferencedFile?: string | null;
  createdAtMs: number;
};

export type TurnTraceRef = {
  turnId: string;
  updatedAtMs: number;
  version?: number | null;
};

export type HistoryBranch = {
  branchId: string;
  sessionId: string;
  baseNodeId?: string | null;
  headNodeId?: string | null;
  forkedFromBranchId?: string | null;
  forkedFromNodeId?: string | null;
  label?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
};

export type HistoryCursorMode = "live" | "historical" | "historical_dirty";

export type HistoryCursorState = {
  sessionId: string;
  visibleNodeId?: string | null;
  activeBranchId?: string | null;
  branchHeadNodeId?: string | null;
  workspaceNodeId?: string | null;
  mode: HistoryCursorMode;
  authorityMode?: "host_authoritative" | "local_preview" | string;
  cursorVersion?: number | null;
  isAtBranchHead?: boolean;
};

export type HistoryCheckoutMode = "transcript_only" | "transcript_and_workspace";

export type HistoryControlResult = HistoryCursorState & {
  historyNodes?: HistoryNode[];
  historyBranches?: HistoryBranch[];
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
};

export type HistoryCheckoutResult = HistoryControlResult & {
  sessionId: string;
  nodeId: string;
  requestedMode: HistoryCheckoutMode;
  appliedMode: HistoryCheckoutMode;
  transcriptRestoreApplied: boolean;
  workspaceRollbackCapable: boolean;
  workspaceRollbackApplied: boolean;
  degraded: boolean;
  workspaceRestoreCapable?: boolean;
  workspaceRestoreApplied?: boolean;
  degradedToTranscriptOnly?: boolean;
  degradationReason?: string | null;
};

export type HistoryRestoreResult = HistoryControlResult & {
  sessionId: string;
  branchId?: string | null;
  restoredNodeId?: string | null;
  transcriptRestoreApplied: boolean;
  workspaceRollbackCapable: boolean;
  workspaceRollbackApplied: boolean;
  degraded: boolean;
  restoredFromNodeId?: string | null;
  workspaceRestoreCapable?: boolean;
  workspaceRestoreApplied?: boolean;
  degradedToTranscriptOnly?: boolean;
  degradationReason?: string | null;
};

export type HistoryForkResult = HistoryControlResult & {
  sessionId: string;
  nodeId: string;
  branch: HistoryBranch;
  forkedFromNodeId?: string;
  forkedFromBranchId?: string | null;
  createdBranchId?: string;
};

export type HistoryBranchSwitchResult = HistoryControlResult & {
  sessionId: string;
  branchId: string;
  nodeId?: string | null;
  previousBranchId?: string | null;
};

export type ChatMessage = {
  id: string;
  turnId: string;
  role: "user" | "assistant" | "tool";
  content: string;
  attachments?: AttachmentMeta[];
  reasoningContent?: string | null;
  status?: "pending" | "done" | "error";
  modelName?: string | null;
  tokenCount?: number | null;
  toolName?: string | null;
  canonicalToolName?: string | null;
  displayNameZh?: string | null;
  detail?: string | null;
  durationSeconds?: number | null;
  errorDetail?: string | null;
  /** PA-114: 后端 ToolActivity.id == call_id，透传用于 Ask 卡片精确绑定 pending ask。 */
  callId?: string | null;
  /** PA-114: 工具调用参数 JSON 字符串（如 ask_user 的 question/options）。 */
  argumentsText?: string | null;
};

export type MessageStateEntry = {
  messageId: string;
  turnId: string;
  role: "user" | "assistant" | "tool" | string;
  content: string;
  attachments?: AttachmentMeta[];
  status?: "pending" | "done" | "error" | string | null;
  reasoningContent?: string | null;
  modelName?: string | null;
  tokenCount?: number | null;
  toolName?: string | null;
  canonicalToolName?: string | null;
  displayNameZh?: string | null;
  detail?: string | null;
  durationSeconds?: number | null;
  errorDetail?: string | null;
};

export type MessageStateSnapshot = {
  sessionId: string;
  revision: string;
  messages: MessageStateEntry[];
};

export type MessageDeltaOp =
  | { kind: "truncateAfter"; messageId?: string | null }
  | { kind: "append"; messages: MessageStateEntry[] }
  | { kind: "replaceAll"; messages: MessageStateEntry[] };

export type MessageStateDelta = {
  sessionId: string;
  baseRevision: string;
  targetRevision: string;
  ops: MessageDeltaOp[];
};

export type HookClass = "observe" | "guard" | "transform" | "side_effect";

export type TurnHookPoint =
  | "turn_prepare_start"
  | "turn_prepare_end"
  | "context_build_start"
  | "context_build_end"
  | "model_call_start"
  | "model_response_end"
  | "tool_call_start"
  | "tool_call_end"
  | "checkpoint_persist_start"
  | "checkpoint_persist_end"
  | "turn_finalize_start"
  | "turn_finalize_end"
  | "planner_turn_preflight"
  | "planner_tool_selection"
  | "planner_graph_decision"
  | "capability_resolve"
  | "skill_tool_actions_resolve";

export type HookResultKind = "observe" | "allow" | "deny" | "patch" | "side_effect_request";

export type HookPatchTarget =
  | "turn_input"
  | "context_payload"
  | "model_request"
  | "memory_write_intent"
  | "planner_facts"
  | "capability_mediation"
  | "tool_arguments"
  | "tool_result"
  | "checkpoint_metadata"
  | "turn_output";

export type HookPatchOperationKind = "set" | "merge" | "remove";

export type HookPatchOperation = {
  target: HookPatchTarget;
  path: string;
  operation: HookPatchOperationKind;
  valueSummary?: string | null;
  valueText?: string | null;
};

export type HookDenyDecision = {
  reasonCode: string;
  message: string;
};

export type HookSideEffectRequest = {
  requestKind: string;
  summary: string;
  requiresPersistenceEvidence: boolean;
};

export type PersistedEffectEvidence = {
  evidenceId: string;
  effectKind: string;
  boundary: string;
  targetSessionId?: string | null;
  sourceHistoryNodeId?: string | null;
  targetSummary: string;
  persistenceRef: string;
  replayDecisionBasis: string;
  persistedAtMs: number;
  replayRequiredIfMissing: boolean;
};

export type HistoryStateHookEvidence = {
  evidenceId: string;
  sessionId: string;
  boundary: string;
  commandKind: string;
  resultKind: string;
  summary: string;
  elapsedMs: number;
  blocked: boolean;
  degraded: boolean;
  requestedNodeId?: string | null;
  requestedBranchId?: string | null;
  resolvedNodeId?: string | null;
  resolvedBranchId?: string | null;
  recordedAtMs: number;
};

export type HistoryStateAuditActionSummary = {
  status: "available" | "missing" | string;
  sourceFamily: "history_state" | string;
  commandKind?: string | null;
  boundary?: string | null;
  resultKind?: string | null;
  summary: string;
  elapsedMs?: number | null;
  blocked: boolean;
  degraded: boolean;
  evidenceId?: string | null;
  observedAtMs?: number | null;
  requestedNodeId?: string | null;
  requestedBranchId?: string | null;
  resolvedNodeId?: string | null;
  resolvedBranchId?: string | null;
};

export type HistoryStateAuditCurrentContext = {
  mode: HistoryCursorMode | string;
  visibleNodeId?: string | null;
  activeBranchId?: string | null;
  branchHeadNodeId?: string | null;
  workspaceNodeId?: string | null;
};

export type HistoryStateAuditSummary = {
  action: HistoryStateAuditActionSummary;
  currentContext: HistoryStateAuditCurrentContext;
};

export type ConversationCheckpointForkTarget = {
  branchId: string;
  nodeId: string;
  label: string;
  summary: string;
  isActive: boolean;
};

export type ConversationCheckpointEntry = {
  nodeId: string;
  turnId: string;
  branchId: string;
  summary: string;
  createdAtMs: number;
  isLatest: boolean;
  isVisible: boolean;
  workspaceRollbackCapable: boolean;
  availableModes: HistoryCheckoutMode[];
  forkTargets: ConversationCheckpointForkTarget[];
};

export type RunControlAuditActionSummary = {
  status: "available" | "missing" | string;
  sourceFamily: "run_control" | string;
  commandKind?: string | null;
  boundary?: string | null;
  resultKind?: string | null;
  summary: string;
  targetSummary: string;
  elapsedMs?: number | null;
  blocked: boolean;
  degraded: boolean;
  evidenceId?: string | null;
  observedAtMs?: number | null;
  runId?: string | null;
  turnId?: string | null;
  checkpointTurnId?: string | null;
  checkpointKind?: string | null;
  recoveryMode?: string | null;
  projectedCommand?: string | null;
  degradationReason?: string | null;
  requestSummary?: string | null;
  startReason?: "initial_turn" | "replay_from_checkpoint" | "restart_from_checkpoint" | string | null;
};

export type RunControlAuditCurrentContext = {
  phase: string;
  checkpointStatus: string;
  activeRunId?: string | null;
  checkpointKind?: string | null;
  checkpointRecoveryMode?: string | null;
  submissionPlanCommand?: string | null;
};

export type RunControlAuditSummary = {
  actionEvidenceSummary: RunControlAuditActionSummary;
  currentContextProjection: RunControlAuditCurrentContext;
};

export type HookStructuredResult =
  | { resultKind: "observe"; payload: { summary: string } }
  | { resultKind: "allow"; payload: { summary: string } }
  | { resultKind: "deny"; payload: HookDenyDecision }
  | { resultKind: "patch"; payload: { operations: HookPatchOperation[] } }
  | { resultKind: "side_effect_request"; payload: HookSideEffectRequest };

export type HookTraceRecord = {
  hookName: string;
  hookClass: HookClass;
  hookPoint: TurnHookPoint;
  hookOrder: number;
  resultKind: HookResultKind;
  structuredResult: HookStructuredResult;
  blocked: boolean;
  elapsedMs: number;
  inputSummary?: string | null;
  persistenceEvidenceRef?: string | null;
  summary: string;
};

export type MemoryWriteIntentRecord = {
  key: string;
  kind: string;
  content: string;
  contentSummary: string;
  source: string;
  operation: "insert" | "update" | "noop" | string;
};

export type TurnTraceRecord = {
  turnId: string;
  sessionId?: string | null;
  eventId?: string | null;
  eventType?: string | null;
  eventVersion?: string | null;
  sequence?: number | null;
  emittedAtMs?: number | null;
  title: string;
  phase: RuntimePhase;
  traceSteps: TraceStep[];
  traceTimeline?: TraceTimelineEntry[];
  traceId?: string | null;
  toolActivities: ToolActivity[];
  providerCallRecords?: ProviderCallCacheRecord[];
  hookTraceRecords?: HookTraceRecord[];
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  /** PA-094：大字段外置引用（`bco:<turn_id>:<seq>`）。新数据走引用（按需加载），
   *  legacy 内嵌数据保留 buildContextObservation 读取兼容。 */
  buildContextObservationRef?: string | null;
  /** C2：本轮是否降级（有工具失败但轮次正常收尾）；后端仅在 `true` 时写键。 */
  degraded?: boolean;
  /** C2：本轮工具失败次数；后端仅在 `> 0` 时写键。 */
  toolFailures?: number;
  sessionSummary?: string | null;
  fallbackReason?: string | null;
  error?: string | null;
  /**
   * PA-172 §6 T2：失败归类（wire 对齐 Rust `TurnTraceRecord.error_class`，
   * `#[serde(default, skip_serializing_if = "Option::is_none")]` ⇒ 键缺席 = 非失败轮）。
   * 取值封闭集 `timeout|rate_limited|quota|network|auth|bad_request|other`，
   * 唯一来源 `agent::retry::classify_error_class`（禁止前端再建第二套分类）。
   */
  errorClass?: string;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  costUsd?: number | null;
  /** wave-54 契约 C3（.dev-team/contract-matrix-wave54.md）：usage 来源
   *  `provider` | `estimated_chars`；缺失表示未知（`estimated_chars` 时 UI 需加 `≈`）。 */
  usageOrigin?: string;
  /** wave-54 契约 C4：上下文占用归一值（协议归一口径，缺失 ⇒ UI `—`）。 */
  contextInputTokens?: number;
  updatedAt: number;
};

export type TurnInput = {
  message: string;
  displayMessage?: string | null;
  providerId?: string | null;
  modelId?: string | null;
  reasoningEffort?: ProviderReasoningEffort | null;
  workspaceMode?: "coding" | "work" | null;
  toolAuthorizationMode?: ToolAuthorizationMode | null;
  sessionId?: string | null;
  nodeId?: string | null;
  history?: TurnHistoryMessage[];
  images?: TurnInputImage[];
  /** Workspace 归属（PA-079）：携带时在会话首次持久化盖章 */
  workspaceId?: string | null;
  /** W36 / PA-141 审批证据 */
  planExecutionApproval?: Record<string, unknown> | null;
};

export type TurnHistoryMessage = {
  role: "user" | "assistant";
  content: string;
  attachments?: AttachmentMeta[];
  turnId?: string | null;
  status?: "done" | "error" | null;
  modelName?: string | null;
  tokenCount?: number | null;
  reasoningContent?: string | null;
  /**
   * 合成记录种类（后端 `HistoryMessageKind`）：`cancelled` = 用户终止，
   * `interrupted` = 上游失败/崩溃收口。缺省表示模型真实产出或旧会话。
   * 展示层据此渲染一行状态，而不是把上下文载体当正文直出。
   */
  kind?: "cancelled" | "interrupted" | null;
};

export type SessionOverview = {
  conversationId: string;
  title?: string | null;
  summary: string;
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
  /** Workspace 归属（PA-079）：None → 默认 workspace */
  workspaceId?: string | null;
  /** 归档标记（侧边栏三级树）：后端恒投影；本地构造可不带（optional 兼容）。 */
  archived?: boolean;
};

export type SessionSnapshot = {
  conversationId: string;
  title?: string | null;
  summary: string;
  history: TurnHistoryMessage[];
  attachmentAssets?: AttachmentAsset[];
  turnTraceHistory?: TurnTraceRecord[];
  longTermMemoryEntries?: LongTermMemoryEntry[];
  memoryWriteEvidence?: PersistedEffectEvidence[];
  historyStateEvidence?: HistoryStateHookEvidence[];
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  runControlAuditSummary?: RunControlAuditSummary | null;
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
  /** Workspace 归属投影（PA-079）：None → 默认 workspace */
  workspaceId?: string | null;
};

export type TurnContext = {
  userMessage: string;
  images: TurnInputImage[];
  referencesImage: boolean;
};

export type SessionContext = {
  conversationId: string;
  title: string;
  summary: string;
  recentHistory: TurnHistoryMessage[];
  recentAttachmentAssets: AttachmentAsset[];
  turnCount: number;
  lastReferencedFile?: string | null;
  contextWindowTokens?: number | null;
};

export type RunState = {
  runId?: string | null;
  goal?: string | null;
  phase?: string | null;
  activeTurnId?: string | null;
  lastCompletedTurnId?: string | null;
  resumeCount?: number | null;
  lastDecisionSummary?: string | null;
  executionCheckpointStatus?: string | null;
  executionCheckpointPhase?: string | null;
};

export type LongTermMemoryEntry = {
  kind: string;
  content: string;
  source: string;
  updatedAtMs: number;
};

export type LongTermMemory = {
  status: string;
  summary?: string | null;
  entries: LongTermMemoryEntry[];
};

export type TranscriptContext = {
  providerNativeMessages: unknown[];
};

export type RetrievedContextState = {
  turnContext: TurnContext;
  sessionContext: SessionContext;
  runState: RunState;
  longTermMemory: LongTermMemory;
  transcript: TranscriptContext;
};

export type GraphRunPhase =
  | "ready"
  | "running"
  | "waiting_user"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export type GraphDecision = {
  kind: string;
  reason: string;
  summary: string;
  targetPhase: GraphRunPhase;
};

export type GraphStep = {
  id: string;
  kind?: string | null;
  turnId?: string | null;
  sessionId?: string | null;
  phase: GraphRunPhase;
  title: string;
  updatedAtMs: number;
};

export type GraphTurnHandoff = {
  contractVersion: string;
  turnId?: string | null;
  sessionId?: string | null;
  turnPhase: string;
  checkpointStatus?: string | null;
  checkpointPhase?: string | null;
  userMessage: string;
  assistantMessage: string;
  sessionSummary: string;
  conversationId: string;
  sessionTurnCount: number;
  runId?: string | null;
  runPhase?: string | null;
  activeTaskFocus?: string | null;
  acceptanceFocus?: string | null;
  lastReferencedFile?: string | null;
  recentAttachmentAssetCount: number;
  longTermMemoryStatus: string;
  longTermMemoryEntryCount: number;
  traceStepCount: number;
  toolActivityCount: number;
  providerName: string;
  providerModel: string;
};

export type GraphRun = {
  id: string;
  goal: string;
  sessionId?: string | null;
  phase: GraphRunPhase;
  steps: GraphStep[];
  activeTurnId?: string | null;
  lastCompletedTurnId?: string | null;
  stopReason?: string | null;
  lastHandoff?: GraphTurnHandoff | null;
  resumeCount: number;
  lastDecision?: GraphDecision | null;
  controlBoundaryEvidence: GraphRunControlBoundaryEvidence[];
  createdAtMs: number;
  updatedAtMs: number;
};

export type GraphRunControlBoundaryEvidence = {
  hookPoint: string;
  canonicalEventType: string;
  canonicalPhase: string;
  summary: string;
  hookEnvelope: RunControlHookEnvelope;
  createdAtMs: number;
};

export type GraphRunCheckpoint = {
  contractVersion: string;
  runId: string;
  goal: string;
  sessionId?: string | null;
  phase: GraphRunPhase;
  activeTurnId?: string | null;
  lastCompletedTurnId?: string | null;
  stopReason?: string | null;
  steps: GraphStep[];
  lastDecision?: GraphDecision | null;
  lastHandoff?: GraphTurnHandoff | null;
  resumeCount: number;
  controlBoundaryEvidence: GraphRunControlBoundaryEvidence[];
  resumable: boolean;
  createdAtMs: number;
  updatedAtMs: number;
};

export type GraphRunEvent = {
  runId: string;
  kind: string;
  phase: GraphRunPhase;
  summary: string;
  stepCount: number;
  updatedAtMs: number;
  hookPoint?: string | null;
  canonicalEventType?: string | null;
  canonicalPhase?: string | null;
};

export type GraphRunTurnResponse = {
  run: GraphRun;
  handoff: GraphTurnHandoff;
  decision: GraphDecision;
  event: GraphRunEvent;
};

export type GraphRunControlResponse = {
  run: GraphRun;
  event: GraphRunEvent;
  controlBoundaryEvidence?: GraphRunControlBoundaryEvidence | null;
  runControlAuditSummary?: RunControlAuditSummary | null;
};

export type GraphRunStreamStartResponse = {
  run: GraphRun;
  event: GraphRunEvent;
  controlBoundaryEvidence?: GraphRunControlBoundaryEvidence | null;
  runControlAuditSummary?: RunControlAuditSummary | null;
  turnId: string;
};

export type TurnResult = {
  eventId?: string | null;
  eventType?: string | null;
  eventVersion?: string | null;
  sequence?: number | null;
  emittedAtMs?: number | null;
  phase: RuntimePhase;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  reasoningContent?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  userMessage: string;
  assistantMessage: string;
  traceSteps: TraceStep[];
  traceTimeline?: TraceTimelineEntry[];
  toolActivities: ToolActivity[];
  providerCallRecords?: ProviderCallCacheRecord[];
  hookTraceRecords?: HookTraceRecord[];
  sessionSummary: string;
};

/**
 * wave-metrics（task-4）：per-call 指标结算快照（wire 对齐 Rust
 * `crates/pony-agent-core/src/agent/metrics_snapshot.rs::TurnMetricsSnapshot`；
 * 契约源 `.dev-team/contract-matrix-metrics-snapshot.md` §4 字段表，camelCase 逐字段一致）。
 *
 * 纪律（契约 §0）：
 * - 快照存在 ⇒ **快照是唯一真值来源**，消费端禁止各自重算（I5/I6）；
 * - `cacheHitRatio` / `contextRatio` 是 **`0..=1` 单位分数**（非百分数），缺失 ⇒ `null`（I2/I3）；
 * - 旧事件缺 `metrics` ⇒ 消费端回退既有 trace 反推路径（I7）。
 */
export interface TurnMetricsSnapshot {
  /** 逻辑 hop 索引（调用方给定）。 */
  step: number;
  /** 协议归一后的上下文输入（I4）。 */
  contextInputTokens: number | null;
  /** 末次调用裸 input。 */
  inputTokens: number | null;
  cacheHitInputTokens: number | null;
  cacheMissInputTokens: number | null;
  outputTokens: number | null;
  reasoningTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  durationMs: number | null;
  callSpeedTokensPerSecond: number | null;
  /** 单调用速度口径（I8/I10）；wire 缺席 ⇔ 无值。 */
  callSpeedBasis?: TurnSpeedBasis | null;
  /** 与 Rust `weighted_turn_speed` 同源（I4/I5）。 */
  turnSpeedTokensPerSecond: number | null;
  /** 轮级速度口径（I10）：`stream_decode` | `end_to_end`；缺席 ⇔ 无 speed。 */
  turnSpeedBasis?: TurnSpeedBasis | null;
  /** 轮累计输出（**参与计算集合** Σout，I7）。 */
  turnOutputTokens: number;
  /** 轮累计时长（毫秒，**参与计算集合**）：`stream_decode` ⇒ Σgen；`end_to_end` ⇒ Σdur（I7）。 */
  turnGenerationMs: number;
  /** 参与计算的调用数（I7：`stream_decode` ⇒ |eligible|；`end_to_end` ⇒ |valid|）。 */
  turnCalls: number;
  /**
   * F1 取证（与 Rust `TurnMetricsSnapshot` 同名字段）：该轮 provider 调用**总数**
   * （含无效/不可解码记录）。旧 payload 无该键 ⇒ 缺失（**不得**当 0 次调用）。
   */
  turnCallsTotal?: number | null;
  /** F1 取证：`decode_eligible` 记录数（进入解码口径分子/分母的样本）。 */
  turnCallsEligible?: number | null;
  /**
   * F1 取证：`valid − decode_eligible`——因突发整包/无 TTFT/单 token 被剔除、
   * 只能进 `end_to_end` 回退的调用数。`> 0` ⇒ 显示端必须可辨别（F1 I4）。
   */
  turnBurstExcluded?: number | null;
  /** 轮级 Σ 上下文输入（逐条 per-call 归一值相加；会话累计的唯一原料，禁止用末次调用冒充）。 */
  turnContextInputTokens: number;
  /** 轮级 Σ 缓存命中输入（只累加**上报了**命中字段的调用）。 */
  turnCacheHitInputTokens: number;
  /** 轮级上报了缓存命中的调用数（会话命中率分母可信度判据）。 */
  turnCallsWithCache: number;
  /** `0..=1` 单位分数（I2）。 */
  cacheHitRatio: number | null;
  contextWindowTokens: number | null;
  /** `0..=1` 单位分数（I3）。 */
  contextRatio: number | null;
  /** usage 来源：`provider` / `estimated_chars`。 */
  origin: string | null;
  provider: string;
  model: string;
  /** 该轮成本（未知 ⇒ `null`，禁止以 0 伪造）。 */
  costUsd: number | null;
}

export type TurnStreamEvent = {
  eventId?: string | null;
  sessionId?: string | null;
  turnId: string;
  /**
   * wave-60 §C2 追加值：`"stop_requested"` = 停止请求已受理的**即时事件**（非终态，
   * 终态仍只有 completed/failed/cancelled 三选一）。契约纪律：只能追加，禁止重排/删除既有值。
   */
  kind: "started" | "delta" | "trace" | "tool" | "output_end" | "completed" | "failed" | "cancelled" | "suspended" | "stop_requested";
  eventType?: TurnLifecycleEventType | string | null;
  eventVersion?: string | null;
  sequence?: number | null;
  emittedAtMs?: number | null;
  phase?: RuntimePhase | string | null;
  text?: string | null;
  reasoningContent?: string | null;
  error?: string | null;
  /**
   * PA-172 §6 T2：失败归类（wire 对齐 Rust `TurnStreamEvent.error_class`，camelCase `errorClass`，
   * `#[serde(default, skip_serializing_if = "Option::is_none")]` ⇒ 非失败事件键缺席）。
   * 取值封闭集 `timeout|rate_limited|quota|network|auth|bad_request|other`，
   * 唯一来源 `agent::retry::classify_error_class`（审计机判据，禁止第二套关键词表）。
   */
  errorClass?: string;
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  fallbackReason?: string | null;
  buildContextObservation?: BuildContextObservation | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  traceSteps?: TraceStep[] | null;
  traceTimeline?: TraceTimelineEntry[] | null;
  toolActivities?: ToolActivity[] | null;
  providerCallRecords?: ProviderCallCacheRecord[] | null;
  hookTraceRecords?: HookTraceRecord[] | null;
  sessionSummary?: string | null;
  traceId?: string | null;
  costUsd?: number | null;
  /**
   * wave-metrics（task-4）：per-call 指标结算快照（wire 对齐 Rust
   * `TurnStreamEvent.metrics`，`#[serde(default, skip_serializing_if = "Option::is_none")]`）。
   *
   * - `undefined`（键缺席）/ `null` ⇒ 旧事件线缆，消费端回退既有 trace 反推路径（I7）；
   * - 非空 ⇒ 桌面端以快照为唯一展示来源（契约 §0，禁止各自重算）。
   */
  metrics?: TurnMetricsSnapshot | null;
  /**
   * F9：逻辑 hop 索引（0-based，wire 对齐 Rust `TurnStreamEvent.step`：
   * crates/pony-agent-core/src/agent/runtime/types.rs:190）。
   * delta/chunk 携带所属 hop；`turn:trace(calling_model)` 携带即将发起的 followup hop 索引。
   * 缺省 = 未知/step 0（Rust 侧 `#[serde(default)]`）。
   */
  step?: number | null;
  /**
   * wave-60 §1/§C3：结构化停止原因（wire 对齐 Rust `TurnStreamEvent.stop_reason`）。
   * 追加字段：旧 payload 缺省 ⇒ `null`（Rust 侧 `#[serde(default)]`）。
   * `turn:cancelled` 携带该轮原因，`turn:completed`/`turn:failed` ⇒ `null`。
   */
  stopReason?: StopReason | null;
};

/**
 * 桌面端上游重试进度事件 `turn:retry` 的负载（wire 对齐 Rust
 * `src-tauri/src/tauri_adapter.rs::TurnRetryPayload`，camelCase）。
 *
 * 真值源与 TUI 状态行是**同一个**：`crates/pony-agent-core/src/agent/retry.rs::RetryNotice`
 * （只报告真实发生的重试；`attempt` = 已失败尝试数，1 起；`maxAttempts` = 本跳允许的尝试总数，
 * 含首次 ⇒ 口径 `attempt/maxAttempts`）。前端不得自行推算分母 —— 历史上伪造「重试中(1/5)」
 * 正是因为拿不到真实分母。
 *
 * 纪律：本事件**不是**回合生命周期事件（不进 `TurnStreamEvent` 状态机，不参与终态判定），
 * 仅用于状态行展示；归属不匹配的负载由 store 丢弃。
 */
export type TurnRetryEvent = {
  turnId: string;
  sessionId: string;
  attempt: number;
  maxAttempts: number;
  /** 本次退避将 sleep 的毫秒数（尚未执行）。 */
  delayMs: number;
  /** 失败分类，封闭集同 `agent::retry::classify_error_class`。 */
  errorClass: string;
};

/**
 * wave-60 §1 结构化停止原因（唯一词汇表，与 Rust
 * `crates/pony-agent-core/src/agent/stop_escalation.rs` 的 `StopReason` 同名同值）。
 * 契约纪律：只能追加，禁止重命名/删除既有值。
 */
export type StopReason = "user" | "parent" | "hook" | "watchdog" | "legacy";

/**
 * wave-60 §C1：`stop_turn` 应答（wire 对齐 Rust `StopTurnResponse`）。
 *
 * `state` 是**封闭集合** `{"stopping","not_found","already_terminal"}`；旧的 `"running"` 已废弃。
 * `accepted=false` ⇒ 不得把前端呈现切到 stopping（§C7 的「被接受后」判据）。
 */
export type StopTurnResponse = {
  turnId: string;
  accepted: boolean;
  state: "stopping" | "not_found" | "already_terminal";
  stopReason?: StopReason | null;
};

export type ExecutionCheckpoint = {
  contractVersion?: string | null;
  turnId: string;
  sessionId?: string | null;
  runId?: string | null;
  checkpointKind: "runtime_control" | "recovery" | string;
  recoveryMode?: "replay_required" | "persisted_effect" | string | null;
  projectedRuntimePhase?: RuntimePhase | string | null;
  submissionCommand?:
    | "start_graph_run_stream"
    | "resume_graph_run_stream"
    | "continue_graph_run_stream"
    | string
    | null;
  resumable: boolean;
  replayable: boolean;
  status: string;
  phase: string;
  providerRequestedName?: string | null;
  providerName?: string | null;
  providerProtocol?: string | null;
  providerModel?: string | null;
  providerSource?: string | null;
  providerMode?: string | null;
  fallbackReason?: string | null;
  completedHops: number;
  maxHops: number;
  activeToolName?: string | null;
  traceSteps: TraceStep[];
  toolActivities: ToolActivity[];
  persistedEffectEvidence?: PersistedEffectEvidence[];
  error?: string | null;
  startedAtMs: number;
  updatedAtMs: number;
  stopRequestedAtMs?: number | null;
};

export type SessionRuntimeView = {
  session: SessionSnapshot;
  messageState?: MessageStateSnapshot | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  runControlAuditSummary?: RunControlAuditSummary | null;
  retrieved: RetrievedContextState;
  checkpoint?: ExecutionCheckpoint | null;
  submissionPlan?: GraphRunSubmissionPlan | null;
  controlBoundaryEvidence?: GraphRunControlBoundaryEvidence[] | null;
  historyNodes?: HistoryNode[];
  historyBranches?: HistoryBranch[];
  historyCursor?: HistoryCursorState | null;
  authorityMode?: "host_authoritative" | "local_preview" | string;
  resolvedVisibleNodeId?: string | null;
  activeBranchHeadNodeId?: string | null;
  isAtBranchHead?: boolean;
  cursorVersion?: number | null;
};

export type GraphRunSubmissionPlan = {
  command: "start_graph_run_stream" | "resume_graph_run_stream" | "continue_graph_run_stream" | string;
  runId?: string | null;
  source?: "checkpoint" | "graph_run" | "default" | string | null;
  hookPoint?: "submission_plan_resolved" | string;
  canonicalEventType?: "graph_run.submission_plan_resolved" | string;
  canonicalPhase?: "ready" | "waiting_user" | "paused" | string;
  hookEnvelope?: RunControlHookEnvelope | null;
};

export type RunControlHookEnvelope = {
  sessionId?: string | null;
  runId?: string | null;
  phase: string;
  command: "start_graph_run_stream" | "continue_graph_run_stream" | "resume_graph_run_stream" | "stop_graph_run" | string;
  source: string;
  checkpointKind?: string | null;
  recoveryMode?: string | null;
  resumable: boolean;
  replayable: boolean;
};

export type ModelMonitorOverview = {
  sessionCount: number;
  requestCount: number;
  modelCallCount: number;
  toolCallCount: number;
  hookCallCount: number;
  blockedHookCount: number;
  failedRequestCount: number;
  retrievalParticipationCount: number;
  inputTokens: number;
  cacheHitInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  avgFirstTokenLatencyMs?: number | null;
  avgTurnDurationMs?: number | null;
  avgHookDurationMs?: number | null;
  totalHookDurationMs: number;
};

export type ModelMonitorDimensionRow = {
  key: string;
  label: string;
  requestCount: number;
  modelCallCount: number;
  failedRequestCount: number;
  retrievalParticipationCount: number;
  inputTokens: number;
  cacheHitInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  avgFirstTokenLatencyMs?: number | null;
  avgTurnDurationMs?: number | null;
};

export type ModelMonitorToolRow = {
  key: string;
  label: string;
  callCount: number;
  failedCallCount: number;
  avgDurationMs?: number | null;
  totalDurationMs: number;
};

export type ModelMonitorActivityRow = {
  key: string;
  label: string;
  callCount: number;
  failedCallCount: number;
  avgDurationMs?: number | null;
  totalDurationMs: number;
};

export type ModelMonitorHookRow = {
  key: string;
  label: string;
  callCount: number;
  blockedCallCount: number;
  avgDurationMs?: number | null;
  totalDurationMs: number;
};

export type ModelMonitorSessionRow = {
  sessionId: string;
  title: string;
  summary: string;
  updatedAtMs: number;
  requestCount: number;
  modelCallCount: number;
  toolCallCount: number;
  hookCallCount: number;
  blockedHookCount: number;
  failedRequestCount: number;
  retrievalParticipationCount: number;
  inputTokens: number;
  cacheHitInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  avgFirstTokenLatencyMs?: number | null;
  avgTurnDurationMs?: number | null;
  avgHookDurationMs?: number | null;
  totalHookDurationMs: number;
};

export type ModelMonitorSummaryView = {
  overview: ModelMonitorOverview;
  providers: ModelMonitorDimensionRow[];
  models: ModelMonitorDimensionRow[];
  tools: ModelMonitorToolRow[];
  hookClasses: ModelMonitorHookRow[];
  hooks: ModelMonitorHookRow[];
  capabilitySources: ModelMonitorActivityRow[];
  capabilityInvocationModes: ModelMonitorActivityRow[];
  capabilityFailureClasses: ModelMonitorActivityRow[];
  skillSelections: ModelMonitorActivityRow[];
  skillSources: ModelMonitorActivityRow[];
  skillFailureLayers: ModelMonitorActivityRow[];
  sessions: ModelMonitorSessionRow[];
  generatedAtMs: number;
};

export type ModelMonitorSessionDrilldownView = {
  sessionId: string;
  metrics: ModelMonitorSessionRow;
  runtimeView: SessionRuntimeView;
};

export type HostInspectionSnapshot = {
  surface?: string | null;
  turn?: ExecutionCheckpoint | null;
  session?: SessionSnapshot | null;
  retrieved?: RetrievedContextState | null;
  sessions?: SessionOverview[] | null;
  run?: GraphRun | null;
  runs?: GraphRun[] | null;
};

export type FrontendTraceValue = string | number | boolean | null;

export type FrontendTraceData = Record<string, FrontendTraceValue>;

export type FrontendTraceEventKind =
  | "span"
  | "instant"
  | "counter"
  | "sample"
  | "stall"
  | "snapshot";

export type FrontendTraceScope = "global" | "pre-session" | "session" | "turn";

export type FrontendStallLevel = "light" | "medium" | "heavy";

export type FrontendTraceEvent = {
  id?: number | null;
  seq: number;
  tsWallMs: number;
  tsPerfMs: number;
  sessionId: string | null;
  turnId: string | null;
  scope: FrontendTraceScope;
  category: string;
  name: string;
  kind: FrontendTraceEventKind;
  durationMs?: number | null;
  stallLevel?: FrontendStallLevel | null;
  triggerKind?: string | null;
  data?: FrontendTraceData | null;
  truncated?: boolean;
};

export type FrontendStallSnapshot = {
  id?: number | null;
  sessionId: string | null;
  turnId: string | null;
  tsWallMs: number;
  tsPerfMs: number;
  stallLevel: FrontendStallLevel;
  triggerKind: string;
  stallGapMs: number;
  snapshot: FrontendTraceData;
  truncated?: boolean;
};

export type FrontendTraceQuery = {
  sessionId?: string | null;
  turnId?: string | null;
  fromWallMs?: number | null;
  toWallMs?: number | null;
  limit?: number | null;
  cursor?: string | null;
};

export type FrontendTraceQueryResult = {
  events: FrontendTraceEvent[];
  nextCursor: string | null;
  hasMore: boolean;
  limit: number;
};

export type FrontendStallSnapshotQuery = {
  sessionId?: string | null;
  turnId?: string | null;
  fromWallMs?: number | null;
  toWallMs?: number | null;
  limit?: number | null;
};

export type FrontendTraceExportFormat = "json" | "chrome-trace";

export type FrontendTraceExportPayload = {
  format: FrontendTraceExportFormat;
  fileName: string;
  content: string;
  truncated: boolean;
  eventCount: number;
  snapshotCount: number;
  fromWallMs?: number | null;
  toWallMs?: number | null;
};

export type FrontendRecorderCapabilitySnapshot = {
  tauriAvailable: boolean;
  persistenceAvailable: boolean;
  performanceObserverAvailable: boolean;
  longtaskAvailable: boolean;
  requestIdleCallbackAvailable: boolean;
  initializedAtMs: number;
};

export type FrontendRecorderStats = {
  initialized: boolean;
  activeSessionId: string | null;
  activeTurnId: string | null;
  seq: number;
  bufferedEventCount: number;
  bufferedSnapshotCount: number;
  flushCount: number;
  flushFailureCount: number;
  droppedEventCount: number;
  droppedSnapshotCount: number;
  stallCount: number;
  lastStallGapMs: number | null;
  lastFlushDurationMs: number | null;
  lastFlushAtMs: number | null;
  lastError: string | null;
  lastExportAtMs: number | null;
};

export function normalizeGraphRunPhase(phase?: string | null): GraphRunPhase | null {
  const normalized = phase?.trim().toLowerCase() ?? "";
  if (
    normalized === "ready" ||
    normalized === "running" ||
    normalized === "waiting_user" ||
    normalized === "paused" ||
    normalized === "completed" ||
    normalized === "failed" ||
    normalized === "cancelled"
  ) {
    return normalized;
  }

  return null;
}

export function deriveGraphRunFromRunState(
  runState: RunState | null | undefined,
  currentSessionId: string,
  updatedAtMs: number,
  options?: { activeTaskFocus?: string | null }
): GraphRun | null {
  const runId = runState?.runId?.trim() ?? "";
  const phase = normalizeGraphRunPhase(runState?.phase);
  if (!runId || !phase) {
    return null;
  }

  const activeTaskFocus = options?.activeTaskFocus?.trim() || null;
  const trackedTurnId = runState?.activeTurnId?.trim() || runState?.lastCompletedTurnId?.trim() || "";

  return {
    id: runId,
    goal: runState?.goal?.trim() || runId,
    sessionId: currentSessionId,
    phase,
    steps: trackedTurnId
      ? [
          {
            id: `${runId}:turn`,
            turnId: trackedTurnId,
            sessionId: currentSessionId,
            phase,
            title: runState?.lastDecisionSummary?.trim() || runState?.goal?.trim() || runId,
            updatedAtMs
          }
        ]
      : [],
    activeTurnId: runState?.activeTurnId?.trim() || null,
    lastCompletedTurnId: runState?.lastCompletedTurnId?.trim() || null,
    stopReason: null,
    lastHandoff: trackedTurnId
      ? {
          contractVersion: "retrieved-run-state@v1",
          turnId: trackedTurnId,
          sessionId: currentSessionId,
          turnPhase: phase,
          checkpointStatus: runState?.executionCheckpointStatus?.trim() || null,
          checkpointPhase: runState?.executionCheckpointPhase?.trim() || null,
          userMessage: "",
          assistantMessage: "",
          sessionSummary: "",
          conversationId: currentSessionId,
          sessionTurnCount: 0,
          runId,
          runPhase: phase,
          activeTaskFocus,
          lastReferencedFile: null,
          recentAttachmentAssetCount: 0,
          longTermMemoryStatus: activeTaskFocus ? "available" : "empty",
          longTermMemoryEntryCount: activeTaskFocus ? 1 : 0,
          traceStepCount: 0,
          toolActivityCount: 0,
          providerName: "",
          providerModel: ""
        }
      : null,
    resumeCount: runState?.resumeCount ?? 0,
    lastDecision: runState?.lastDecisionSummary?.trim()
      ? {
          kind: "continue",
          reason: "retrieved_run_state",
          summary: runState.lastDecisionSummary.trim(),
          targetPhase: phase
        }
      : null,
    controlBoundaryEvidence: [],
    createdAtMs: updatedAtMs,
    updatedAtMs
  };
}

export function extractActiveTaskFocus(entries?: LongTermMemoryEntry[] | null) {
  const content =
    entries?.find((entry) => entry.kind === "project_focus.active_task")?.content?.trim() ?? "";
  if (!content) {
    return "";
  }

  const taskId = content.match(/\b[A-Z]{2,6}-\d{1,6}\b/)?.[0] ?? "";
  return taskId || content;
}

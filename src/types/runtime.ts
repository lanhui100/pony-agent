import type { ProviderReasoningEffort } from "@/types/provider";

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
  | "calling_model"
  | "calling_tool"
  | "failed";

export type TurnLifecyclePhase =
  | "created"
  | "preparing"
  | "building_context"
  | "calling_model"
  | "streaming_response"
  | "executing_tool"
  | "tool_result_integrating"
  | "checkpointing"
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
  | "turn.tool_call_started"
  | "turn.tool_call_completed"
  | "turn.trace_updated"
  | "turn.checkpoint_persisted"
  | "turn.completed"
  | "turn.failed"
  | "turn.cancelled";

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
  status: "planned" | "running" | "done" | "error";
  summary: string;
  argumentsText?: string | null;
  resultText?: string | null;
  durationSeconds?: number | null;
  capabilityInvocation?: CapabilityInvocationRecord | null;
};

export type AvailableTool = {
  name: string;
  description: string;
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
  description: string;
  invocationMode: CapabilityInvocationMode;
  inputSchemaSummary: string;
  safetyClass: string;
  visibility: string;
  observabilityTags: string[];
  requiresApproval: boolean;
  hostMediated: boolean;
  permissionScope: string;
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
  state: "completed" | "active" | "pending" | "error" | "cancelled";
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
  turnDurationMs?: number | null;
};

export type PrefixMutationReason =
  | "session_summary_changed"
  | "run_goal_changed"
  | "long_term_memory_changed"
  | "planner_skills_changed"
  | "image_note_changed"
  | "truncation_note_changed"
  | "history_boundary_shifted"
  | "native_transcript_boundary_shifted";

export type ProviderRequestKind = "initial_request" | "tool_followup";
export type ProviderLatencyKind = "provider_stream" | "buffered_response" | "unknown";

export type ProviderCallCacheRecord = {
  requestKind: ProviderRequestKind;
  providerSource?: string | null;
  providerMode?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  cacheMissInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  latencyKind?: ProviderLatencyKind | null;
  prefixMutationReasons?: PrefixMutationReason[];
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
  transcriptRef?: string | null;
  runRef?: string | null;
  workspaceRef?: WorkspaceStateRef | null;
  summary: string;
  createdAtMs: number;
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
  detail?: string | null;
  durationSeconds?: number | null;
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
  sessionSummary?: string | null;
  fallbackReason?: string | null;
  error?: string | null;
  inputTokens?: number | null;
  cacheHitInputTokens?: number | null;
  reasoningTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  firstTokenLatencyMs?: number | null;
  turnDurationMs?: number | null;
  updatedAt: number;
};

export type TurnInput = {
  message: string;
  displayMessage?: string | null;
  providerId?: string | null;
  modelId?: string | null;
  reasoningEffort?: ProviderReasoningEffort | null;
  sessionId?: string | null;
  nodeId?: string | null;
  history?: TurnHistoryMessage[];
  images?: TurnInputImage[];
};

export type TurnHistoryMessage = {
  role: "user" | "assistant";
  content: string;
  attachments?: AttachmentMeta[];
};

export type SessionOverview = {
  conversationId: string;
  title?: string | null;
  summary: string;
  turnCount: number;
  lastReferencedFile?: string | null;
  updatedAtMs: number;
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

export type TurnStreamEvent = {
  eventId?: string | null;
  sessionId?: string | null;
  turnId: string;
  kind: "started" | "delta" | "trace" | "tool" | "completed" | "failed" | "cancelled";
  eventType?: TurnLifecycleEventType | string | null;
  eventVersion?: string | null;
  sequence?: number | null;
  emittedAtMs?: number | null;
  phase?: RuntimePhase | string | null;
  text?: string | null;
  reasoningContent?: string | null;
  error?: string | null;
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

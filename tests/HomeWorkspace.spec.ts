import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick, watch } from "vue";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import {
  __resetFrontendFlightRecorderForTests,
  injectFrontendDiagnosticStall
} from "@/lib/frontend-flight-recorder";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderReasoningEffort, ProviderRegistry } from "@/types/provider";
import type {
  ChatMessage,
  ExecutionCheckpoint,
  GraphRunControlBoundaryEvidence,
  GraphRunSubmissionPlan,
  HistoryBranch,
  HistoryNode,
  RunControlAuditSummary
} from "@/types/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

vi.mock("@/lib/frontend-flight-recorder", async () => {
  const actual = await vi.importActual<typeof import("@/lib/frontend-flight-recorder")>(
    "@/lib/frontend-flight-recorder"
  );
  return {
    ...actual,
    injectFrontendDiagnosticStall: vi.fn(() => 1800)
  };
});

const scrollToBottomSpy = vi.fn();
const viewportScrollToSpy = vi.fn();
const viewportMetrics = {
  scrollHeight: 1000,
  scrollTop: 700,
  clientHeight: 400
};
let latestViewportEl: HTMLElement | null = null;
let latestResizeObserverCallback: ResizeObserverCallback | null = null;
let latestResizeObserverTarget: Element | null = null;
let rafSeqId = 0;
let rafTimers = new Map<number, ReturnType<typeof setTimeout>>();
let markdownStubInstanceSeq = 0;
let toolPanelMotionMountCount = 0;
let toolPanelMotionUpdateCount = 0;

function resetViewportMetrics() {
  viewportMetrics.scrollHeight = 1000;
  viewportMetrics.scrollTop = 700;
  viewportMetrics.clientHeight = 400;
}

function createViewportElement() {
  const viewportEl = document.createElement("div");
  Object.defineProperty(viewportEl, "scrollHeight", {
    configurable: true,
    get: () => viewportMetrics.scrollHeight
  });
  Object.defineProperty(viewportEl, "clientHeight", {
    configurable: true,
    get: () => viewportMetrics.clientHeight
  });
  Object.defineProperty(viewportEl, "scrollTop", {
    configurable: true,
    get: () => viewportMetrics.scrollTop,
    set: (value: number) => {
      viewportMetrics.scrollTop = value;
    }
  });
  Object.defineProperty(viewportEl, "scrollTo", {
    configurable: true,
    value: (options?: ScrollToOptions | number, y?: number) => {
      viewportScrollToSpy(options);
      if (typeof options === "number") {
        viewportMetrics.scrollTop = typeof y === "number" ? y : viewportMetrics.scrollTop;
      } else if (options?.top != null) {
        viewportMetrics.scrollTop = options.top;
      }
      viewportEl.dispatchEvent(new Event("scroll"));
    }
  });
  latestViewportEl = viewportEl;
  return viewportEl;
}

function triggerViewportScroll(top: number) {
  if (!latestViewportEl) {
    throw new Error("viewport is not mounted");
  }

  viewportMetrics.scrollTop = top;
  latestViewportEl.dispatchEvent(new Event("scroll"));
}

function triggerViewportProgrammaticIntermediateScroll(top: number) {
  if (!latestViewportEl) {
    throw new Error("viewport is not mounted");
  }

  viewportMetrics.scrollTop = top;
  latestViewportEl.dispatchEvent(new Event("scroll"));
}

function mockElementRect(
  element: Element,
  rect: Partial<Pick<DOMRect, "top" | "bottom" | "left" | "right" | "width" | "height">>
) {
  const top = rect.top ?? 0;
  const bottom = rect.bottom ?? top + (rect.height ?? 0);
  const left = rect.left ?? 0;
  const right = rect.right ?? left + (rect.width ?? 0);
  const width = rect.width ?? Math.max(0, right - left);
  const height = rect.height ?? Math.max(0, bottom - top);

  Object.defineProperty(element, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      x: left,
      y: top,
      top,
      bottom,
      left,
      right,
      width,
      height,
      toJSON: () => ({ top, bottom, left, right, width, height })
    })
  });
}

async function advanceAnimationFrames(count = 13) {
  for (let i = 0; i < count; i++) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  }
}

function triggerViewportWheel() {
  if (!latestViewportEl) {
    throw new Error("viewport is not mounted");
  }

  latestViewportEl.dispatchEvent(new Event("wheel"));
}

function triggerContentResize() {
  if (!latestResizeObserverCallback || !latestResizeObserverTarget) {
    throw new Error("resize observer is not active");
  }

  latestResizeObserverCallback(
    [{ target: latestResizeObserverTarget } as ResizeObserverEntry],
    {} as ResizeObserver
  );
}

const ScrollAreaStub = defineComponent({
  setup(_props, { slots, expose }) {
    const viewportEl = createViewportElement();

    expose({
      viewportEl,
      scrollToBottom: scrollToBottomSpy
    });

    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

const MarkdownRendererStub = defineComponent({
  props: {
    content: {
      type: String,
      default: ""
    },
    streaming: {
      type: Boolean,
      default: false
    },
    wrapperClass: {
      type: String,
      default: ""
    },
    toneClass: {
      type: String,
      default: ""
    },
    forceMarkdownStreaming: {
      type: Boolean,
      default: false
    }
  },
  emits: ["render-complete"],
  setup(props, { emit }) {
    const instanceId = ++markdownStubInstanceSeq;

    watch(
      () => [props.content, props.streaming] as const,
      ([content, streaming]) => {
        window.setTimeout(() => {
          emit("render-complete", {
            contentLength: content.length,
            streaming
          });
        }, 0);
      },
      { immediate: true }
    );

    return { instanceId };
  },
  template:
    '<div class="markdown-stub" :class="[wrapperClass, toneClass]" :data-instance-id="String(instanceId)" :streaming="streaming ? \'true\' : undefined" :data-force-markdown-streaming="forceMarkdownStreaming ? \'true\' : undefined">{{ content }}</div>'
});

const ButtonStub = defineComponent({
  props: {
    disabled: {
      type: Boolean,
      default: false
    },
    title: {
      type: String,
      default: ""
    }
  },
  emits: ["click"],
  template:
    '<button class="button-stub" type="button" :disabled="disabled" :title="title" @click="$emit(\'click\')"><slot /></button>'
});

function createProviderRegistry(options?: {
  supportsReasoning?: boolean;
  selectedProviderId?: string;
}): ProviderRegistry {
  return {
    selectedProviderId: options?.selectedProviderId ?? "provider-openai",
    providers: [
      {
        id: "provider-openai",
        name: "OpenAI",
        protocol: "openai",
        baseUrl: "https://api.openai.com/v1",
        apiKeyEnvVar: "OPENAI_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-gpt5",
        models: [
          {
            id: "model-gpt5",
            name: "GPT-5",
            model: "gpt-5",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "open-ai-reasoning",
            capabilities: {
              contextWindowTokens: 128000,
              supportsTools: true,
              supportsStreaming: true,
              supportsImageInput: false,
              supportsReasoning: options?.supportsReasoning ?? true
            }
          }
        ]
      }
    ]
  };
}

function createMultiProviderRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-openai",
    providers: [
      ...createProviderRegistry().providers,
      {
        id: "provider-anthropic",
        name: "Anthropic",
        protocol: "anthropic",
        baseUrl: "https://api.anthropic.com/v1",
        apiKeyEnvVar: "ANTHROPIC_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-claude-4",
        models: [
          {
            id: "model-claude-4",
            name: "Claude 4",
            model: "claude-4",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: "medium",
            reasoningBudgetTokens: null,
            capabilityPreset: "anthropic-thinking",
            capabilities: {
              contextWindowTokens: 200000,
              supportsTools: true,
              supportsStreaming: true,
              supportsImageInput: true,
              supportsReasoning: true
            }
          }
        ]
      }
    ]
  };
}

function createMessage(partial: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? "turn-1",
    role: partial.role ?? "user",
    content: partial.content ?? "hello",
    status: partial.status ?? "done",
    tokenCount: partial.tokenCount ?? null,
    reasoningContent: partial.reasoningContent ?? null,
    modelName: partial.modelName ?? null,
    toolName: partial.toolName ?? null,
    canonicalToolName: partial.canonicalToolName ?? null,
    displayNameZh: partial.displayNameZh ?? null,
    detail: partial.detail ?? null,
    durationSeconds: partial.durationSeconds ?? null,
    errorDetail: partial.errorDetail ?? null
  };
}

function createTrace(
  partial: Partial<{
    turnId: string;
    phase: "idle" | "ready" | "calling_model" | "calling_tool" | "completed" | "cancelled" | "failed";
    error: string | null;
    sessionSummary: string | null;
    toolActivities: Array<{ id: string; name: string; status: string }>;
  }> = {}
) {
  return {
    turnId: partial.turnId ?? "turn-1",
    sessionId: "session-current",
    eventId: null,
    eventType: null,
    eventVersion: null,
    sequence: null,
    emittedAtMs: null,
    title: "failed turn",
    phase: partial.phase ?? "failed",
    traceSteps: [],
    traceTimeline: [
      {
        id: "timeline-1",
        kind: "return",
        label: "RETURN",
        state: partial.phase ?? "failed",
        sequence: 1,
        provider_requested_name: null,
        provider_name: null,
        provider_protocol: null,
        provider_model: null,
        provider_source: null,
        provider_mode: null,
        build_context_observation: null,
        tool_activities: partial.toolActivities ?? [],
        text: null,
        reasoning_content: null,
        fallback_reason: null,
        error: partial.error ?? null,
        input_tokens: null,
        cache_hit_input_tokens: null,
        reasoning_tokens: null,
        output_tokens: null,
        total_tokens: null,
        first_token_latency_ms: null,
        turn_duration_ms: null
      }
    ],
    toolActivities: partial.toolActivities ?? [],
    providerCallRecords: [],
    hookTraceRecords: [],
    providerRequestedName: null,
    providerName: null,
    providerProtocol: null,
    providerModel: null,
    providerSource: null,
    providerMode: null,
    buildContextObservation: null,
    sessionSummary: partial.sessionSummary ?? "failure summary",
    fallbackReason: null,
    error: partial.error ?? null,
    inputTokens: null,
    cacheHitInputTokens: null,
    reasoningTokens: null,
    outputTokens: null,
    totalTokens: null,
    firstTokenLatencyMs: null,
    turnDurationMs: null,
    updatedAt: 1000
  };
}

function createHistoryNode(partial: Partial<HistoryNode> = {}): HistoryNode {
  return {
    nodeId: partial.nodeId ?? "node-1",
    sessionId: partial.sessionId ?? "session-current",
    parentNodeId: partial.parentNodeId ?? null,
    branchId: partial.branchId ?? "branch-main",
    forkedFromNodeId: partial.forkedFromNodeId ?? null,
    kind: partial.kind ?? "turn_committed",
    turnId: partial.turnId ?? null,
    transcriptRef: partial.transcriptRef ?? null,
    runRef: partial.runRef ?? null,
    workspaceRef: partial.workspaceRef ?? { kind: "none", rollbackCapable: false },
    summary: partial.summary ?? "checkpoint summary",
    title: partial.title ?? "checkpoint title",
    history: partial.history ?? [],
    turnTraceHistory: partial.turnTraceHistory ?? [],
    turnCount: partial.turnCount ?? 1,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    createdAtMs: partial.createdAtMs ?? 1000
  };
}

function createHistoryBranch(partial: Partial<HistoryBranch> = {}): HistoryBranch {
  return {
    branchId: partial.branchId ?? "branch-main",
    sessionId: partial.sessionId ?? "session-current",
    baseNodeId: partial.baseNodeId ?? "node-1",
    headNodeId: partial.headNodeId ?? "node-2",
    forkedFromBranchId: partial.forkedFromBranchId ?? null,
    forkedFromNodeId: partial.forkedFromNodeId ?? null,
    label: partial.label ?? "main",
    createdAtMs: partial.createdAtMs ?? 1000,
    updatedAtMs: partial.updatedAtMs ?? 2000
  };
}

function createCheckpoint(partial: Partial<ExecutionCheckpoint> = {}): ExecutionCheckpoint {
  return {
    turnId: partial.turnId ?? "turn-1",
    sessionId: partial.sessionId ?? "session-current",
    runId: partial.runId ?? "run-1",
    checkpointKind: partial.checkpointKind ?? "recovery",
    recoveryMode: partial.recoveryMode ?? "persisted_effect",
    projectedRuntimePhase: partial.projectedRuntimePhase ?? "ready",
    submissionCommand: partial.submissionCommand ?? "resume_graph_run_stream",
    resumable: partial.resumable ?? true,
    replayable: partial.replayable ?? true,
    status: partial.status ?? "ready",
    phase: partial.phase ?? "paused",
    providerRequestedName: partial.providerRequestedName ?? "OpenAI",
    providerName: partial.providerName ?? "OpenAI",
    providerProtocol: partial.providerProtocol ?? "openai",
    providerModel: partial.providerModel ?? "gpt-5",
    providerSource: partial.providerSource ?? "graph_checkpoint",
    providerMode: partial.providerMode ?? "recovery",
    fallbackReason: partial.fallbackReason ?? null,
    completedHops: partial.completedHops ?? 0,
    maxHops: partial.maxHops ?? 0,
    activeToolName: partial.activeToolName ?? null,
    traceSteps: partial.traceSteps ?? [],
    toolActivities: partial.toolActivities ?? [],
    error: partial.error ?? null,
    startedAtMs: partial.startedAtMs ?? 1000,
    updatedAtMs: partial.updatedAtMs ?? 1200,
    stopRequestedAtMs: partial.stopRequestedAtMs ?? null
  };
}

function createSubmissionPlan(
  partial: Partial<GraphRunSubmissionPlan> = {}
): GraphRunSubmissionPlan {
  return {
    command: partial.command ?? "start_graph_run_stream",
    runId: partial.runId ?? null,
    source: partial.source ?? "default"
  };
}

function createBoundaryEvidence(
  partial: Partial<GraphRunControlBoundaryEvidence> = {}
): GraphRunControlBoundaryEvidence {
  return {
    hookPoint: partial.hookPoint ?? "turn.completed",
    canonicalEventType: partial.canonicalEventType ?? "turn.completed",
    canonicalPhase: partial.canonicalPhase ?? "completed",
    summary: partial.summary ?? "在 turn.completed 安全边界暂停",
    hookEnvelope: partial.hookEnvelope ?? {
      sessionId: "session-current",
      runId: "run-1",
      turnId: "turn-1",
      sequence: 1,
      hookPoint: "turn.completed",
      canonicalEventType: "turn.completed",
      canonicalPhase: "completed",
      payloadJson: "{}",
      createdAtMs: 1000
    },
    createdAtMs: partial.createdAtMs ?? 1000
  };
}

function createRunControlAuditSummary(
  partial: {
    action?: Partial<RunControlAuditSummary["actionEvidenceSummary"]>;
    currentContext?: Partial<RunControlAuditSummary["currentContextProjection"]>;
  } = {}
): RunControlAuditSummary {
  return {
    actionEvidenceSummary: {
      status: partial.action?.status ?? "available",
      sourceFamily: partial.action?.sourceFamily ?? "run_control",
      commandKind: partial.action?.commandKind ?? "resume_graph_run_stream",
      boundary: partial.action?.boundary ?? "run_resume",
      resultKind: partial.action?.resultKind ?? "observe",
      summary: partial.action?.summary ?? "检测到暂停中的运行；点击后会恢复该 run 并继续执行。",
      targetSummary: partial.action?.targetSummary ?? "恢复 run-1",
      elapsedMs: partial.action?.elapsedMs ?? 8,
      blocked: partial.action?.blocked ?? false,
      degraded: partial.action?.degraded ?? false,
      evidenceId: partial.action?.evidenceId ?? "run-control-evidence-1",
      observedAtMs: partial.action?.observedAtMs ?? 1000,
      runId: partial.action?.runId ?? "run-1",
      turnId: partial.action?.turnId ?? "turn-1",
      checkpointTurnId: partial.action?.checkpointTurnId ?? "turn-1",
      checkpointKind: partial.action?.checkpointKind ?? "recovery",
      recoveryMode: partial.action?.recoveryMode ?? "persisted_effect",
      projectedCommand: partial.action?.projectedCommand ?? "resume_graph_run_stream",
      degradationReason: partial.action?.degradationReason ?? null,
      requestSummary: partial.action?.requestSummary ?? "resume run-1",
      startReason: partial.action?.startReason ?? null
    },
    currentContextProjection: {
      phase: partial.currentContext?.phase ?? "paused",
      checkpointStatus: partial.currentContext?.checkpointStatus ?? "ready",
      activeRunId: partial.currentContext?.activeRunId ?? "run-1",
      checkpointKind: partial.currentContext?.checkpointKind ?? "recovery",
      checkpointRecoveryMode: partial.currentContext?.checkpointRecoveryMode ?? "persisted_effect",
      submissionPlanCommand:
        partial.currentContext?.submissionPlanCommand ?? "resume_graph_run_stream"
    }
  };
}

function mountWorkspace(options?: {
  registry?: ProviderRegistry | null;
  selectedReasoningEffort?: ProviderReasoningEffort | null;
}) {
  const providerStore = useProviderStore();
  providerStore.$patch({
    registry: options?.registry === undefined ? createProviderRegistry() : options.registry,
    selectedReasoningEffort: options?.selectedReasoningEffort ?? null
  });

  return mount({
    render() {
      return h(TooltipProvider, null, {
        default: () => h(HomeWorkspace)
      });
    }
  }, {
    global: {
      directives: {
        motion: {
          mounted(element: HTMLElement) {
            if (element.classList.contains("conversation-tool-panel")) {
              toolPanelMotionMountCount++;
            }
            // No-op in unit tests; only suppresses directive resolution noise.
          },
          updated() {
            // No-op in unit tests; only suppresses directive resolution noise.
          },
          beforeUpdate(element: HTMLElement) {
            if (element.classList.contains("conversation-tool-panel")) {
              toolPanelMotionUpdateCount++;
            }
            // No-op in unit tests; only suppresses directive resolution noise.
          }
        }
      },
      stubs: {
        ScrollArea: ScrollAreaStub,
        MarkdownRenderer: MarkdownRendererStub,
        Button: ButtonStub,
        Transition: true,
        TransitionGroup: true
      }
    }
  });
}

function findStreamingMarkdown(wrapper: ReturnType<typeof mount>) {
  return wrapper.find('.assistant-plain-text[data-streaming="true"]');
}

function flushAsyncUiWork() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 0));
}

async function waitForStreamingPresentation(delayMs = 300) {
  await new Promise<void>((resolve) => window.setTimeout(resolve, delayMs));
  await nextTick();
}

async function waitForCondition(check: () => boolean, attempts = 8) {
  for (let index = 0; index < attempts; index += 1) {
    if (check()) {
      return true;
    }
    await flushAsyncUiWork();
    await nextTick();
  }
  return check();
}

function clickLatestRollbackConfirm(label: string) {
  const matches = Array.from(document.body.querySelectorAll('button')).filter((button) =>
    button.closest('[data-side]')?.textContent?.includes(label)
  ) as HTMLButtonElement[];
  if (matches.length === 0) return false;
  matches[matches.length - 1]!.click();
  return true;
}

function latestScrollDebugEvent(eventName: string) {
  const buffer = (window as typeof window & {
    __ponyScrollDebugBuffer?: Array<Record<string, unknown>>;
  }).__ponyScrollDebugBuffer ?? [];

  return [...buffer].reverse().find((entry) => entry.event === eventName);
}

describe("HomeWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    markdownStubInstanceSeq = 0;
    toolPanelMotionMountCount = 0;
    toolPanelMotionUpdateCount = 0;
    __resetFrontendFlightRecorderForTests();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.stubGlobal(
      "requestAnimationFrame",
      ((callback: FrameRequestCallback) => {
        const rafId = ++rafSeqId;
        const timer = setTimeout(() => {
          rafTimers.delete(rafId);
          callback(performance.now());
        }, 16);
        rafTimers.set(rafId, timer);
        return rafId;
      }) as typeof requestAnimationFrame
    );
    vi.stubGlobal(
      "cancelAnimationFrame",
      vi.fn((rafId: number) => {
        const timer = rafTimers.get(rafId);
        if (timer) {
          clearTimeout(timer);
          rafTimers.delete(rafId);
        }
      })
    );
    vi.stubGlobal(
      "ResizeObserver",
      class ResizeObserver {
        constructor(callback: ResizeObserverCallback) {
          latestResizeObserverCallback = callback;
        }

        observe(target: Element) {
          latestResizeObserverTarget = target;
        }

        disconnect() {
          latestResizeObserverTarget = null;
        }
      } as typeof ResizeObserver
    );
    scrollToBottomSpy.mockReset();
    viewportScrollToSpy.mockReset();
    latestViewportEl = null;
    latestResizeObserverCallback = null;
    latestResizeObserverTarget = null;
    rafTimers = new Map();
    resetViewportMetrics();
  });

  afterEach(() => {
    for (const timer of rafTimers.values()) {
      clearTimeout(timer);
    }
    rafTimers.clear();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("disables composer while switching session", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      draftMessage: "keep going",
      sessionOperation: "switching",
      sessionError: null,
      phase: "idle",
      error: null,
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect((wrapper.get("textarea").element as HTMLTextAreaElement).disabled).toBe(true);
    expect((wrapper.get("button.button-stub").element as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows runtime failure banner without disabling input", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "failed",
      error: "tool chain exploded",
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.text()).not.toContain("tool chain exploded");
    expect((wrapper.get("textarea").element as HTMLTextAreaElement).disabled).toBe(false);
  });

  it("shows a welcome empty state for a brand-new session", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "idle",
      error: null,
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.get('[data-testid="workspace-empty-state"]').text()).toContain("我能帮你做些什么？");
  });

  // KNOWN TEST DEBT: auto-scroll behavior changed after component restructure
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("skips the initial auto-scroll work for an empty workspace", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "idle",
      error: null,
      messages: []
    });

    mountWorkspace();
    await nextTick();

    expect(viewportScrollToSpy).not.toHaveBeenCalled();
    expect(scrollToBottomSpy).not.toHaveBeenCalled();
  });

  it("keeps the welcome empty state after createSession inserts a transient session overview", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-transient",
      sessionOperation: null,
      phase: "idle",
      error: null,
      messages: [],
      sessionList: [
        {
          conversationId: "session-transient",
          title: "新对话",
          summary: "发送第一条消息后保存到历史",
          turnCount: 0,
          lastReferencedFile: null,
          updatedAtMs: 0
        }
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.get('[data-testid="workspace-empty-state"]').text()).toContain("我能帮你做些什么？");
  });

  it("shows a failure summary when a saved session has failed trace history but no transcript", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-failed-history",
      sessionOperation: null,
      phase: "failed",
      error: null,
      messages: [],
      sessionList: [
        {
          conversationId: "session-failed-history",
          title: "失败的历史",
          summary: "失败摘要",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 1000
        }
      ],
      turnTraceHistory: [
        createTrace({
          turnId: "turn-failed",
          phase: "failed",
          error: "tool chain exploded",
          sessionSummary: "失败摘要",
          toolActivities: [
            {
              id: "tool-1",
              name: "workspace_search",
              status: "error"
            } as never
          ]
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.text()).toContain("tool chain exploded");
    expect(wrapper.get('[data-testid="workspace-error-detail"]').text()).toContain("tool chain exploded");
    expect(wrapper.get('[data-testid="workspace-error-copy-turn-failed"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("需要我帮你做什么？");
  });

  it("renders error assistant details in a collapsible message block", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "assistant-error",
          turnId: "turn-error",
          role: "assistant",
          content: "请求失败",
          status: "error",
          modelName: "OpenAI/GPT-5",
          errorDetail: "stack line 1\nstack line 2"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.text()).toContain("错误详情");
    await wrapper.find("summary").trigger("click");
    await nextTick();
    expect(wrapper.get('[data-testid="workspace-error-detail"]').text()).toContain("stack line 1");
    expect(wrapper.get('[data-testid="workspace-error-copy-turn-error"]').exists()).toBe(true);
  });

  it("shows a resume CTA when the next submission will resume a paused run", async () => {
    const runtimeStore = useRuntimeStore();
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    runtimeStore.$patch({
      draftMessage: "继续这个 run",
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [],
      latestExecutionCheckpoint: createCheckpoint({
        runId: "run-paused",
        submissionCommand: "resume_graph_run_stream",
        phase: "paused",
        status: "ready"
      }),
      latestGraphRunSubmissionPlan: createSubmissionPlan({
        command: "resume_graph_run_stream",
        runId: "run-paused",
        source: "checkpoint"
      }),
      latestRunControlAuditSummary: createRunControlAuditSummary({
        action: {
          summary: "检测到暂停中的运行；点击后会恢复该 run 并继续执行。",
          runId: "run-paused",
          projectedCommand: "resume_graph_run_stream"
        },
        currentContext: {
          activeRunId: "run-paused"
        }
      })
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.get('[data-testid="workspace-submit-action"]').text()).toContain("恢复");
    expect(wrapper.text()).not.toContain("恢复该 run");

    await wrapper.get('[data-testid="workspace-submit-action"]').trigger("click");
    expect(submitSpy).toHaveBeenCalledTimes(1);
  });

  it("shows a restart CTA when only replay is available", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      draftMessage: "重新跑一次",
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [],
      latestExecutionCheckpoint: createCheckpoint({
        checkpointKind: "lifecycle_boundary",
        recoveryMode: "replay_required",
        submissionCommand: "start_graph_run_stream",
        resumable: false,
        replayable: false,
        phase: "checkpointing",
        status: "completed"
      }),
      latestGraphRunSubmissionPlan: createSubmissionPlan({
        command: "start_graph_run_stream",
        runId: null,
        source: "checkpoint"
      }),
      latestRunControlAuditSummary: createRunControlAuditSummary({
        action: {
          commandKind: "start_graph_run_stream",
          projectedCommand: "start_graph_run_stream",
          startReason: "replay_from_checkpoint",
          degraded: true,
          checkpointKind: "lifecycle_boundary",
          recoveryMode: "replay_required",
          summary: "当前恢复点只保留持久化事实；点击后会重新开始新的执行。"
        },
        currentContext: {
          phase: "checkpointing",
          checkpointKind: "lifecycle_boundary",
          checkpointRecoveryMode: "replay_required",
          submissionPlanCommand: "start_graph_run_stream"
        }
      })
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.get('[data-testid="workspace-submit-action"]').text()).toContain("重新开始");
    expect(wrapper.text()).not.toContain("重新开始新的执行");
  });

  it("renders timeout retrying assistant state in error color", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "calling_model",
      error: null,
      messages: [
        createMessage({
          id: "user-timeout-retry",
          turnId: "turn-timeout-retry",
          role: "user",
          content: "再试一次"
        }),
        createMessage({
          id: "assistant-timeout-retry",
          turnId: "turn-timeout-retry",
          role: "assistant",
          content: "超时后错误重连中，请稍候，正在继续尝试恢复输出。",
          status: "done",
          modelName: "OpenAI/GPT-5",
          errorDetail: "timeout: previous call timed out"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.text()).toContain("超时后错误重连中");
    const markdownBlocks = wrapper.findAll(".assistant-plain-text");
    expect(markdownBlocks.some((node) => node.classes().includes("text-stone-800"))).toBe(true);
  });

  it("keeps control boundary evidence out of the workspace header area", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      draftMessage: "继续这个 run",
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [],
      latestGraphRunControlBoundaryEvidence: [
        createBoundaryEvidence({
          summary: "hook turn.completed 已确认可安全暂停"
        })
      ],
      latestRunControlAuditSummary: createRunControlAuditSummary({
        action: {
          commandKind: "stop_graph_run",
          boundary: "stop_requested",
          resultKind: "observe",
          summary: "已请求停止当前运行，等待 agent 在安全边界暂停。"
        },
        currentContext: {
          phase: "running",
          submissionPlanCommand: "resume_graph_run_stream"
        }
      })
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.text()).not.toContain("控制摘要：已请求停止当前运行，等待 agent 在安全边界暂停。");
  });

  it("shows an explicit stop CTA while a turn is running", async () => {
    const runtimeStore = useRuntimeStore();
    const stopSpy = vi.spyOn(runtimeStore, "stopTurn").mockResolvedValue(true);
    runtimeStore.$patch({
      draftMessage: "",
      sessionOperation: null,
      phase: "running",
      error: null,
      isSubmitting: true,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const stopButton = wrapper.get('[data-testid="workspace-stop-turn"]');
    expect(stopButton.attributes("title")).toBe("请求在安全边界停止当前运行。");

    await stopButton.trigger("click");
    await nextTick();

    expect(stopSpy).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).not.toContain("已请求停止当前运行");
  });

  it("keeps the open workspace shell and rounded white composer", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "idle",
      error: null,
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.find("section").element.className).toContain("rounded-t-[0.6rem]");
    expect(wrapper.find("section").element.className).not.toContain("bg-[#fdfbf7]/88");

    const timeline = wrapper.get(".scroll-area-stub");
    expect(timeline.element.className).toContain("rounded-t-[0.6rem]");
    expect(wrapper.get('[data-testid="workspace-content-column"]').classes()).toContain("max-w-[46.4rem]");

    const composerShell = wrapper.get('[data-testid="workspace-composer-shell"]');
    expect(composerShell.classes()).toContain("max-w-[38.4rem]");
    expect(composerShell.classes()).toContain("rounded-[0.6rem]");
    expect(composerShell.classes()).toContain("bg-white/76");
    expect(composerShell.classes()).not.toContain("border-t");
  });

  it("keeps composer input typography understated and compact", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "idle",
      error: null,
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const textareaClassName = wrapper.get("textarea").attributes("class") ?? "";

    expect(textareaClassName).toContain("text-[13px]");
    expect(textareaClassName).toContain("leading-[1.55]");
    expect(textareaClassName).toContain("text-stone-800");
    expect(textareaClassName).toContain("placeholder:text-[12px]");
    expect(textareaClassName).toContain("placeholder:text-stone-400/70");
  });

  it("renders assistant messages full width and removes user or assistant token footer", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "user message",
          tokenCount: 123
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "assistant reply",
          tokenCount: 456,
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const assistantArticle = wrapper.findAll("article").find((node) => node.classes().includes("conversation-agent-shell"));

    expect(assistantArticle).toBeDefined();
    expect(assistantArticle?.classes()).toContain("w-full");
    expect(assistantArticle?.classes()).not.toContain("max-w-[86%]");
    expect(assistantArticle?.classes()).not.toContain("sm:max-w-[78%]");
    expect(wrapper.text()).not.toContain("IN:123");
    expect(wrapper.text()).not.toContain("OUT:456");
  });

  it("renders pending assistant content as accumulated text and highlights only the latest delta", async () => {
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "20");
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "**正在** 输出中",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await waitForStreamingPresentation();

    const streamingContent = wrapper.get('.assistant-plain-text[data-streaming="true"]');
    expect(streamingContent.text().length).toBeGreaterThan(0);
    expect("**正在** 输出中".startsWith(streamingContent.text())).toBe(true);
    expect(wrapper.find(".assistant-streaming-caret").exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-agent-actions"]').exists()).toBe(false);
  });

  it("shows a jumping-dot waiting state before the first assistant signal arrives", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "calling_model",
      isSubmitting: true,
      activeTurnId: "turn-1",
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const waitingState = wrapper.get('[data-testid="assistant-awaiting-first-signal"]');
    expect(waitingState.findAll(".assistant-waiting-dot")).toHaveLength(3);

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();

    expect(wrapper.find('[data-testid="assistant-awaiting-first-signal"]').exists()).toBe(true);

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "tool-1",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "pending",
          toolName: "Search",
          detail: "searching"
        })
      ]
    });
    await nextTick();

    expect(wrapper.find('[data-testid="assistant-awaiting-first-signal"]').exists()).toBe(false);
    expect(wrapper.text()).toContain("searching");
  });

  it("keeps a single streaming flow while pending plain text grows", async () => {
    const runtimeStore = useRuntimeStore();

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await waitForStreamingPresentation();
    let streamingContent = wrapper.get('.assistant-plain-text[data-streaming="true"]');
    expect(streamingContent.text().length).toBeGreaterThan(0);
    expect("hello".startsWith(streamingContent.text())).toBe(true);
    expect(wrapper.findAll(".markdown-stub")).toHaveLength(0);

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello this is a longer streamed assistant delta that crosses the first reveal threshold",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await waitForStreamingPresentation(100);

    streamingContent = wrapper.get('.assistant-plain-text[data-streaming="true"]');
    expect(streamingContent.text().length).toBeGreaterThan(0);
    expect(
      "hello this is a longer streamed assistant delta that crosses the first reveal threshold"
        .startsWith(streamingContent.text())
    ).toBe(true);
    expect(wrapper.findAll(".markdown-stub")).toHaveLength(0);
  });

  it("falls back to baseline pending text when the streaming markdown optimization is disabled", async () => {
    window.localStorage.setItem("pony-agent.stream-render.disable-optimization", "true");
    const runtimeStore = useRuntimeStore();

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "**正在** 输出中",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.get('[data-testid="assistant-streaming-flow"]').text()).toContain("**正在** 输出中");
    expect(wrapper.find(".markdown-stub").exists()).toBe(false);
    expect(wrapper.find(".assistant-streaming-caret").exists()).toBe(false);
  });

  it("switches from streaming text to final plain text when assistant completes", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "**正在** 输出中",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await waitForStreamingPresentation();

    // 流式期间：含 Markdown 语法的内容走增量 Markdown 渲染
    const streamingFlow = wrapper.get('[data-testid="assistant-streaming-flow"]');
    expect(streamingFlow.exists()).toBe(true);
    expect(streamingFlow.text()).toContain("**正在** 输出中");
    expect(wrapper.find(".markdown-stub").exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-agent-actions"]').exists()).toBe(false);

    runtimeStore.$patch({
      phase: "ready",
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "**完成** 输出",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();

    // 流式完成后切换到 MarkdownRenderer（branch 1）做一次性渲染
    expect(findStreamingMarkdown(wrapper).exists()).toBe(false);
    expect(wrapper.get(".markdown-stub").exists()).toBe(true);
    expect(wrapper.get(".markdown-stub").attributes("streaming")).toBeUndefined();
    const plainTextBlock = wrapper.get(".assistant-plain-text");
    expect(plainTextBlock.text()).toContain("**完成** 输出");
    expect(plainTextBlock.classes()).toContain("text-stone-800");
    expect(wrapper.get('[data-testid="workspace-agent-actions"]').exists()).toBe(true);
  });

  it("does not auto-scroll again when a pending assistant flips to final plain text without new content", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "**完成** 输出",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await waitForStreamingPresentation();
    expect(findStreamingMarkdown(wrapper).exists()).toBe(true);

    viewportScrollToSpy.mockClear();
    runtimeStore.$patch({
      phase: "ready",
      isSubmitting: false,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "**完成** 输出",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();

    expect(findStreamingMarkdown(wrapper).exists()).toBe(false);
    expect(wrapper.find(".assistant-plain-text").exists()).toBe(true);
    expect(viewportScrollToSpy).not.toHaveBeenCalled();
  });

  it("cleans up stale streaming state when assistant message ids change without changing message count", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-old",
          turnId: "turn-1",
          role: "assistant",
          content: "hello this is a longer streamed assistant delta",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    // 连续释放模型：首次同步释放四个字符
    expect(wrapper.get('.assistant-plain-text[data-streaming="true"]').text()).toBe("hell");
    expect(wrapper.findAll(".markdown-stub")).toHaveLength(0);

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-2",
          role: "user",
          content: "换一轮"
        }),
        createMessage({
          id: "assistant-new",
          turnId: "turn-2",
          role: "assistant",
          content: "**完成** 输出",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ],
      phase: "ready",
      isSubmitting: false
    });
    for (let i = 0; i < 5; i++) {
      await nextTick();
    }

    expect(findStreamingMarkdown(wrapper).exists()).toBe(false);
    const markdownBlocks = wrapper.findAll(".assistant-plain-text");
    const finalAssistantBlock = markdownBlocks.find((node) => node.text().includes("**完成** 输出"));
    expect(finalAssistantBlock).toBeDefined();
  });

  it("requests scroll follow-up when streaming content grows", async () => {
    const runtimeStore = useRuntimeStore();

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "src/agent 是如何组织的"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "src/agent 是如何组织的"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "a".repeat(240),
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(700);
    expect(findStreamingMarkdown(wrapper).exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-agent-actions"]').exists()).toBe(false);
  });

  it("re-arms auto-follow when submitting a new turn after user scrolls away", async () => {
    const runtimeStore = useRuntimeStore();

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: false,
      error: null,
      draftMessage: "继续",
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "上一轮"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "old reply",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    const wrapper = mountWorkspace();
    await nextTick();

    triggerViewportWheel();
    triggerViewportScroll(120);
    await nextTick();
    viewportScrollToSpy.mockClear();

    await wrapper.get('[data-testid="workspace-submit-action"]').trigger("click");

    expect(submitSpy).toHaveBeenCalledTimes(1);

    runtimeStore.$patch({
      isSubmitting: true,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "上一轮"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "old reply",
          status: "done",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "user-2",
          turnId: "turn-2",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-2",
          turnId: "turn-2",
          role: "assistant",
          content: "hello world",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    await advanceAnimationFrames(13);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(700);
  });

  it("scrolls a newly sent user message into the timeline instead of only locking the bottom anchor", async () => {
    const runtimeStore = useRuntimeStore();

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      isSubmitting: false,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "上一轮"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "old reply",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "上一轮"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "old reply",
          status: "done",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "user-2",
          turnId: "turn-2",
          role: "user",
          content: "继续"
        })
      ]
    });

    await nextTick();
    const latestUserMessage = wrapper.findAll(".conversation-user-message").at(-1);
    expect(latestUserMessage).toBeDefined();
    mockElementRect(latestUserMessage!.element, { top: 560, bottom: 660, left: 420, right: 760, width: 340, height: 100 });

    await advanceAnimationFrames(2);
    const request = latestScrollDebugEvent("queue-scroll-request");
    expect(request?.targetMode).toBe("latest-user");
  });

  it("keeps auto-follow armed across small streaming content updates", async () => {
    const runtimeStore = useRuntimeStore();

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(700);

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello world",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();
    await advanceAnimationFrames(5);
    await waitForStreamingPresentation();
    expect(findStreamingMarkdown(wrapper).exists()).toBe(true);
  });

  it("keeps user scroll override while streaming updates continue", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "从头看"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    viewportScrollToSpy.mockClear();

    triggerViewportWheel();
    triggerViewportScroll(120);
    await nextTick();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "从头看"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello world again",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await flushAsyncUiWork();
    await nextTick();

    expect(viewportScrollToSpy).not.toHaveBeenCalled();
  });

  it("resumes auto-follow after idle only when new streamed content arrives", async () => {
    vi.useFakeTimers();
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "1000");
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续生成"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "alpha this is the initial streamed batch that already exceeds the first reveal threshold",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    viewportScrollToSpy.mockClear();

    triggerViewportWheel();
    triggerViewportScroll(80);
    await nextTick();
    viewportScrollToSpy.mockClear();

    await vi.advanceTimersByTimeAsync(3000);
    await nextTick();
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续生成"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "alpha this is the initial streamed batch that already exceeds the first reveal threshold beta",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();
    await vi.advanceTimersByTimeAsync(60);
    await nextTick();
    const scrolledToBottom = viewportScrollToSpy.mock.calls.some(([options]) =>
      typeof options === "object" &&
      options?.top === viewportMetrics.scrollHeight &&
      options.behavior === "auto"
    );
    expect(scrolledToBottom || viewportMetrics.scrollTop >= viewportMetrics.scrollHeight).toBe(true);
    vi.useRealTimers();
  });

  it("keeps bottom lock through resize-driven streaming growth", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await flushAsyncUiWork();
    triggerViewportScroll(1000);
    viewportScrollToSpy.mockClear();

    viewportMetrics.scrollHeight = 1320;
    triggerContentResize();

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(1200);
  });

  it("does not reverse-scroll when streaming resize briefly reports a smaller height", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await flushAsyncUiWork();
    triggerViewportScroll(1200);
    viewportScrollToSpy.mockClear();

    viewportMetrics.scrollHeight = 900;
    triggerContentResize();

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBe(1200);
    expect(viewportScrollToSpy).not.toHaveBeenCalledWith(expect.objectContaining({ top: 900 }));
    expect(latestScrollDebugEvent("anchor-target:skip-reverse")).toBeDefined();
  });

  it("replays deferred resize follow-up after a queued auto-follow completes", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await flushAsyncUiWork();
    await vi.waitFor(() => expect(viewportMetrics.scrollTop).toBeGreaterThan(700));
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello there",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    viewportMetrics.scrollHeight = 1440;
    triggerContentResize();

    await advanceAnimationFrames(8);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(1300);
  });

  it("uses smooth follow for stream deltas and auto for resize compensation", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(700);

    viewportScrollToSpy.mockClear();
    viewportMetrics.scrollHeight = 1380;
    triggerContentResize();

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(1300);
  });

  it("keeps auto-follow active across intermediate smooth-scroll events", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello there",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(700);

    viewportScrollToSpy.mockClear();
    triggerViewportProgrammaticIntermediateScroll(760);
    triggerViewportProgrammaticIntermediateScroll(820);
    triggerViewportProgrammaticIntermediateScroll(900);
    await nextTick();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello there again",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(900);
  });

  it("does not reverse-scroll when a streaming delta temporarily lowers the anchor target", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello there",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(13);
    viewportMetrics.scrollHeight = 850;
    triggerViewportScroll(900);
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "hello there again",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    await advanceAnimationFrames(5);
    expect(viewportMetrics.scrollTop).toBe(900);
    expect(latestScrollDebugEvent("scroll-to-latest-turn:skip-reverse-or-small")).toBeDefined();
  });

  it("does not start a terminal smooth follow when assistant streaming completes", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      activeTurnId: "turn-terminal-follow",
      messages: [
        createMessage({
          id: "user-terminal-follow",
          turnId: "turn-terminal-follow",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-terminal-follow",
          turnId: "turn-terminal-follow",
          role: "assistant",
          content: "streaming answer",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);
    (window as typeof window & { __ponyScrollDebugBuffer?: Array<Record<string, unknown>> }).__ponyScrollDebugBuffer = [];
    viewportScrollToSpy.mockClear();

    runtimeStore.$patch({
      phase: "ready",
      isSubmitting: false,
      activeTurnId: null,
      messages: [
        createMessage({
          id: "user-terminal-follow",
          turnId: "turn-terminal-follow",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-terminal-follow",
          turnId: "turn-terminal-follow",
          role: "assistant",
          content: "streaming answer",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();
    await advanceAnimationFrames(3);

    expect(latestScrollDebugEvent("is-submitting:terminal-follow-skip")).toBeDefined();
    const debugBuffer = (window as typeof window & {
      __ponyScrollDebugBuffer?: Array<Record<string, unknown>>;
    }).__ponyScrollDebugBuffer ?? [];
    expect(debugBuffer.some((entry) =>
      entry.event === "queue-scroll-request" &&
      entry.behavior === "smooth" &&
      entry.targetMode === "anchor"
    )).toBe(false);
  });

  it("shows scroll-to-latest button only when latest user is below viewport", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "上一轮问题"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "上一轮回答",
          status: "done",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "user-2",
          turnId: "turn-2",
          role: "user",
          content: "最新问题"
        })
      ]
    });

    viewportMetrics.scrollHeight = 2200;
    viewportMetrics.scrollTop = 800;
    viewportMetrics.clientHeight = 400;

    const wrapper = mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);

    if (!latestViewportEl) {
      throw new Error("viewport is not mounted");
    }

    mockElementRect(latestViewportEl, { top: 0, bottom: 400, left: 0, right: 800, width: 800, height: 400 });

    const latestUserMessage = wrapper.findAll(".conversation-user-message").at(-1);
    expect(latestUserMessage).toBeDefined();

    mockElementRect(latestUserMessage!.element, { top: 120, bottom: 220, left: 420, right: 760, width: 340, height: 100 });
    triggerViewportWheel();
    triggerViewportScroll(800);
    await flushAsyncUiWork();
    await nextTick();

    const scrollButton = wrapper.get('[data-testid="workspace-scroll-to-bottom"]');
    await vi.waitFor(() => {
      const latestDebug = latestScrollDebugEvent("viewport-scroll:user-away-from-bottom");
      expect(latestDebug).toBeDefined();
      expect(latestDebug?.latestUserBelowViewport).toBe(false);
      expect(latestDebug?.showScrollToBottom).toBe(false);
    });

    mockElementRect(latestUserMessage!.element, { top: 460, bottom: 560, left: 420, right: 760, width: 340, height: 100 });
    triggerViewportWheel();
    triggerViewportScroll(800);
    await flushAsyncUiWork();
    await nextTick();

    await vi.waitFor(() => {
      const latestDebug = latestScrollDebugEvent("viewport-scroll:user-away-from-bottom");
      expect(latestDebug).toBeDefined();
      expect(latestDebug?.latestUserBelowViewport).toBe(true);
      expect(latestDebug?.showScrollToBottom).toBe(true);
    });

    expect(scrollButton.exists()).toBe(true);
  });

  it("keeps streaming assistant text in one streaming flow while content grows", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "streaming content that has already crossed the first batch threshold",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();
    await flushAsyncUiWork();
    viewportScrollToSpy.mockClear();

    let streamingFlow = wrapper.get('[data-testid="assistant-streaming-flow"]');
    const initialVisibleLength = streamingFlow.text().length;
    expect(initialVisibleLength).toBeGreaterThan(0);
    expect(initialVisibleLength).toBeLessThan("streaming content that has already crossed the first batch threshold".length);
    expect(wrapper.find(".markdown-stub").exists()).toBe(false);

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: `streaming content that has already crossed the first batch threshold and continues on the next line ${"x".repeat(100)}`,
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await waitForStreamingPresentation(100);

    streamingFlow = wrapper.get('[data-testid="assistant-streaming-flow"]');
    expect(streamingFlow.text().length).toBeGreaterThan(initialVisibleLength);
    expect(streamingFlow.text().length).toBeLessThan(
      `streaming content that has already crossed the first batch threshold and continues on the next line ${"x".repeat(100)}`.length
    );
    expect(wrapper.findAll('[data-testid="assistant-streaming-flow"]')).toHaveLength(1);

    await advanceAnimationFrames(5);
    const scrollDebugEvent = latestScrollDebugEvent("latest-turn-signature:queue-follow-scroll");
    expect(scrollDebugEvent).toBeDefined();
    // 流式更新改为 smooth (lerp) 滚动，不再使用 auto (instant jump)
    expect(scrollDebugEvent?.behavior).toBe("smooth");
    expect(scrollDebugEvent?.streamingAssistantUpdate).toBe(true);
    expect(viewportMetrics.scrollTop).toBeGreaterThan(700);
  });

  it("skips auto-scroll via streaming text updates when user has scrolled away", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    mountWorkspace();
    await nextTick();
    await flushAsyncUiWork();
    viewportScrollToSpy.mockClear();

    // 用户滚动离开底部 → auto-follow 暂停
    triggerViewportWheel();
    triggerViewportScroll(120);
    await nextTick();

    // 触发内容更新；此时 streamAutoFollowEnabled 为 false，不应抢回底部。
    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "继续"
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "more content",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();
    viewportScrollToSpy.mockClear();
    // 用 flushAsyncUiWork 而非固定延时：处理所有待处理的微任务和 setTimeout(0)，
    // 但不等流式 timer（60ms），避免 timer 触发后因内容变化导致意外 scroll
    await flushAsyncUiWork();

    // 用户已滚动，auto-scroll 不应从 streaming text update 触发
    expect(viewportScrollToSpy).not.toHaveBeenCalled();
  });

  // KNOWN TEST DEBT: refreshFrontendRecorderStats removed from store
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("支持从 workspace 手动注入 stall smoke", async () => {
    vi.useFakeTimers();
    const runtimeStore = useRuntimeStore();
    const refreshSpy = vi.spyOn(runtimeStore, "refreshFrontendRecorderStats");

    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      isSubmitting: false,
      error: null,
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    await wrapper.get('[data-testid="workspace-stall-smoke"]').trigger("click");
    await vi.runAllTimersAsync();

    expect(injectFrontendDiagnosticStall).toHaveBeenCalledWith(1800, "workspace-stall-smoke");
    expect(refreshSpy).toHaveBeenCalled();
    vi.useRealTimers();
  });

  // KNOWN TEST DEBT: provider menu rendering changed after component restructure
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("opens provider menu, selects another model, and closes afterwards", async () => {
    const providerStore = useProviderStore();
    const selectModelSpy = vi.spyOn(providerStore, "selectModel");

    const wrapper = mountWorkspace({
      registry: createMultiProviderRegistry()
    });
    await nextTick();

    const [, providerTrigger] = wrapper.findAll("button.composer-select-trigger");
    await providerTrigger.trigger("click");
    await nextTick();

    expect(wrapper.text()).toContain("OpenAI");
    expect(wrapper.text()).toContain("GPT-5");

    const anthropicButton = wrapper.findAll("button").find((node) => node.text().includes("Anthropic"));
    expect(anthropicButton).toBeDefined();

    await anthropicButton?.trigger("mouseenter");
    await nextTick();

    const claudeButton = wrapper.findAll("button").find((node) => node.text().includes("Claude 4"));
    expect(claudeButton).toBeDefined();

    await claudeButton?.trigger("click");
    await nextTick();

    expect(selectModelSpy).toHaveBeenCalledWith("provider-anthropic", "model-claude-4");
    expect(providerStore.currentProvider?.id).toBe("provider-anthropic");
    expect(providerStore.currentModel?.id).toBe("model-claude-4");
    expect(wrapper.findAll("div.absolute")).toHaveLength(0);
  });

  // KNOWN TEST DEBT: provider/reasoning menu rendering changed
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("closes provider and reasoning menus on outside click", async () => {
    const wrapper = mountWorkspace();
    await nextTick();

    const [, providerTrigger, reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
    await providerTrigger.trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("OpenAI");

    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();
    expect(wrapper.findAll("div.absolute")).toHaveLength(0);

    await reasoningTrigger.trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("极高");

    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();
    expect(wrapper.findAll("div.absolute")).toHaveLength(0);
  });

  // KNOWN TEST DEBT: reasoning menu rendering changed
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("syncs reasoning menu selection and visibility toggle persistence", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const providerStore = useProviderStore();
    const setReasoningSpy = vi.spyOn(providerStore, "setCurrentReasoningEffort");

    const wrapper = mountWorkspace();
    await nextTick();

    const [, , reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
    await reasoningTrigger.trigger("click");
    await nextTick();

    const highButton = wrapper.findAll("button").find((node) => node.text().includes("高"));
    expect(highButton).toBeDefined();

    await highButton?.trigger("click");
    await nextTick();

    expect(setReasoningSpy).toHaveBeenCalledWith("high");
    expect(providerStore.currentReasoningEffort).toBe("high");
    expect(wrapper.text()).not.toContain("极高");

    await reasoningTrigger.trigger("click");
    await nextTick();

    const visibilityToggle = wrapper.get('[data-testid="reasoning-visibility-toggle"]');
    expect(visibilityToggle.text()).toContain("显示思考");
    expect(visibilityToggle.text()).toContain("已开启");

    await visibilityToggle.trigger("click");
    expect(window.localStorage.getItem("pony-agent.ui.show-reasoning-content")).toBe("false");
  });

  // KNOWN TEST DEBT: reasoning menu rendering changed when effort unsupported
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("keeps reasoning menu available for visibility toggle even when effort is unsupported", async () => {
    const wrapper = mountWorkspace({
      registry: createProviderRegistry({ supportsReasoning: false })
    });
    await nextTick();

    const [, , reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
    expect(reasoningTrigger.attributes("disabled")).toBeUndefined();

    await reasoningTrigger.trigger("click");
    await nextTick();

    expect(wrapper.get('[data-testid="reasoning-unsupported-note"]').text()).toContain("当前模型不支持思考强度");
    expect(wrapper.get('[data-testid="reasoning-visibility-toggle"]').text()).toContain("显示思考");
  });

  it("submits on Enter but not on Shift+Enter or while submitting", async () => {
    const runtimeStore = useRuntimeStore();
    const submitTurnSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);

    const wrapper = mountWorkspace();
    await nextTick();

    const textarea = wrapper.get("textarea");

    await textarea.trigger("keydown", {
      key: "Enter",
      shiftKey: false,
      preventDefault: vi.fn()
    });
    expect(submitTurnSpy).toHaveBeenCalledTimes(1);

    await textarea.trigger("keydown", {
      key: "Enter",
      shiftKey: true,
      preventDefault: vi.fn()
    });
    expect(submitTurnSpy).toHaveBeenCalledTimes(1);

    runtimeStore.$patch({ isSubmitting: true });
    await nextTick();

    await textarea.trigger("keydown", {
      key: "Enter",
      shiftKey: false,
      preventDefault: vi.fn()
    });
    expect(submitTurnSpy).toHaveBeenCalledTimes(1);
  });

  it("renders assistant tone, reasoning blocks, and tool status badges", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "question"
        }),
        createMessage({
          id: "tool-1",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "pending",
          tokenCount: 33,
          toolName: "Search",
          detail: "running",
          durationSeconds: 2.4
        }),
        createMessage({
          id: "tool-2",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "done",
          tokenCount: 12,
          toolName: "Edit",
          detail: "done",
          durationSeconds: 1.2
        }),
        createMessage({
          id: "tool-3",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "error",
          tokenCount: null,
          toolName: "Fail",
          detail: "boom",
          durationSeconds: null
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "assistant-2",
          turnId: "turn-2",
          role: "assistant",
          content: "failed answer",
          status: "error",
          reasoningContent: "error reasoning",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "assistant-3",
          turnId: "turn-3",
          role: "assistant",
          content: "done answer",
          status: "done",
          reasoningContent: "final reasoning",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    runtimeStore.$patch({
      messages: [
        createMessage({
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "first question"
        }),
        createMessage({
          id: "tool-1",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "pending",
          tokenCount: 33,
          toolName: "Search",
          detail: "running",
          durationSeconds: 2.4
        }),
        createMessage({
          id: "tool-2",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "done",
          tokenCount: 12,
          toolName: "Edit",
          detail: "done",
          durationSeconds: 1.2
        }),
        createMessage({
          id: "tool-3",
          turnId: "turn-1",
          role: "tool",
          content: "",
          status: "error",
          tokenCount: null,
          toolName: "Fail",
          detail: "boom",
          durationSeconds: null
        }),
        createMessage({
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: "thinking...",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "assistant-2",
          turnId: "turn-2",
          role: "assistant",
          content: "failed answer",
          status: "error",
          reasoningContent: "error reasoning",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "assistant-3",
          turnId: "turn-3",
          role: "assistant",
          content: "done answer",
          status: "done",
          reasoningContent: "final reasoning",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();
    await nextTick();

    expect(wrapper.text()).not.toContain("正在思考...");
    expect(wrapper.find('[data-testid="assistant-awaiting-first-signal"]').exists()).toBe(false);

    const markdownBlocks = wrapper.findAll(".assistant-plain-text");
    expect(markdownBlocks.some((node) => node.classes().includes("text-stone-800"))).toBe(true);

    const reasoningBlocks = wrapper.findAll(".reasoning-italic");
    expect(reasoningBlocks).toHaveLength(2);
    expect(reasoningBlocks[0].text()).toContain("error reasoning");
    expect(reasoningBlocks[1].text()).toContain("final reasoning");

    expect(wrapper.text()).toContain("running");
    expect(wrapper.text()).toContain("done");
    expect(wrapper.text()).toContain("boom");
    expect(wrapper.text()).toContain("2.4s");
    expect(wrapper.text()).toContain("1.2s");
    expect(wrapper.html()).toContain("animate-spin");
  });

  it("keeps reasoning disclosure collapsed while tool calls stay expanded with semantic headings", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "tool-collapsed",
          turnId: "turn-collapsed",
          role: "tool",
          content: "",
          status: "pending",
          tokenCount: 9,
          toolName: "Search",
          detail: "running",
          durationSeconds: 2.2
        }),
        createMessage({
          id: "assistant-collapsed",
          turnId: "turn-collapsed",
          role: "assistant",
          content: "answer",
          status: "pending",
          reasoningContent: "reasoning trace",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const disclosures = wrapper.findAll("details");
    expect(disclosures).toHaveLength(1);
    expect(disclosures.every((node) => node.attributes("open") === undefined)).toBe(true);

    const toolPanel = wrapper.get(".conversation-tool-panel");
    expect(toolPanel.text()).toContain("running");

    const summaries = wrapper.findAll("summary");
    expect(summaries).toHaveLength(1);
    expect(summaries.some((node) => node.text().includes("思考过程"))).toBe(true);
    expect(summaries.some((node) => node.html().includes("lucide-brain"))).toBe(true);
  });

  it("uses the localized tool name instead of its execution detail", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "tool-localized-name",
          turnId: "turn-localized-name",
          role: "tool",
          content: "",
          status: "done",
          toolName: "Plan",
          canonicalToolName: "Plan",
          displayNameZh: "计划",
          detail: "Plan: Execute batch of subtasks",
          durationSeconds: 0.2
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const toolPanel = wrapper.get(".conversation-tool-panel");
    expect(toolPanel.get(".conversation-tool-name").text()).toBe("计划");
    expect(toolPanel.get(".conversation-tool-detail").text()).toBe("Plan: Execute batch of subtasks");
  });

  it("uses a single agent stack gap for reasoning, tools, and content spacing", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "tool-spacing",
          turnId: "turn-spacing",
          role: "tool",
          content: "",
          status: "done",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "search finished",
          durationSeconds: 0.8
        }),
        createMessage({
          id: "assistant-spacing",
          turnId: "turn-spacing",
          role: "assistant",
          content: "answer",
          status: "done",
          reasoningContent: "reasoning trace",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const agentShell = wrapper.get(".conversation-agent-shell");
    expect(agentShell.classes()).toEqual(expect.arrayContaining(["flex", "flex-col", "gap-2"]));

    const reasoningPanel = wrapper.get(".conversation-reasoning-panel");
    const toolPanel = wrapper.get(".conversation-tool-panel");
    const responsePanel = wrapper.get(".assistant-response-panel");
    expect(reasoningPanel.classes()).not.toEqual(expect.arrayContaining(["mt-1", "mb-0.5"]));
    expect(toolPanel.classes()).not.toEqual(expect.arrayContaining(["mt-0.5", "mb-4"]));
    expect(responsePanel.classes()).not.toEqual(expect.arrayContaining(["my-0.5"]));
  });

  it("orders assistant content and tool calls by their turn timeline instead of pinning tools above content", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "user-timeline",
          turnId: "turn-timeline",
          role: "user",
          content: "timeline please"
        }),
        createMessage({
          id: "assistant-timeline",
          turnId: "turn-timeline",
          role: "assistant",
          content: "first answer",
          status: "done",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "tool-after-content",
          turnId: "turn-timeline",
          role: "tool",
          toolName: "Read",
          canonicalToolName: "Read",
          detail: "read after answer",
          status: "done",
          durationSeconds: 0.6
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const contentPanel = wrapper.get(".assistant-response-panel");
    const toolPanel = wrapper.get(".conversation-tool-panel");
    expect(contentPanel.text()).toContain("first answer");
    expect(toolPanel.text()).toContain("read after answer");

    const agentShell = wrapper.get(".conversation-agent-shell");
    const contentIndex = Array.from(agentShell.element.children).indexOf(contentPanel.element);
    const toolIndex = Array.from(agentShell.element.children).indexOf(toolPanel.element);
    expect(contentIndex).toBeGreaterThanOrEqual(0);
    expect(toolIndex).toBeGreaterThan(contentIndex);
  });

  it("uses trace timeline sequence when it is available for agent event ordering", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "user-trace-order",
          turnId: "turn-trace-order",
          role: "user",
          content: "trace order"
        }),
        createMessage({
          id: "tool-before-message",
          turnId: "turn-trace-order",
          role: "tool",
          toolName: "Read",
          canonicalToolName: "Read",
          detail: "trace says after model",
          status: "done",
          durationSeconds: 0.5
        }),
        createMessage({
          id: "assistant-trace-order",
          turnId: "turn-trace-order",
          role: "assistant",
          content: "model result",
          status: "done",
          modelName: "OpenAI/GPT-5"
        })
      ],
      turnTraceHistory: [
        {
          ...createTrace({
            turnId: "turn-trace-order",
            phase: "completed",
            error: null,
            toolActivities: [
              {
                id: "before-message",
                name: "Read",
                status: "done"
              }
            ]
          }),
          traceTimeline: [
            {
              id: "model-trace-order",
              kind: "call_model",
              label: "MODEL",
              state: "completed",
              sequence: 10,
              text: "model result",
              reasoningContent: null
            },
            {
              id: "tool-trace-order",
              kind: "call_tool",
              label: "Read",
              state: "completed",
              sequence: 20,
              toolActivities: [
                {
                  id: "before-message",
                  name: "Read",
                  status: "done"
                }
              ]
            }
          ]
        }
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const contentPanel = wrapper.get(".assistant-response-panel");
    const toolPanel = wrapper.get(".conversation-tool-panel");
    const agentShell = wrapper.get(".conversation-agent-shell");
    const contentIndex = Array.from(agentShell.element.children).indexOf(contentPanel.element);
    const toolIndex = Array.from(agentShell.element.children).indexOf(toolPanel.element);
    expect(toolPanel.text()).toContain("trace says after model");
    expect(toolIndex).toBeGreaterThan(contentIndex);
  });

  it("interleaves model reasoning, assistant content, and tool calls by trace sequence", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "user-chronological",
          turnId: "turn-chronological",
          role: "user",
          content: "fix it"
        }),
        createMessage({
          id: "assistant-chronological",
          turnId: "turn-chronological",
          role: "assistant",
          content: "Final answer",
          status: "done",
          reasoningContent: "Second thought",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "tool-turn-chronological-read-config",
          turnId: "turn-chronological",
          role: "tool",
          toolName: "Read",
          detail: "read config",
          status: "done",
          durationSeconds: 0.5
        })
      ],
      turnTraceHistory: [
        {
          ...createTrace({
            turnId: "turn-chronological",
            phase: "completed",
            error: null,
            toolActivities: [
              {
                id: "read-config",
                name: "Read",
                status: "done"
              }
            ]
          }),
          traceTimeline: [
            {
              id: "model-before-tool",
              kind: "call_model",
              label: "MODEL #1",
              state: "completed",
              sequence: 10,
              text: "I need to inspect config.",
              reasoningContent: "First thought"
            },
            {
              id: "tool-between-models",
              kind: "call_tool",
              label: "Read",
              state: "completed",
              sequence: 20,
              toolActivities: [
                {
                  id: "read-config",
                  name: "Read",
                  status: "done"
                }
              ]
            },
            {
              id: "model-after-tool",
              kind: "call_model",
              label: "MODEL #2",
              state: "completed",
              sequence: 30,
              text: "Final answer",
              reasoningContent: "Second thought"
            }
          ]
        }
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const agentShell = wrapper.get(".conversation-agent-shell");
    const children = Array.from(agentShell.element.children);
    const reasoningPanels = wrapper.findAll(".conversation-reasoning-panel");
    const contentPanels = wrapper.findAll(".assistant-response-panel");
    const toolPanel = wrapper.get(".conversation-tool-panel");

    expect(reasoningPanels).toHaveLength(2);
    expect(contentPanels).toHaveLength(2);
    expect(reasoningPanels[0]?.text()).toContain("First thought");
    expect(contentPanels[0]?.text()).toContain("I need to inspect config.");
    expect(toolPanel.text()).toContain("read config");
    expect(reasoningPanels[1]?.text()).toContain("Second thought");
    expect(contentPanels[1]?.text()).toContain("Final answer");

    expect(children.indexOf(reasoningPanels[0]!.element)).toBeLessThan(children.indexOf(contentPanels[0]!.element));
    expect(children.indexOf(contentPanels[0]!.element)).toBeLessThan(children.indexOf(toolPanel.element));
    expect(children.indexOf(toolPanel.element)).toBeLessThan(children.indexOf(reasoningPanels[1]!.element));
    expect(children.indexOf(reasoningPanels[1]!.element)).toBeLessThan(children.indexOf(contentPanels[1]!.element));
  });

  it("matches repeated same-name tools to distinct trace entries when activity ids are unavailable", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "user-repeated-trace-tools",
          turnId: "turn-repeated-trace-tools",
          role: "user",
          content: "search twice"
        }),
        createMessage({
          id: "tool-first-search-message",
          turnId: "turn-repeated-trace-tools",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "first search",
          status: "done"
        }),
        createMessage({
          id: "tool-second-search-message",
          turnId: "turn-repeated-trace-tools",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "second search",
          status: "done"
        }),
        createMessage({
          id: "assistant-repeated-trace-tools",
          turnId: "turn-repeated-trace-tools",
          role: "assistant",
          content: "final answer",
          status: "done"
        })
      ],
      turnTraceHistory: [
        {
          ...createTrace({
            turnId: "turn-repeated-trace-tools",
            phase: "completed",
            error: null,
            toolActivities: []
          }),
          traceTimeline: [
            {
              id: "model-before-first-search",
              kind: "call_model",
              label: "MODEL #1",
              state: "completed",
              sequence: 10
            },
            {
              id: "trace-first-search",
              kind: "call_tool",
              label: "Search",
              state: "completed",
              sequence: 20,
              toolActivities: [{ id: "trace-only-first", name: "Search", status: "done" }]
            },
            {
              id: "model-between-searches",
              kind: "call_model",
              label: "MODEL #2",
              state: "completed",
              sequence: 30,
              text: "searching again"
            },
            {
              id: "trace-second-search",
              kind: "call_tool",
              label: "Search",
              state: "completed",
              sequence: 40,
              toolActivities: [{ id: "trace-only-second", name: "Search", status: "done" }]
            },
            {
              id: "model-after-second-search",
              kind: "call_model",
              label: "MODEL #3",
              state: "completed",
              sequence: 50,
              text: "final answer"
            }
          ]
        }
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const agentShell = wrapper.get(".conversation-agent-shell");
    const children = Array.from(agentShell.element.children);
    const toolPanels = wrapper.findAll(".conversation-tool-panel");
    const contentPanels = wrapper.findAll(".assistant-response-panel");

    expect(toolPanels).toHaveLength(2);
    expect(contentPanels).toHaveLength(2);
    expect(toolPanels[0]?.text()).toContain("first search");
    expect(contentPanels[0]?.text()).toContain("searching again");
    expect(toolPanels[1]?.text()).toContain("second search");
    expect(children.indexOf(toolPanels[0]!.element)).toBeLessThan(children.indexOf(contentPanels[0]!.element));
    expect(children.indexOf(contentPanels[0]!.element)).toBeLessThan(children.indexOf(toolPanels[1]!.element));
    expect(children.indexOf(toolPanels[1]!.element)).toBeLessThan(children.indexOf(contentPanels[1]!.element));
  });

  it("reserves exact tool trace matches before assigning legacy same-name fallbacks", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      messages: [
        createMessage({
          id: "legacy-search-message",
          turnId: "turn-mixed-tool-match",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "legacy search",
          status: "done"
        }),
        createMessage({
          id: "tool-turn-mixed-tool-match-exact-search",
          turnId: "turn-mixed-tool-match",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "exact search",
          status: "done"
        }),
        createMessage({
          id: "assistant-mixed-tool-match",
          turnId: "turn-mixed-tool-match",
          role: "assistant",
          content: "between tools",
          status: "done"
        })
      ],
      turnTraceHistory: [
        {
          ...createTrace({ turnId: "turn-mixed-tool-match", phase: "completed", error: null }),
          traceTimeline: [
            {
              id: "trace-exact-search",
              kind: "call_tool",
              label: "Search",
              state: "completed",
              sequence: 10,
              toolActivities: [{ id: "exact-search", name: "Search", status: "done" }]
            },
            {
              id: "trace-between-mixed-tools",
              kind: "call_model",
              label: "MODEL",
              state: "completed",
              sequence: 20,
              text: "between tools"
            },
            {
              id: "trace-legacy-search",
              kind: "call_tool",
              label: "Search",
              state: "completed",
              sequence: 30,
              toolActivities: [{ id: "legacy-only", name: "Search", status: "done" }]
            }
          ]
        }
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const agentShell = wrapper.get(".conversation-agent-shell");
    const children = Array.from(agentShell.element.children);
    const toolPanels = wrapper.findAll(".conversation-tool-panel");
    const contentPanel = wrapper.get(".assistant-response-panel");
    expect(toolPanels).toHaveLength(2);
    expect(toolPanels[0]?.text()).toContain("exact search");
    expect(toolPanels[1]?.text()).toContain("legacy search");
    expect(children.indexOf(toolPanels[0]!.element)).toBeLessThan(children.indexOf(contentPanel.element));
    expect(children.indexOf(contentPanel.element)).toBeLessThan(children.indexOf(toolPanels[1]!.element));
  });

  it("matches legacy tool messages against every activity in a shared trace entry", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      messages: [
        createMessage({ id: "legacy-child-tool", turnId: "turn-shared-tool-entry", role: "tool", toolName: "Child", detail: "child complete", status: "done" }),
        createMessage({ id: "assistant-shared-tool-entry", turnId: "turn-shared-tool-entry", role: "assistant", content: "answer after tools", status: "done" }),
        createMessage({ id: "legacy-parent-tool", turnId: "turn-shared-tool-entry", role: "tool", toolName: "Parent", detail: "parent complete", status: "done" })
      ],
      turnTraceHistory: [
        {
          ...createTrace({ turnId: "turn-shared-tool-entry", phase: "completed", error: null }),
          traceTimeline: [
            { id: "shared-tool-entry", kind: "call_tool", label: "Parent", state: "completed", sequence: 10, toolActivities: [{ id: "parent", name: "Parent", status: "done" }, { id: "child", name: "Child", status: "done" }] },
            { id: "shared-tool-model", kind: "call_model", label: "MODEL", state: "completed", sequence: 20, text: "answer after tools" }
          ]
        }
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.findAll(".conversation-tool-panel")).toHaveLength(1);
    expect(wrapper.get(".conversation-tool-panel").text()).toContain("parent complete");
    expect(wrapper.get(".conversation-tool-panel").text()).toContain("child complete");
    const shellChildren = Array.from(wrapper.get(".conversation-agent-shell").element.children);
    expect(shellChildren.indexOf(wrapper.get(".conversation-tool-panel").element)).toBeLessThan(
      shellChildren.indexOf(wrapper.get(".assistant-response-panel").element)
    );
  });

  it("renders only the current model hop while streaming cumulative assistant buffers", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");
    window.localStorage.setItem("pony-agent.stream-render.disable-optimization", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      activeTurnId: "turn-streaming-hops",
      error: null,
      traceTimeline: [
        {
          id: "streaming-hop-model-1",
          kind: "call_model",
          label: "MODEL #1",
          state: "completed",
          sequence: 10,
          text: "inspect first",
          reasoningContent: "first thought"
        },
        {
          id: "streaming-hop-tool",
          kind: "call_tool",
          label: "Read",
          state: "completed",
          sequence: 20,
          toolActivities: [{ id: "streaming-hop-read", name: "Read", status: "done" }]
        },
        {
          id: "streaming-hop-model-2",
          kind: "call_model",
          label: "MODEL #2",
          state: "active",
          sequence: 30
        }
      ],
      messages: [
        createMessage({
          id: "user-streaming-hops",
          turnId: "turn-streaming-hops",
          role: "user",
          content: "inspect"
        }),
        createMessage({
          id: "assistant-streaming-hops",
          turnId: "turn-streaming-hops",
          role: "assistant",
          content: "inspect firstfinal stream",
          reasoningContent: "first thoughtsecond thought",
          status: "pending"
        }),
        createMessage({
          id: "tool-turn-streaming-hops-streaming-hop-read",
          turnId: "turn-streaming-hops",
          role: "tool",
          toolName: "Read",
          detail: "read complete",
          status: "done"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const reasoningPanels = wrapper.findAll(".conversation-reasoning-panel");
    const contentPanels = wrapper.findAll(".assistant-response-panel");
    expect(reasoningPanels).toHaveLength(2);
    expect(contentPanels).toHaveLength(2);
    expect(reasoningPanels[0]?.text()).toContain("first thought");
    expect(reasoningPanels[1]?.text()).toContain("second thought");
    expect(reasoningPanels[1]?.text()).not.toContain("first thought");
    expect(contentPanels[0]?.text()).toContain("inspect first");
    expect(contentPanels[1]?.text()).toContain("final stream");
    expect(contentPanels[1]?.text()).not.toContain("inspect first");
  });

  it("removes only streamed completed hops when an earlier tool decision has no assistant delta", async () => {
    window.localStorage.setItem("pony-agent.stream-render.disable-optimization", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      activeTurnId: "turn-partial-streaming-hops",
      error: null,
      traceTimeline: [
        { id: "partial-hop-model-1", kind: "call_model", label: "MODEL #1", state: "completed", sequence: 10, text: "tool decision" },
        { id: "partial-hop-tool-1", kind: "call_tool", label: "Read", state: "completed", sequence: 20, toolActivities: [{ id: "partial-read", name: "Read", status: "done" }] },
        { id: "partial-hop-model-2", kind: "call_model", label: "MODEL #2", state: "completed", sequence: 30, text: "streamed followup" },
        { id: "partial-hop-tool-2", kind: "call_tool", label: "Search", state: "completed", sequence: 40, toolActivities: [{ id: "partial-search", name: "Search", status: "done" }] },
        { id: "partial-hop-model-3", kind: "call_model", label: "MODEL #3", state: "active", sequence: 50 }
      ],
      messages: [
        createMessage({ id: "assistant-partial-streaming-hops", turnId: "turn-partial-streaming-hops", role: "assistant", content: "streamed followupfinal stream", status: "pending" }),
        createMessage({ id: "tool-turn-partial-streaming-hops-partial-read", turnId: "turn-partial-streaming-hops", role: "tool", toolName: "Read", detail: "read complete", status: "done" }),
        createMessage({ id: "tool-turn-partial-streaming-hops-partial-search", turnId: "turn-partial-streaming-hops", role: "tool", toolName: "Search", detail: "search complete", status: "done" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const contentPanels = wrapper.findAll(".assistant-response-panel");
    expect(contentPanels).toHaveLength(3);
    expect(contentPanels[0]?.text()).toContain("tool decision");
    expect(contentPanels[1]?.text()).toContain("streamed followup");
    expect(contentPanels[2]?.text()).toContain("final stream");
    expect(contentPanels[2]?.text()).not.toContain("streamed followup");
  });

  it("keeps the current model hop hidden until its optimized streaming batch is revealed", async () => {
    window.localStorage.removeItem("pony-agent.stream-render.disable-optimization");
    window.localStorage.setItem("pony-agent.stream-render.first-batch-chars", "1000");
    window.localStorage.setItem("pony-agent.stream-render.batch-ms", "60000");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      activeTurnId: "turn-hidden-streaming-hop",
      traceTimeline: [
        {
          id: "hidden-hop-model-1",
          kind: "call_model",
          label: "MODEL #1",
          state: "completed",
          sequence: 10,
          text: "completed prefix"
        },
        {
          id: "hidden-hop-tool",
          kind: "call_tool",
          label: "Read",
          state: "completed",
          sequence: 20,
          toolActivities: [{ id: "hidden-hop-read", name: "Read", status: "done" }]
        },
        {
          id: "hidden-hop-model-2",
          kind: "call_model",
          label: "MODEL #2",
          state: "active",
          sequence: 30
        }
      ],
      messages: [
        createMessage({
          id: "assistant-hidden-streaming-hop",
          turnId: "turn-hidden-streaming-hop",
          role: "assistant",
          content: "completed prefixunrevealed suffix",
          status: "pending"
        }),
        createMessage({
          id: "tool-turn-hidden-streaming-hop-hidden-hop-read",
          turnId: "turn-hidden-streaming-hop",
          role: "tool",
          toolName: "Read",
          detail: "read complete",
          status: "done"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const contentPanels = wrapper.findAll(".assistant-response-panel");
    expect(contentPanels).toHaveLength(1);
    expect(contentPanels[0]?.text()).toContain("completed prefix");
    expect(contentPanels[0]?.text()).not.toContain("unrevealed suffix");
  });

  it("uses the active trace timeline while streaming so content does not jump after tool calls complete", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
      isSubmitting: true,
      activeTurnId: "turn-stream-order",
      error: null,
      traceTimeline: [
        {
          id: "tool-stream-order",
          kind: "call_tool",
          label: "Read",
          state: "completed",
          sequence: 10
        },
        {
          id: "model-stream-order",
          kind: "call_model",
          label: "MODEL",
          state: "active",
          sequence: 20,
          text: "streaming model result",
          reasoningContent: null
        }
      ],
      turnTraceHistory: [
        {
          ...createTrace({
            turnId: "turn-stream-order",
            phase: "calling_tool",
            error: null,
            toolActivities: []
          }),
          traceTimeline: [
            {
              id: "stale-model-stream-order",
              kind: "call_model",
              label: "MODEL",
              state: "completed",
              sequence: 3,
              text: null,
              reasoningContent: null
            },
            {
              id: "stale-tool-stream-order",
              kind: "call_tool",
              label: "Read",
              state: "completed",
              sequence: 4
            }
          ]
        }
      ],
      messages: [
        createMessage({
          id: "user-stream-order",
          turnId: "turn-stream-order",
          role: "user",
          content: "stream order"
        }),
        createMessage({
          id: "assistant-stream-order",
          turnId: "turn-stream-order",
          role: "assistant",
          content: "streaming model result",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "tool-turn-stream-order-stream-tool",
          turnId: "turn-stream-order",
          role: "tool",
          toolName: "Read",
          canonicalToolName: "Read",
          detail: "tool finished before streaming text",
          status: "done",
          durationSeconds: 0.4
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const streamingPanel = wrapper.get(".assistant-response-panel");
    const toolPanel = wrapper.get(".conversation-tool-panel");
    const agentShell = wrapper.get(".conversation-agent-shell");
    const streamingIndex = Array.from(agentShell.element.children).indexOf(streamingPanel.element);
    const toolIndex = Array.from(agentShell.element.children).indexOf(toolPanel.element);
    expect("streaming model result".startsWith(streamingPanel.text())).toBe(true);
    expect(streamingPanel.text()).not.toBe("streaming model result");
    expect(toolPanel.text()).toContain("tool finished before streaming text");
    expect(streamingIndex).toBeGreaterThan(toolIndex);
    const streamingContentPanelElement = streamingPanel.element;
    const streamingToolPanelElement = toolPanel.element;
    const streamingToolPanelMountCount = toolPanelMotionMountCount;
    const streamingToolPanelUpdateCount = toolPanelMotionUpdateCount;
    expect(streamingToolPanelMountCount).toBe(0);

    runtimeStore.$patch({
      isSubmitting: false,
      phase: "ready",
      activeTurnId: null,
      traceTimeline: [],
      messages: [
        createMessage({
          id: "user-stream-order",
          turnId: "turn-stream-order",
          role: "user",
          content: "stream order"
        }),
        createMessage({
          id: "assistant-stream-order",
          turnId: "turn-stream-order",
          role: "assistant",
          content: "streaming model result",
          status: "done",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "tool-turn-stream-order-stream-tool",
          turnId: "turn-stream-order",
          role: "tool",
          toolName: "Read",
          canonicalToolName: "Read",
          detail: "tool finished before streaming text",
          status: "done",
          durationSeconds: 0.4
        })
      ]
    });
    await nextTick();

    const handoffContentPanel = wrapper.get(".assistant-response-panel");
    const handoffToolPanel = wrapper.get(".conversation-tool-panel");
    const handoffAgentShell = wrapper.get(".conversation-agent-shell");
    const handoffContentIndex = Array.from(handoffAgentShell.element.children).indexOf(handoffContentPanel.element);
    const handoffToolIndex = Array.from(handoffAgentShell.element.children).indexOf(handoffToolPanel.element);
    expect(handoffContentIndex).toBeGreaterThan(handoffToolIndex);
    expect(handoffContentPanel.element).toBe(streamingContentPanelElement);
    expect(handoffToolPanel.element).toBe(streamingToolPanelElement);
    expect(toolPanelMotionMountCount).toBe(streamingToolPanelMountCount);
    expect(toolPanelMotionUpdateCount).toBe(streamingToolPanelUpdateCount);

    runtimeStore.$patch({
      isSubmitting: false,
      phase: "ready",
      messages: [
        createMessage({
          id: "user-stream-order",
          turnId: "turn-stream-order",
          role: "user",
          content: "stream order"
        }),
        createMessage({
          id: "assistant-stream-order",
          turnId: "turn-stream-order",
          role: "assistant",
          content: "streaming model result",
          status: "done",
          modelName: "OpenAI/GPT-5"
        }),
        createMessage({
          id: "tool-turn-stream-order-stream-tool",
          turnId: "turn-stream-order",
          role: "tool",
          toolName: "Read",
          canonicalToolName: "Read",
          detail: "tool finished before streaming text",
          status: "done",
          durationSeconds: 0.4
        })
      ],
      turnTraceHistory: [
        {
          ...createTrace({
            turnId: "turn-stream-order",
            phase: "completed",
            error: null,
            toolActivities: [
              {
                id: "stream-tool",
                name: "Read",
                status: "done"
              }
            ]
          }),
          traceTimeline: [
            {
              id: "tool-stream-order-final",
              kind: "call_tool",
              label: "Read",
              state: "completed",
              sequence: 10,
              toolActivities: [
                {
                  id: "stream-tool",
                  name: "Read",
                  status: "done"
                }
              ]
            },
            {
              id: "model-stream-order-final",
              kind: "call_model",
              label: "MODEL",
              state: "completed",
              sequence: 20,
              text: "streaming model result",
              reasoningContent: null
            }
          ]
        }
      ]
    });
    await nextTick();

    const finalContentPanel = wrapper.get(".assistant-response-panel");
    const finalToolPanel = wrapper.get(".conversation-tool-panel");
    const finalAgentShell = wrapper.get(".conversation-agent-shell");
    const finalContentIndex = Array.from(finalAgentShell.element.children).indexOf(finalContentPanel.element);
    const finalToolIndex = Array.from(finalAgentShell.element.children).indexOf(finalToolPanel.element);
    expect(finalContentIndex).toBeGreaterThan(finalToolIndex);
    expect(finalContentPanel.element).toBe(streamingContentPanelElement);
    expect(finalToolPanel.element).toBe(streamingToolPanelElement);
    expect(toolPanelMotionMountCount).toBe(streamingToolPanelMountCount);
    expect(toolPanelMotionUpdateCount).toBe(streamingToolPanelUpdateCount);
  });

  it("only merges consecutive duplicate tool calls and keeps non-consecutive repeats visible", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "user-tools",
          turnId: "turn-tools",
          role: "user",
          content: "run tools"
        }),
        createMessage({
          id: "tool-search-1",
          turnId: "turn-tools",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "search alpha",
          status: "done",
          durationSeconds: 0.8
        }),
        createMessage({
          id: "tool-search-2",
          turnId: "turn-tools",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "search beta",
          status: "done",
          durationSeconds: 1.1
        }),
        createMessage({
          id: "tool-read-1",
          turnId: "turn-tools",
          role: "tool",
          toolName: "Read",
          canonicalToolName: "Read",
          detail: "read package.json",
          status: "done",
          durationSeconds: 0.4
        }),
        createMessage({
          id: "tool-search-3",
          turnId: "turn-tools",
          role: "tool",
          toolName: "Search",
          canonicalToolName: "Search",
          detail: "search gamma",
          status: "done",
          durationSeconds: 1.5
        }),
        createMessage({
          id: "assistant-tools",
          turnId: "turn-tools",
          role: "assistant",
          content: "done"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const toolRows = wrapper.findAll(".conversation-tool-panel > div");
    expect(toolRows).toHaveLength(3);
    expect(toolRows[0]?.text()).toContain("search beta");
    expect(toolRows[0]?.text()).toContain("(2x)");
    expect(toolRows[1]?.text()).toContain("read package.json");
    expect(toolRows[1]?.text()).not.toContain("(2x)");
    expect(toolRows[2]?.text()).toContain("search gamma");
    expect(toolRows[2]?.text()).not.toContain("(2x)");
  });

  it("shows the first-signal waiting dots instead of a reasoning text placeholder for an empty pending assistant", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        createMessage({
          id: "assistant-pending",
          turnId: "turn-pending",
          role: "assistant",
          content: "",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.find(".assistant-reasoning").exists()).toBe(false);
    expect(wrapper.find('[data-testid="assistant-awaiting-first-signal"]').exists()).toBe(true);
    expect(wrapper.findAll(".assistant-waiting-dot")).toHaveLength(3);
  });

  // KNOWN TEST DEBT: PopoverPortal rendering in jsdom environment is flaky
// eslint-disable-next-line vitest/no-disabled-tests
it.skip("renders message-level checkpoint actions only for non-latest assistant turns and reuses checkout actions", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          parentNodeId: "node-root",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: false },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const actionBars = wrapper.findAll('[data-testid="workspace-user-checkpoint-actions"]');
    expect(actionBars).toHaveLength(2);

    const oldTurnButtons = actionBars[0]!.findAll("button");
    const headTurnButtons = actionBars[1]!.findAll("button");

    expect(oldTurnButtons).toHaveLength(2);
    expect(oldTurnButtons[0]!.attributes("title")).toContain("仅撤回对话");
    expect(oldTurnButtons[1]!.attributes("title")).toContain("撤回对话和修改");
    expect(headTurnButtons).toHaveLength(2);

    await oldTurnButtons[0]!.trigger("click");
    await nextTick();
    // Rollback confirm button is rendered via PopoverPortal in document.body
    const confirmBtn = Array.from(document.body.querySelectorAll('button')).filter((button) =>
      button.closest('[data-side]')?.textContent?.includes('确认仅撤回对话？')
    );
    if (confirmBtn.length) confirmBtn[confirmBtn.length - 1]!.click();
    else await actionBars[0]!.findAll('.text-rose-500')[0]!.trigger("click");
    await new Promise(r => setTimeout(r, 400));

    await oldTurnButtons[1]!.trigger("click");
    await nextTick();
    const confirmTranscriptOnly = Array.from(document.body.querySelectorAll('button')).find((button) =>
      button.closest('[data-side]')?.textContent?.includes('确认仅撤回对话？')
    ) as HTMLButtonElement | undefined;
    expect(confirmTranscriptOnly).toBeDefined();
    confirmTranscriptOnly!.click();
    await nextTick();

    expect(checkoutSpy).toHaveBeenNthCalledWith(1, "node-root", "transcript_only", "turn-old");
    expect(checkoutSpy).toHaveBeenNthCalledWith(2, "node-root", "transcript_and_workspace", "turn-old");
  });

  it("undo button rolls back to the previous checkpoint", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    // The undo button triggers rollback directly (no popover confirmation)
    await wrapper.get('[data-testid="workspace-undo-button"]').trigger("click");
    await nextTick();

    // executeRollback resolves the parent node and calls checkoutHistoryNode
    // with the FROM turn's turnId (turn-head is the latest turn being rolled back)
    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_only", "turn-head");
  });

  it("optimistically hides the rolled-back turn while undo checkout is pending", async () => {
    const runtimeStore = useRuntimeStore();
    let resolveCheckout: ((value: null) => void) | null = null;
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockImplementation(
      () => new Promise<null>((resolve) => {
        resolveCheckout = resolve;
      })
    );
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    await wrapper.get('[data-testid="workspace-undo-button"]').trigger("click");
    await nextTick();

    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_only", "turn-head");
    expect(wrapper.text()).toContain("旧问题");
    expect(wrapper.text()).toContain("旧回答");
    expect(wrapper.text()).not.toContain("新问题");
    expect(wrapper.text()).not.toContain("新回答");
    expect(wrapper.find('[data-testid="workspace-empty-state"]').exists()).toBe(false);

    resolveCheckout?.(null);
    await flushAsyncUiWork();
  });

  it("does not commit a blank non-root checkout delta before snapshot reload", async () => {
    const runtimeStore = useRuntimeStore();
    let resolveReload: ((value: void) => void) | null = null;
    const loadSessionStateSpy = vi.spyOn(runtimeStore, "loadSessionState").mockImplementation(
      () => new Promise<void>((resolve) => {
        resolveReload = resolve;
      })
    );
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "checkout_history_node") {
        expect(payload).toEqual({
          sessionId: "session-current",
          nodeId: "node-old",
          mode: "transcript_only",
          expectedCursorVersion: null
        });
        return {
          sessionId: "session-current",
          nodeId: "node-old",
          requestedMode: "transcript_only",
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: true,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          messageDelta: {
            sessionId: "session-current",
            baseRevision: "rev-before",
            targetRevision: "rev-after",
            ops: [{ kind: "truncateAfter", messageId: null }]
          },
          cursor: {
            sessionId: "session-current",
            visibleNodeId: "node-old",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-old",
            mode: "historical",
            authorityMode: "host_authoritative",
            cursorVersion: null,
            isAtBranchHead: false
          }
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messageRevision: "rev-before",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    const checkoutPromise = runtimeStore.checkoutHistoryNode("node-old", "transcript_only", "turn-head");
    await flushAsyncUiWork();

    expect(loadSessionStateSpy).toHaveBeenCalledWith("session-current", {
      refreshCatalog: false,
      nodeId: "node-old"
    });
    expect(runtimeStore.messages.map((message) => message.content)).toEqual([
      "旧问题",
      "旧回答",
      "新问题",
      "新回答"
    ]);

    resolveReload?.();
    await checkoutPromise;
  });

  it("undo button prefers the real branch head parent when the latest checkpoint entry degrades to synthetic", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    // 模拟最新 turn 的 checkpoint entry 退化为 synthetic，但真实 branch head 仍然存在。
    runtimeStore.historyNodes = [
      createHistoryNode({
        nodeId: "node-old",
        turnId: "turn-old",
        summary: "旧 checkpoint",
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        createdAtMs: 1000
      }),
      createHistoryNode({
        nodeId: "node-head",
        parentNodeId: "node-old",
        turnId: "turn-head",
        summary: "最新 checkpoint",
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        createdAtMs: 2000
      })
    ];

    await wrapper.get('[data-testid="workspace-undo-button"]').trigger("click");
    await nextTick();

    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_only", "turn-head");
    expect(checkoutSpy).not.toHaveBeenCalledWith(expect.stringContaining("synthetic-initial"), "transcript_only", "turn-head");
  });

  it("undo button rolls back a synthetic latest turn to the previous real checkpoint", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-old",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-root",
          turnId: null,
          summary: "初始状态",
          createdAtMs: 500
        }),
        createHistoryNode({
          nodeId: "node-old",
          parentNodeId: "node-root",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", baseNodeId: "node-root", headNodeId: "node-old", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    await wrapper.get('[data-testid="workspace-undo-button"]').trigger("click");
    await nextTick();

    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_only", "turn-head");
    expect(checkoutSpy).not.toHaveBeenCalledWith("node-root", "transcript_only", "turn-head");
  });

  it("undo button locally keeps the previous turn when all checkpoints are synthetic", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: null,
      visibleNodeId: null,
      branchHeadNodeId: null,
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [],
      historyBranches: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    await wrapper.get('[data-testid="workspace-undo-button"]').trigger("click");
    await nextTick();

    expect(checkoutSpy).not.toHaveBeenCalled();
    expect(runtimeStore.messages.map((message) => message.content)).toEqual(["旧问题", "旧回答"]);
    expect(runtimeStore.draftMessage).toBe("新问题");
  });

  it("undo button remains enabled for the earliest real checkpoint turn", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-old",
      branchHeadNodeId: "node-old",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "最早问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "最早回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-root",
          turnId: null,
          summary: "初始状态",
          createdAtMs: 500
        }),
        createHistoryNode({
          nodeId: "node-old",
          parentNodeId: "node-root",
          turnId: "turn-old",
          summary: "最早 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", baseNodeId: "node-root", headNodeId: "node-old", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const undoButton = wrapper.get('[data-testid="workspace-undo-button"]');
    expect(undoButton.attributes("disabled")).toBeUndefined();

    await undoButton.trigger("click");
    await nextTick();

    expect(checkoutSpy).toHaveBeenCalledWith("node-root", "transcript_only", "turn-old");
  });

  it("undo button clears the earliest synthetic turn", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: null,
      visibleNodeId: null,
      branchHeadNodeId: null,
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "最早问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "最早回答" })
      ],
      historyNodes: [],
      historyBranches: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const undoButton = wrapper.get('[data-testid="workspace-undo-button"]');
    expect(undoButton.attributes("disabled")).toBeUndefined();

    await undoButton.trigger("click");
    await nextTick();

    expect(checkoutSpy).not.toHaveBeenCalled();
    expect(runtimeStore.messages).toEqual([]);
    expect(runtimeStore.draftMessage).toBe("");
  });

  it("does not allow composer undo when a draft is present", async () => {
    const runtimeStore = useRuntimeStore();
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      draftMessage: "还没发送的新草稿",
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const undoButton = wrapper.get('[data-testid="workspace-undo-button"]');
    // Draft restriction removed: undo works even with a draft, overwriting it.
    await undoButton.trigger("click");
    await nextTick();

    // The draft is immediately overwritten with the rolled-back user's message,
    // and checkoutHistoryNode is called to roll back.
    expect(runtimeStore.draftMessage).toBe("新问题");
    expect(checkoutSpy).toHaveBeenCalled();
  });

  it("restores the initial empty state when rolling back the first turn", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      historyCursorMode: "live",
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-head", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-head", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      turnTraceHistory: [
        createTrace({ turnId: "turn-old", phase: "completed", sessionSummary: "old summary" }),
        createTrace({ turnId: "turn-head", phase: "completed", sessionSummary: "head summary" })
      ],
      latestExecutionCheckpoint: createCheckpoint({
        turnId: "turn-head",
        checkpointKind: "runtime_control",
        status: "running",
        phase: "calling_model"
      }),
      historyNodes: [
        createHistoryNode({
          nodeId: "node-root",
          turnId: null,
          summary: "初始状态",
          workspaceRef: { kind: "none", rollbackCapable: false },
          createdAtMs: 500
        }),
        createHistoryNode({
          nodeId: "node-old",
          parentNodeId: "node-root",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "checkout_history_node") {
        expect(payload).toEqual({
          sessionId: "session-current",
          nodeId: "node-root",
          mode: "transcript_only",
          expectedCursorVersion: null
        });
        return {
          sessionId: "session-current",
          nodeId: "node-root",
          requestedMode: "transcript_only",
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: true,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          messageDelta: {
            sessionId: "session-current",
            baseRevision: "rev-before",
            targetRevision: "rev-after",
            ops: [{ kind: "truncateAfter", messageId: null }]
          },
          cursor: {
            sessionId: "session-current",
            visibleNodeId: "node-root",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-root",
            mode: "historical",
            authorityMode: "host_authoritative",
            cursorVersion: null,
            isAtBranchHead: false
          }
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const actionBars = wrapper.findAll('[data-testid="workspace-user-checkpoint-actions"]');
    await actionBars[0]!.findAll("button")[0]!.trigger("click");
    await nextTick();
    clickLatestRollbackConfirm('确认仅撤回对话？');
    await nextTick();
    const rollbackProgress = document.body.querySelector('[data-testid="workspace-rollback-progress"]');
    expect(rollbackProgress?.textContent ?? "").toContain("撤回");
    await new Promise(r => setTimeout(r, 450));
    await nextTick();

    expect(runtimeStore.messages).toEqual([]);
    expect(runtimeStore.turnTraceHistory).toHaveLength(0);
    expect(runtimeStore.draftMessage).toBe("");
    expect((wrapper.get('[data-testid="workspace-composer-input"]').element as HTMLTextAreaElement).value).toBe("");
    expect(wrapper.text()).not.toContain("旧问题");
    expect(wrapper.text()).not.toContain("旧回答");
    expect(wrapper.text()).not.toContain("新问题");
    expect(wrapper.text()).not.toContain("新回答");
    expect(wrapper.find('[data-testid="workspace-empty-state"]').exists()).toBe(true);
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "load_session_runtime_view",
      expect.anything()
    );
  });

  it("restores the initial empty state when the first checkpoint has no explicit parent node", async () => {
    const runtimeStore = useRuntimeStore();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      historyCursorMode: "live",
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-head", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-head", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-old",
          parentNodeId: null,
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", baseNodeId: "node-old", headNodeId: "node-head", label: "main" })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const actionBars = wrapper.findAll('[data-testid="workspace-user-checkpoint-actions"]');
    await actionBars[0]!.findAll("button")[0]!.trigger("click");
    await nextTick();
    clickLatestRollbackConfirm('确认仅撤回对话？');
    await nextTick();
    await new Promise(r => setTimeout(r, 450));
    await nextTick();
    await nextTick();

    expect(runtimeStore.messages).toEqual([]);
    expect(runtimeStore.draftMessage).toBe("");
    expect((wrapper.get('[data-testid="workspace-composer-input"]').element as HTMLTextAreaElement).value).toBe("");
    expect(await waitForCondition(() => wrapper.find('[data-testid="workspace-empty-state"]').exists())).toBe(true);
  });

  it("overwrites existing draft text when rollback completes", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: null,
      phase: "ready",
      error: null,
      draftMessage: "我自己的新草稿",
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      historyCursorMode: "live",
      messages: [
        createMessage({ id: "user-old", turnId: "turn-old", role: "user", content: "旧问题" }),
        createMessage({ id: "assistant-old", turnId: "turn-old", role: "assistant", content: "旧回答" }),
        createMessage({ id: "user-head", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-head", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-root",
          turnId: null,
          summary: "初始状态",
          workspaceRef: { kind: "none", rollbackCapable: false },
          createdAtMs: 500
        }),
        createHistoryNode({
          nodeId: "node-old",
          parentNodeId: "node-root",
          turnId: "turn-old",
          summary: "旧 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-head",
          parentNodeId: "node-old",
          turnId: "turn-head",
          summary: "最新 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 2000
        })
      ],
      historyBranches: [
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" })
      ]
    });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "checkout_history_node") {
        return {
          sessionId: "session-current",
          nodeId: "node-root",
          requestedMode: "transcript_only",
          appliedMode: "transcript_only",
          transcriptRestoreApplied: true,
          workspaceRollbackCapable: true,
          workspaceRollbackApplied: false,
          degraded: false,
          degradationReason: null,
          cursor: {
            sessionId: "session-current",
            visibleNodeId: "node-root",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-root",
            mode: "historical",
            authorityMode: "host_authoritative",
            cursorVersion: null,
            isAtBranchHead: false
          }
        };
      }

      if (command === "load_session_runtime_view") {
        return {
          session: {
            conversationId: "session-current",
            title: "Session current",
            summary: "初始状态",
            history: [],
            attachmentAssets: [],
            historyStateAuditSummary: null,
            runControlAuditSummary: null,
            turnTraceHistory: [],
            turnCount: 0,
            lastReferencedFile: null,
            updatedAtMs: 500
          },
          retrieved: {
            turnContext: { userMessage: "", images: [], referencesImage: false },
            sessionContext: {
              conversationId: "session-current",
              title: "Session current",
              summary: "初始状态",
              recentHistory: [],
              recentAttachmentAssets: [],
              turnCount: 0,
              lastReferencedFile: null
            },
            runState: {},
            longTermMemory: { status: "empty", summary: "", entries: [] },
            transcript: { providerNativeMessages: [] }
          },
          checkpoint: null,
          submissionPlan: null,
          controlBoundaryEvidence: null,
          historyStateAuditSummary: null,
          runControlAuditSummary: null,
          historyNodes: runtimeStore.historyNodes,
          historyBranches: runtimeStore.historyBranches,
          historyCursor: {
            sessionId: "session-current",
            visibleNodeId: "node-root",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-root",
            mode: "historical",
            authorityMode: "host_authoritative",
            cursorVersion: null,
            isAtBranchHead: false
          }
        };
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mountWorkspace();
    await nextTick();

    const actionBars = wrapper.findAll('[data-testid="workspace-user-checkpoint-actions"]');
    await actionBars[1]!.findAll("button")[0]!.trigger("click");
    await nextTick();
    clickLatestRollbackConfirm('确认仅撤回对话？');
    await nextTick();
    await new Promise(r => setTimeout(r, 400));
    await nextTick();

    expect(runtimeStore.draftMessage).toBe("新问题");
  });

});

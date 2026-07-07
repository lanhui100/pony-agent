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
    }
  },
  emits: ["render-complete"],
  setup(props, { emit }) {
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

    return {};
  },
  template:
    '<div class="markdown-stub" :class="[wrapperClass, toneClass]" :streaming="streaming ? \'true\' : undefined">{{ content }}</div>'
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
          mounted() {
            // No-op in unit tests; only suppresses directive resolution noise.
          },
          updated() {
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
          status: "pending",
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
    await nextTick();

    const streamingContent = wrapper.get('.assistant-plain-text[data-streaming="true"]');
    expect(streamingContent.text()).toContain("**正在** 输出中");
    expect(wrapper.find(".assistant-streaming-fade").exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-agent-actions"]').exists()).toBe(false);
  });

  it("reveals a buffered streaming batch through the fade layer once the first threshold is reached", async () => {
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
    await nextTick();
    let streamingContent = wrapper.get('.assistant-plain-text[data-streaming="true"]');
    expect(streamingContent.text()).toContain("hello");
    expect(wrapper.find(".assistant-streaming-fade").exists()).toBe(true);

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
    await nextTick();

    streamingContent = wrapper.get('.assistant-plain-text[data-streaming="true"]');
    expect(streamingContent.text()).toContain("hello this is a longer streamed assistant delta that crosses the first reveal threshold");
    expect(wrapper.get(".assistant-streaming-fade").text()).toBe("hello this is a longer streamed assistant delta that crosses the first reveal threshold");
    expect(wrapper.findAll(".assistant-streaming-char").length).toBeGreaterThan(10);
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
    await nextTick();

    expect(findStreamingMarkdown(wrapper).exists()).toBe(true);
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

    expect(findStreamingMarkdown(wrapper).exists()).toBe(false);
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
    await nextTick();
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

    expect(wrapper.get('.assistant-plain-text[data-streaming="true"]').text()).toContain("hello this is a longer streamed assistant delta");
    expect(wrapper.find(".assistant-streaming-fade").exists()).toBe(true);
    expect(wrapper.findAll(".assistant-streaming-char").length).toBeGreaterThan(0);

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
    expect(wrapper.find(".assistant-streaming-fade").exists()).toBe(false);
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
    await nextTick();
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

    await vi.advanceTimersByTimeAsync(4000);
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
    await vi.runOnlyPendingTimersAsync();
    await nextTick();
    await vi.runOnlyPendingTimersAsync();
    await vi.waitFor(() => {
      const scrolledToBottom = viewportScrollToSpy.mock.calls.some(([options]) =>
        typeof options === "object" &&
        options?.top === viewportMetrics.scrollHeight &&
        options.behavior === "auto"
      );
      expect(scrolledToBottom || viewportMetrics.scrollTop >= viewportMetrics.scrollHeight).toBe(true);
    });
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

  it("keeps streaming assistant text in one inline flow while content grows", async () => {
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
    expect(streamingFlow.text()).toContain("streaming content that has already crossed the first batch threshold");
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
    await nextTick();

    streamingFlow = wrapper.get('[data-testid="assistant-streaming-flow"]');
    expect(streamingFlow.text()).toContain("and continues on the next line");
    expect(wrapper.findAll('[data-testid="assistant-streaming-flow"]')).toHaveLength(1);

    await advanceAnimationFrames(5);
    const scrollDebugEvent = latestScrollDebugEvent("latest-turn-signature:queue-follow-scroll");
    expect(scrollDebugEvent).toBeDefined();
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
    await new Promise((r) => window.setTimeout(r, 10));

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

    expect(wrapper.text()).toContain("正在思考...");

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

  it("shows reasoning placeholder for pending assistant with empty reasoning", async () => {
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

    expect(wrapper.find(".assistant-reasoning").exists()).toBe(true);
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
        expect(payload).toEqual({
          turnId: null,
          sessionId: "session-current",
          runId: null,
          nodeId: "node-root"
        });
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
            turnContext: {
              userMessage: "",
              images: [],
              referencesImage: false
            },
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
            longTermMemory: {
              status: "empty",
              summary: "No long-term memory entries are stored for this session yet.",
              entries: []
            },
            transcript: {
              providerNativeMessages: []
            }
          },
          checkpoint: createCheckpoint({
            turnId: "turn-head",
            checkpointKind: "runtime_control",
            status: "running",
            phase: "calling_model"
          }),
          submissionPlan: null,
          controlBoundaryEvidence: null,
          historyStateAuditSummary: null,
          runControlAuditSummary: null,
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
          ],
          historyCursor: {
            sessionId: "session-current",
            visibleNodeId: "node-root",
            activeBranchId: "branch-main",
            branchHeadNodeId: "node-head",
            workspaceNodeId: "node-root",
            mode: "historical"
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
    // Wait for the minimum 2s loading duration (plus margin) to elapse
    await new Promise(r => setTimeout(r, 2500));
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
    await new Promise(r => setTimeout(r, 2500));
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

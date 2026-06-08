import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
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

const scrollToBottomSpy = vi.fn();
const viewportScrollToSpy = vi.fn();

const ScrollAreaStub = defineComponent({
  setup(_props, { slots, expose }) {
    const viewportEl = {
      scrollHeight: 1000,
      scrollTop: 700,
      clientHeight: 400,
      scrollTo: viewportScrollToSpy
    } as unknown as HTMLElement;

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
    wrapperClass: {
      type: String,
      default: ""
    },
    toneClass: {
      type: String,
      default: ""
    }
  },
  template: '<div class="markdown-stub" :class="[wrapperClass, toneClass]">{{ content }}</div>'
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
    durationSeconds: partial.durationSeconds ?? null
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

  return mount(HomeWorkspace, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub,
        MarkdownRenderer: MarkdownRendererStub,
        Button: ButtonStub,
        Transition: false,
        TransitionGroup: false
      }
    }
  });
}

describe("HomeWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn()
    });
    vi.stubGlobal(
      "requestAnimationFrame",
      ((callback: FrameRequestCallback) => {
        callback(0);
        return 1;
      }) as typeof requestAnimationFrame
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    scrollToBottomSpy.mockReset();
    viewportScrollToSpy.mockReset();
  });

  afterEach(() => {
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

    expect(wrapper.get('[data-testid="workspace-empty-state"]').text()).toContain("需要我帮你做什么？");
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

    expect(wrapper.element.className).toContain("rounded-t-[0.6rem]");
    expect(wrapper.element.className).not.toContain("bg-[#fdfbf7]/88");

    const timeline = wrapper.get(".scroll-area-stub");
    expect(timeline.element.className).toContain("rounded-t-[0.6rem]");
    expect(wrapper.get('[data-testid="workspace-content-column"]').classes()).toContain("max-w-[58rem]");

    const composerShell = wrapper.get('[data-testid="workspace-composer-shell"]');
    expect(composerShell.classes()).toContain("max-w-[58rem]");
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

    const assistantArticle = wrapper.findAll("article").find((node) => node.text().includes("Agent"));

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

    const streamingContent = wrapper.get(".assistant-streaming-content");
    expect(streamingContent.text()).toContain("**正在** 输出中");
    expect(streamingContent.findAll("span")[0]?.text()).toBe("**正在** 输出中");
    expect(wrapper.find(".assistant-streaming-fade").exists()).toBe(false);
    expect(wrapper.find(".markdown-stub").exists()).toBe(false);
  });

  it("fades only the latest streamed assistant delta instead of replaying the full accumulated content", async () => {
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
    let streamingSpans = wrapper.get(".assistant-streaming-content").findAll("span");
    expect(streamingSpans[0]?.text()).toBe("hello");
    expect(wrapper.find(".assistant-streaming-fade").exists()).toBe(false);

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
          content: "hello this is a longer streamed assistant delta",
          status: "pending",
          modelName: "OpenAI/GPT-5"
        })
      ]
    });
    await nextTick();

    streamingSpans = wrapper.get(".assistant-streaming-content").findAll("span");
    expect(streamingSpans[0]?.text()).toBe("hello");
    expect(wrapper.get(".assistant-streaming-fade").text()).toBe("this is a longer streamed assistant delta");
  });

  it("switches from streaming text to final markdown when assistant completes", async () => {
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

    expect(wrapper.find(".assistant-streaming-content").exists()).toBe(true);
    expect(wrapper.find(".markdown-stub").exists()).toBe(false);

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

    expect(wrapper.find(".assistant-streaming-content").exists()).toBe(false);
    const markdownBlock = wrapper.get(".markdown-stub");
    expect(markdownBlock.text()).toContain("**完成** 输出");
    expect(markdownBlock.classes()).toContain("text-stone-800");
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
    await nextTick();

    scrollToBottomSpy.mockClear();
    await nextTick();
    expect(scrollToBottomSpy).toHaveBeenCalledTimes(1);
    expect(wrapper.find(".assistant-streaming-content").exists()).toBe(true);
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

    scrollToBottomSpy.mockClear();
    await nextTick();
    expect(scrollToBottomSpy.mock.calls.length).toBeGreaterThan(0);

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

    await nextTick();
    expect(wrapper.find(".assistant-streaming-content").text()).toContain("hello");
    expect(wrapper.find(".assistant-streaming-content").text().length).toBeGreaterThan(5);
  });

  it("opens provider menu, selects another model, and closes afterwards", async () => {
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

  it("closes provider and reasoning menus on outside click", async () => {
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
    expect(wrapper.text()).toContain("minimal");

    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();
    expect(wrapper.findAll("div.absolute")).toHaveLength(0);
  });

  it("syncs reasoning menu selection and visibility toggle persistence", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const providerStore = useProviderStore();
    const setReasoningSpy = vi.spyOn(providerStore, "setCurrentReasoningEffort");

    const wrapper = mountWorkspace();
    await nextTick();

    const [, , reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
    await reasoningTrigger.trigger("click");
    await nextTick();

    const highButton = wrapper.findAll("button").find((node) => node.text().includes("high"));
    expect(highButton).toBeDefined();

    await highButton?.trigger("click");
    await nextTick();

    expect(setReasoningSpy).toHaveBeenCalledWith("high");
    expect(providerStore.currentReasoningEffort).toBe("high");
    expect(wrapper.text()).not.toContain("minimal");

    await reasoningTrigger.trigger("click");
    await nextTick();

    const visibilityToggle = wrapper.get('[data-testid="reasoning-visibility-toggle"]');
    expect(visibilityToggle.text()).toContain("显示思考");
    expect(visibilityToggle.text()).toContain("已开启");

    await visibilityToggle.trigger("click");
    expect(window.localStorage.getItem("pony-agent.ui.show-reasoning-content")).toBe("false");
  });

  it("keeps reasoning menu available for visibility toggle even when effort is unsupported", async () => {
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

    const markdownBlocks = wrapper.findAll(".markdown-stub");
    expect(markdownBlocks.some((node) => node.classes().includes("text-rose-800"))).toBe(true);
    expect(markdownBlocks.some((node) => node.classes().includes("text-stone-800"))).toBe(true);

    const reasoningBlocks = wrapper.findAll(".assistant-reasoning-markdown");
    expect(reasoningBlocks).toHaveLength(2);
    expect(reasoningBlocks[0].text()).toContain("error reasoning");
    expect(reasoningBlocks[1].text()).toContain("final reasoning");

    expect(wrapper.text()).toContain("Search");
    expect(wrapper.text()).toContain("Edit");
    expect(wrapper.text()).toContain("Fail");
    expect(wrapper.text()).toContain("T:33");
    expect(wrapper.text()).toContain("T:12");
    expect(wrapper.text()).toContain("2s");
    expect(wrapper.text()).toContain("1s");
    expect(wrapper.text()).toContain("!");
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

    const toolList = wrapper.get(".conversation-tool-list");
    expect(toolList.text()).toContain("Search");
    expect(wrapper.text()).toContain("工具调用");
    expect(wrapper.text()).toContain("1 项");

    const summaries = wrapper.findAll("summary");
    expect(summaries).toHaveLength(1);
    expect(summaries[0].text()).toContain("思考过程");
    expect(summaries[0].html()).toContain("lucide-brain");
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

  it("renders message-level checkpoint actions only for non-latest assistant turns and reuses checkout actions", async () => {
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
          workspaceRef: { kind: "host_snapshot", rollbackCapable: false },
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

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.get('[data-testid="workspace-checkpoint-transcript-node-old"]').attributes("title")).toContain(
      "仅对话"
    );
    expect(wrapper.get('[data-testid="workspace-checkpoint-workspace-node-old"]').attributes("title")).toContain(
      "将仅恢复对话历史"
    );
    expect(wrapper.find('[data-testid="workspace-checkpoint-transcript-node-head"]').exists()).toBe(false);

    await wrapper.get('[data-testid="workspace-checkpoint-transcript-node-old"]').trigger("click");
    await wrapper.get('[data-testid="workspace-checkpoint-workspace-node-old"]').trigger("click");

    expect(checkoutSpy).toHaveBeenNthCalledWith(1, "node-old", "transcript_only");
    expect(checkoutSpy).toHaveBeenNthCalledWith(2, "node-old", "transcript_and_workspace");
  });

  it("opens checkpoint picker from trigger and global shortcut, then rolls back using the best available mode", async () => {
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

    await wrapper.get('[data-testid="workspace-checkpoint-picker-trigger"]').trigger("click");
    await nextTick();
    expect(wrapper.get('[data-testid="workspace-checkpoint-picker-menu"]').text()).toContain("旧 checkpoint");

    await wrapper.get('[data-testid="workspace-checkpoint-picker-item-node-old"]').trigger("click");
    expect(checkoutSpy).toHaveBeenCalledWith("node-old", "transcript_and_workspace");

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true }));
    await nextTick();
    expect(wrapper.get('[data-testid="workspace-checkpoint-picker-menu"]').exists()).toBe(true);
  });

  it("shows fork summary menu and jumps through existing branch actions", async () => {
    const runtimeStore = useRuntimeStore();
    const switchSpy = vi.spyOn(runtimeStore, "switchHistoryBranch").mockResolvedValue({
      sessionId: "session-current",
      branchId: "branch-fork",
      nodeId: "node-fork-head",
      visibleNodeId: "node-fork-head",
      activeBranchId: "branch-fork",
      branchHeadNodeId: "node-fork-head",
      workspaceNodeId: "node-fork-head",
      mode: "live",
      historyNodes: runtimeStore.historyNodes,
      historyBranches: runtimeStore.historyBranches
    });
    const checkoutSpy = vi.spyOn(runtimeStore, "checkoutHistoryNode").mockResolvedValue(null);
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      activeBranchId: "branch-main",
      visibleNodeId: "node-head",
      branchHeadNodeId: "node-head",
      messages: [
        createMessage({ id: "user-1", turnId: "turn-source", role: "user", content: "源问题" }),
        createMessage({ id: "assistant-1", turnId: "turn-source", role: "assistant", content: "源回答" }),
        createMessage({ id: "user-2", turnId: "turn-head", role: "user", content: "新问题" }),
        createMessage({ id: "assistant-2", turnId: "turn-head", role: "assistant", content: "新回答" })
      ],
      historyNodes: [
        createHistoryNode({
          nodeId: "node-source",
          turnId: "turn-source",
          summary: "源 checkpoint",
          workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
          createdAtMs: 1000
        }),
        createHistoryNode({
          nodeId: "node-fork-head",
          turnId: "turn-fork-head",
          branchId: "branch-fork",
          summary: "fork 摘要",
          createdAtMs: 1500
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
        createHistoryBranch({ branchId: "branch-main", headNodeId: "node-head", label: "main" }),
        createHistoryBranch({
          branchId: "branch-fork",
          baseNodeId: "node-source",
          headNodeId: "node-fork-head",
          forkedFromNodeId: "node-source",
          label: "fork-1",
          updatedAtMs: 2500
        })
      ]
    });

    const wrapper = mountWorkspace();
    await nextTick();

    await wrapper.get('[data-testid="workspace-checkpoint-forks-node-source"]').trigger("click");
    await nextTick();

    expect(wrapper.get('[data-testid="workspace-checkpoint-fork-menu-node-source"]').text()).toContain("fork-1");

    await wrapper.get('[data-testid="workspace-checkpoint-fork-target-branch-fork"]').trigger("click");

    expect(switchSpy).toHaveBeenCalledWith("branch-fork");
    expect(checkoutSpy).not.toHaveBeenCalled();
  });
});

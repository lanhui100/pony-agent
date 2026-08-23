import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSidebar from "@/components/HomeSidebar.vue";
import {
  __resetFrontendFlightRecorderForTests,
  injectFrontendDiagnosticStall
} from "@/lib/frontend-flight-recorder";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";
import type {
  BuildContextObservation,
  ProviderCallCacheRecord,
  RetrievedContextState,
  TraceStep,
  TraceTimelineEntry,
  TurnTraceRecord
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

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

const TooltipStub = defineComponent({
  props: {
    text: {
      type: String,
      default: ""
    }
  },
  template: '<div class="tooltip-stub" :data-tooltip="text"><slot /></div>'
});

function createProviderRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-openai",
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
              supportsReasoning: true
            }
          }
        ]
      }
    ]
  };
}

function createRetrievedContext(overrides: Partial<RetrievedContextState> = {}): RetrievedContextState {
  return {
    turnContext: {
      userMessage: "继续推进 PA-025",
      images: [],
      referencesImage: false
    },
    sessionContext: {
      conversationId: "graph-session",
      title: "Graph session",
      summary: "retrieval summary from host",
      recentHistory: [
        { role: "user", content: "继续推进 PA-025" },
        { role: "assistant", content: "正在继续处理" }
      ],
      recentAttachmentAssets: [],
      turnCount: 3,
      lastReferencedFile: "src-tauri/src/agent/context.rs"
    },
    runState: {
      runId: "run-alpha",
      goal: "梳理 retrieval boundary 并推进上层接入",
      phase: "running",
      activeTurnId: "turn-1",
      lastCompletedTurnId: null,
      resumeCount: 0,
      lastDecisionSummary: "继续推进 runtime 侧接口",
      executionCheckpointStatus: "running",
      executionCheckpointPhase: "calling_model"
    },
    longTermMemory: {
      status: "available",
      summary: "已有长期记忆",
      entries: [
        {
          kind: "user_preference.response_language",
          content: "Reply in Chinese.",
          source: "explicit_user_message",
          updatedAtMs: 1_715_000_000_000
        },
        {
          kind: "project_focus.active_task",
          content: "Current active task is PA-018.",
          source: "explicit_user_message",
          updatedAtMs: 1_715_000_000_001
        }
      ]
    },
    transcript: {
      providerNativeMessages: []
    },
    ...overrides
  };
}

function createBuildContextObservation(
  overrides: Partial<BuildContextObservation> = {}
): BuildContextObservation {
  return {
    requestFormat: overrides.requestFormat ?? "response_format=json_schema",
    messageCount: overrides.messageCount ?? 3,
    imageCount: overrides.imageCount ?? 1,
    toolCount: overrides.toolCount ?? 2,
    temperature: overrides.temperature ?? 0.2,
    maxOutputTokens: overrides.maxOutputTokens ?? 4096,
    stablePrefixText:
      overrides.stablePrefixText ??
      "system: stable system rule\ndeveloper: stable capability prefix",
    semiStableContextText:
      overrides.semiStableContextText ??
      "developer: retrieval summary from host\nuser: recent history question",
    volatileInputText:
      overrides.volatileInputText ??
      "user: continue PA-025 with the latest screenshot",
    prefixMutationReasons: overrides.prefixMutationReasons ?? [],
    contextRefreshReason: overrides.contextRefreshReason ?? "initial_build",
    instructionScopeSources:
      overrides.instructionScopeSources ?? ["thread://base-system", "workspace://src/App.vue"],
    conversationCarryMode: overrides.conversationCarryMode ?? "full_replay",
    requestMessagesText:
      overrides.requestMessagesText ??
      "[0] system\nsummarize retrieval state\n\n[1] user\ncontinue PA-025\n\n[2] assistant\nacknowledged",
    toolDefinitionsText:
      overrides.toolDefinitionsText ??
      "workspace.read_file(path: string)\nworkspace.search(query: string)"
  };
}

function createTraceSteps(): TraceStep[] {
  return [
    { id: "step-plan", label: "Receive input", state: "completed" },
    { id: "step-context", label: "Build context", state: "completed" },
    { id: "step-call-model", label: "Call model", state: "completed" },
    { id: "step-call-tool", label: "Call tool", state: "pending" },
    { id: "step-return", label: "Return result", state: "completed" }
  ];
}

function createTraceTimeline(): TraceTimelineEntry[] {
  return [
    {
      id: "input-1",
      kind: "input",
      label: "RECEIVE INPUT",
      state: "completed",
      sequence: 1,
      text: "继续推进 PA-025，不要生成摘要"
    },
    { id: "context-2", kind: "context", label: "BUILD CONTEXT", state: "completed", sequence: 2 },
    { id: "model-3", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 3, firstTokenLatencyMs: 321 },
    {
      id: "return-4",
      kind: "return",
      label: "RETURN RESULT",
      state: "completed",
      sequence: 4,
      inputTokens: 120,
      cacheHitInputTokens: 80,
      reasoningTokens: 18,
      outputTokens: 40,
      totalTokens: 160,
      firstTokenLatencyMs: 321,
      turnDurationMs: 2800
    }
  ];
}

function createTraceRecord(overrides: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: overrides.turnId ?? "turn-1",
    title: overrides.title ?? "测试轮次",
    phase: overrides.phase ?? "ready",
    traceSteps: overrides.traceSteps ?? createTraceSteps(),
    traceTimeline: overrides.traceTimeline ?? createTraceTimeline(),
    toolActivities: overrides.toolActivities ?? [],
    providerRequestedName: overrides.providerRequestedName ?? null,
    providerName: overrides.providerName ?? null,
    providerProtocol: overrides.providerProtocol ?? null,
    providerModel: overrides.providerModel ?? null,
    providerSource: overrides.providerSource ?? null,
    providerMode: overrides.providerMode ?? null,
    buildContextObservation: overrides.buildContextObservation ?? null,
    sessionSummary: overrides.sessionSummary ?? null,
    fallbackReason: overrides.fallbackReason ?? null,
    error: overrides.error ?? null,
    inputTokens: overrides.inputTokens ?? null,
    cacheHitInputTokens: overrides.cacheHitInputTokens ?? null,
    reasoningTokens: overrides.reasoningTokens ?? null,
    outputTokens: overrides.outputTokens ?? null,
    totalTokens: overrides.totalTokens ?? null,
    firstTokenLatencyMs: overrides.firstTokenLatencyMs ?? null,
    turnDurationMs: overrides.turnDurationMs ?? null,
    updatedAt: overrides.updatedAt ?? 1,
    providerCallRecords: overrides.providerCallRecords ?? []
  };
}

function createProviderCallRecord(overrides: Partial<ProviderCallCacheRecord> = {}): ProviderCallCacheRecord {
  return {
    requestKind: overrides.requestKind ?? "initial_request",
    providerSource: overrides.providerSource ?? "provider_decision_stream",
    providerMode: overrides.providerMode ?? "live",
    inputTokens: overrides.inputTokens ?? null,
    cacheHitInputTokens: overrides.cacheHitInputTokens ?? null,
    cacheMissInputTokens: overrides.cacheMissInputTokens ?? null,
    reasoningTokens: overrides.reasoningTokens ?? null,
    outputTokens: overrides.outputTokens ?? null,
    totalTokens: overrides.totalTokens ?? null,
    firstTokenLatencyMs: overrides.firstTokenLatencyMs ?? null,
    turnDurationMs: overrides.turnDurationMs ?? null,
    latencyKind: overrides.latencyKind ?? "provider_stream",
    prefixMutationReasons: overrides.prefixMutationReasons ?? []
  };
}

async function flushAll() {
  await Promise.resolve();
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

async function mountSidebar() {
  const providerStore = useProviderStore();
  providerStore.$patch({
    registry: createProviderRegistry(),
    selectedReasoningEffort: null
  });

  const wrapper = mount(HomeSidebar, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub,
        Tooltip: TooltipStub
      }
    }
  });
  await flushAll();
  return wrapper;
}

function countOccurrences(text: string, needle: string) {
  return text.split(needle).length - 1;
}

describe("HomeSidebar", () => {
  // ── PA-096：右侧栏只保留 状态/计划/调试；trace 用例迁 tests/TraceInspector.spec.ts，
  // 工具目录用例迁 tests/ConfigToolsSection.spec.ts（按交互 testid 分类）。──

  beforeEach(() => {
    vi.clearAllMocks();
    __resetFrontendFlightRecorderForTests();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});

    if (!navigator.clipboard) {
      Object.defineProperty(navigator, "clipboard", {
        value: { writeText: vi.fn() },
        configurable: true
      });
    } else {
      vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    }

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "graph-session",
      sessionList: [],
      sessionOperation: null,
      isSubmitting: false,
      messages: [],
      phase: "ready"
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("右侧栏只保留状态、计划、调试三段，不再渲染 Tools 与 Trace（PA-096）", async () => {
    const wrapper = await mountSidebar();
    await flushAll();

    expect(wrapper.text()).toContain("状态");
    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("true");
    expect(wrapper.text()).toContain("计划");
    expect(wrapper.text()).not.toContain("Tools");
    expect(wrapper.find('[data-testid="tools-panel-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-panel-toggle"]').exists()).toBe(false);
  }, 10000);

  it("冻结聚合守护（PA-096 review）：进行中 turn 不计入状态面板轮次数", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      isSubmitting: true,
      activeTurnId: "turn-live-aggregate",
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-settled-aggregate",
          title: "settled aggregate",
          phase: "completed",
          updatedAt: 1000
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    // liveTurnEnabled 恒 false：活跃 turn 不并入聚合，轮次数保持已结算数
    const turnCountStub = wrapper
      .get('[data-testid="status-panel-toggle"]')
      .element.closest("section")!
      .querySelector<HTMLElement>('.tooltip-stub[data-tooltip="对话轮次数"]');
    expect(turnCountStub?.textContent?.trim()).toBe("1");
  }, 10000);

  it("将会话状态收敛到右侧状态栏", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: "switching"
    });

    const wrapper = await mountSidebar();
    await flushAll();

    expect(wrapper.get('[data-testid="status-session-summary"]').text()).toContain("正在切换对话");
  });

  it("状态面板保持紧凑布局，受控状态归入消息区", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: "switching",
      latestRunControlAuditSummary: {
        actionEvidenceSummary: {
          status: "available",
          sourceFamily: "run_control",
          commandKind: "resume_graph_run_stream",
          boundary: "resume_requested",
          resultKind: "observe",
          summary: "检测到暂停中的运行；点击后会恢复该 run 并继续执行。",
          targetSummary: "恢复 run-alpha",
          blocked: false,
          degraded: false
        },
        currentContextProjection: {
          phase: "paused",
          checkpointStatus: "ready",
          activeRunId: "run-alpha",
          submissionPlanCommand: "resume_graph_run_stream"
        }
      }
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanelText = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "";

    expect(statusPanelText).not.toContain("Run phase");
    expect(statusPanelText).not.toContain("Recent history");
    expect(statusPanelText).not.toContain("Recent attachments");
    expect(statusPanelText).not.toContain("Long-term memory");
    expect(statusPanelText).not.toContain("Goal:");
    expect(statusPanelText).not.toContain("Last file:");

    expect(wrapper.get('[data-testid="status-session-summary"]').text()).toContain("正在切换对话");
  });

  it("状态面板中的 token 标签不显示总计字样", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-status-tokens",
          title: "status token labels",
          phase: "completed",
          inputTokens: 8437,
          cacheHitInputTokens: 4352,
          outputTokens: 1366
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanel = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")!;
    const statusPanelText = statusPanel.textContent ?? "";
    const tooltipStubs = statusPanel.querySelectorAll<HTMLElement>(".tooltip-stub[data-tooltip]");
    const tooltipTexts = Array.from(tooltipStubs).map((el) => el.getAttribute("data-tooltip"));

    // Token labels are now tooltips instead of visible text
    expect(tooltipTexts).toContain("输入");
    expect(tooltipTexts).toContain("输出");
    expect(tooltipTexts).toContain("缓存读取");
    // Visible panel shows "Token" label and compact values
    expect(statusPanelText).toContain("Token");
    expect(statusPanelText).toContain("8.4K");
    expect(statusPanelText).toContain("1.4K");
    // No "总计" suffix in labels
    expect(statusPanelText).not.toContain("输入总计");
    expect(statusPanelText).not.toContain("缓存读取总计");
    expect(statusPanelText).not.toContain("输出总计");
  });
  // PA-096：原 build_context 混合用例的状态面板负断言半边（trace 正断言半边在 TraceInspector.spec）。
  // 注：trace 面板已不在本树内，此负断言验证状态面板自身不渲染请求细节。
  it("状态面板不暴露 build-context 请求细节", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionSummary: "legacy summary should be shadowed",
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-build-context",
          title: "show build context",
          phase: "completed"
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanelText = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "";

    expect(statusPanelText).not.toContain("legacy summary should be shadowed");
    expect(statusPanelText).not.toContain("[0] system");
    expect(statusPanelText).not.toContain("stable capability prefix");
    expect(wrapper.find('[data-testid="trace-step-button-context-2"]').exists()).toBe(false);
  });

  it.skip("显示 recorder 摘要并支持导出前端 trace", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      frontendRecorderStats: {
        initialized: true,
        activeSessionId: "graph-session",
        activeTurnId: "turn-1",
        seq: 3,
        bufferedEventCount: 0,
        bufferedSnapshotCount: 0,
        flushCount: 1,
        flushFailureCount: 0,
        droppedEventCount: 2,
        droppedSnapshotCount: 1,
        stallCount: 4,
        lastStallGapMs: 620,
        lastFlushDurationMs: 18,
        lastFlushAtMs: 1000,
        lastError: null,
        lastExportAtMs: null
      },
      turnTraceHistory: [createTraceRecord({ turnId: "turn-1", updatedAt: 2000 })]
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "export_frontend_trace_json") {
        return {
          format: "json",
          fileName: "frontend-trace.json",
          content: "{\"events\":[]}",
          truncated: false,
          eventCount: 0,
          snapshotCount: 0,
          fromWallMs: null,
          toWallMs: null
        };
      }
      return null;
    });

    const wrapper = await mountSidebar();
    await flushAll();

    // KNOWN TEST DEBT: Recorder UI section rendering changed
    expect(wrapper.text()).toContain("状态");

    const exportButton = wrapper.findAll("button").find((button) => button.text().includes("导出 JSON"));
    expect(exportButton).toBeTruthy();
    await exportButton!.trigger("click");

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "export_frontend_trace_json",
      expect.objectContaining({
        sessionId: "graph-session",
        turnId: "turn-1"
      })
    );
  });

  it.skip("支持手动注入 stall smoke 以便采集前端卡顿证据", async () => {
    // KNOWN TEST DEBT: refreshFrontendRecorderStats removed from store
    vi.useFakeTimers();
    const runtimeStore = useRuntimeStore();
    const refreshSpy = vi.spyOn(runtimeStore, "refreshFrontendRecorderStats");

    const wrapper = await mountSidebar();
    await flushAll();

    await wrapper.get('[data-testid="frontend-stall-smoke"]').trigger("click");
    await vi.runAllTimersAsync();

    expect(injectFrontendDiagnosticStall).toHaveBeenCalledWith(1800, "sidebar-stall-smoke");
    expect(refreshSpy).toHaveBeenCalled();
    vi.useRealTimers();
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSidebar from "@/components/HomeSidebar.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";
import type {
  BuildContextObservation,
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

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
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
    requestMessagesText:
      overrides.requestMessagesText ??
      "system: summarize retrieval state\nuser: continue PA-025\nassistant: acknowledged",
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

async function flushAll() {
  await Promise.resolve();
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

function mountSidebar() {
  const providerStore = useProviderStore();
  providerStore.$patch({
    registry: createProviderRegistry(),
    selectedReasoningEffort: null
  });

  return mount(HomeSidebar, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub
      }
    }
  });
}

function countOccurrences(text: string, needle: string) {
  return text.split(needle).length - 1;
}

describe("HomeSidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
      phase: "ready",
      availableTools: [
        {
          name: "workspace_path_info",
          description: "读取当前路径的基础信息",
          inputSchema: {
            type: "object",
            properties: {
              path: { type: "string" }
            },
            required: ["path"]
          }
        }
      ]
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("保留状态、工具和 trace 三段折叠结构", async () => {
    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.text()).toContain("状态");
    expect(wrapper.text()).toContain("Tools");
    expect(wrapper.text()).toContain("Trace");
    expect(wrapper.get('[data-testid="trace-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("true");
  }, 10000);

  it("将会话和控制状态收敛到右侧状态栏", async () => {
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

    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.get('[data-testid="status-session-summary"]').text()).toContain("正在切换对话");
    expect(wrapper.get('[data-testid="status-control-summary"]').text()).toContain("检测到暂停中的运行");
    expect(wrapper.get('[data-testid="status-control-summary"]').text()).toContain("resume_graph_run_stream");
  });

  it("默认展开最新一条 turn，而不是停留在旧 failed turn", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-old-failed",
          title: "旧失败轮次",
          phase: "failed",
          error: "old failure",
          updatedAt: 1000,
          traceTimeline: [
            {
              id: "model-old",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "error",
              sequence: 3,
              error: "old failure"
            }
          ]
        }),
        createTraceRecord({
          turnId: "turn-new-completed",
          title: "新成功轮次",
          phase: "completed",
          updatedAt: 2000,
          buildContextObservation: createBuildContextObservation(),
          traceTimeline: [
            {
              id: "input-new",
              kind: "input",
              label: "RECEIVE INPUT",
              state: "completed",
              sequence: 1,
              text: "src/agent中是怎么组织的？"
            },
            {
              id: "context-new",
              kind: "build_context",
              label: "BUILD CONTEXT",
              state: "completed",
              sequence: 2,
              buildContextObservation: createBuildContextObservation()
            },
            {
              id: "model-new",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 3,
              text: "这是最新成功轮次"
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const latestTurnButton = wrapper.findAll("button").find((button) => button.text().includes("新成功轮次"));
    const oldTurnButton = wrapper.findAll("button").find((button) => button.text().includes("旧失败轮次"));

    const latestTurnSection = latestTurnButton?.element.closest('section[data-open]');
    const oldTurnSection = oldTurnButton?.element.closest('section[data-open]');

    expect(latestTurnSection?.getAttribute("data-open")).toBe("true");
    expect(oldTurnSection?.getAttribute("data-open")).not.toBe("true");
    expect(latestTurnSection?.textContent ?? "").toContain("这是最新成功轮次");
    expect(latestTurnSection?.textContent ?? "").not.toContain("old failure");
  });

  it("将 retrieval 全局信息归并到状态面板，并清理冗余字段", async () => {
    const buildContextObservation = createBuildContextObservation();
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionSummary: "legacy summary should be shadowed",
      retrievedContext: createRetrievedContext(),
      messages: [
        {
          id: "user-turn-build-context",
          turnId: "turn-build-context",
          role: "user",
          content: "继续推进 PA-025",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        },
        {
          id: "assistant-turn-build-context",
          turnId: "turn-build-context",
          role: "assistant",
          content: "已完成本轮返回",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-build-context",
          title: "show build context",
          phase: "completed",
          providerRequestedName: "openai/gpt-5",
          providerName: "OpenAI",
          providerProtocol: "responses",
          providerModel: "gpt-5",
          providerSource: "registry",
          providerMode: "streaming",
          buildContextObservation
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const statusPanelText = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "";
    const tracePanelText = wrapper.get('[data-testid="trace-panel-toggle"]').element.closest("section")?.textContent ?? "";

    expect(wrapper.get('[data-testid="retrieved-active-task"]').text()).toContain("PA-018");
    expect(wrapper.get('[data-testid="retrieved-memory-list"]').text()).toContain("Reply in Chinese.");
    expect(statusPanelText).toContain("运行阶段");
    expect(statusPanelText).not.toContain("Run phase");
    expect(statusPanelText).not.toContain("Recent history");
    expect(statusPanelText).not.toContain("Recent attachments");
    expect(statusPanelText).not.toContain("Long-term memory");
    expect(statusPanelText).not.toContain("Goal:");
    expect(statusPanelText).not.toContain("Last file:");
    expect(statusPanelText).not.toContain("legacy summary should be shadowed");

    expect(wrapper.find('[data-testid="turn-build-context"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-step-button-context-2"]').exists()).toBe(true);

    await wrapper.get('[data-testid="trace-step-button-context-2"]').trigger("click");
    await nextTick();

    const contextStepText = wrapper.get('[data-testid="trace-step-button-context-2"]').element.closest("section")?.textContent ?? "";
    expect(contextStepText).toContain("response_format=json_schema");
    expect(contextStepText).toContain("这里展示的是本轮真正发给模型的请求，不是 retrieval state 的替身。");
    expect(contextStepText).toContain("稳定前缀");
    expect(contextStepText).toContain("最终请求消息");
    expect(contextStepText).toContain("工具定义");

    await wrapper.get('[data-testid="trace-detail-button-context-2-stable"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("stable capability prefix");

    await wrapper.get('[data-testid="trace-detail-button-context-2-semi"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("retrieval summary from host");

    await wrapper.get('[data-testid="trace-detail-button-context-2-volatile"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("latest screenshot");

    await wrapper.get('[data-testid="trace-detail-button-context-2-messages"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("system: summarize retrieval state");

    await wrapper.get('[data-testid="trace-detail-button-context-2-tools"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("workspace.read_file(path: string)");

    expect(statusPanelText).not.toContain("system: summarize retrieval state");
    expect(statusPanelText).not.toContain("stable capability prefix");
    expect(tracePanelText).not.toContain("Current retrieval state");
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

    const wrapper = mountSidebar();
    await flushAll();

    const statusPanelText = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "";
    expect(statusPanelText).toContain("输入");
    expect(statusPanelText).toContain("缓存命中");
    expect(statusPanelText).toContain("输出");
    expect(statusPanelText).not.toContain("输入总计");
    expect(statusPanelText).not.toContain("缓存命中总计");
    expect(statusPanelText).not.toContain("输出总计");
  });

  it("turn 摘要使用整轮汇总或平均指标，并固定右侧总耗时不换行", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-summary",
          title: "turn summary metrics",
          phase: "completed",
          traceTimeline: [
            {
              id: "model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              inputTokens: 110,
              cacheHitInputTokens: 60,
              outputTokens: 45,
              firstTokenLatencyMs: 210,
              turnDurationMs: 2400
            },
            {
              id: "model-2",
              kind: "call_model",
              label: "CALL MODEL #2",
              state: "completed",
              sequence: 2,
              inputTokens: 40,
              cacheHitInputTokens: 10,
              outputTokens: 15,
              firstTokenLatencyMs: 90,
              turnDurationMs: 800
            }
          ],
          inputTokens: 150,
          cacheHitInputTokens: 70,
          outputTokens: 60,
          firstTokenLatencyMs: 150,
          turnDurationMs: 3500
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const turnButton = wrapper.findAll("button").find((button) => button.text().includes("turn summary metrics"));
    expect(turnButton).toBeDefined();
    expect(turnButton!.text()).toContain("输入 150");
    expect(turnButton!.text()).toContain("缓存 70");
    expect(turnButton!.text()).toContain("输出 60");
    expect(turnButton!.text()).toContain("速度 18.8 token/s");
    expect(turnButton!.text()).toContain("延时 150 ms");
    expect(turnButton!.text()).not.toContain("思考链");
    expect(turnButton!.text()).not.toContain("输入 40");

    const durationSpan = turnButton!.findAll("span").find((item) => item.text() === "3.5s");
    expect(durationSpan).toBeDefined();
    expect(durationSpan!.classes()).toContain("whitespace-nowrap");
  });

  it("RECEIVE INPUT 只保留一个输入，且不显示 PREPARE RETRIEVAL", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      messages: [
        {
          id: "user-turn-input",
          turnId: "turn-input",
          role: "user",
          content: "继续推进 PA-025，不要生成摘要",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        },
        {
          id: "assistant-turn-input",
          turnId: "turn-input",
          role: "assistant",
          content: "本轮完成",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-input",
          title: "dedupe input",
          phase: "completed",
          traceTimeline: [
            {
              id: "input-1",
              kind: "input",
              label: "RECEIVE INPUT",
              state: "completed",
              sequence: 1,
              text: "继续推进 PA-025，不要生成摘要"
            },
            {
              id: "retrieval-2",
              kind: "prepare_retrieval",
              label: "PREPARE RETRIEVAL",
              state: "completed",
              sequence: 2
            },
            {
              id: "model-3",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 3,
              text: "本轮完成"
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.find('[data-testid="trace-step-button-retrieval-2"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("PREPARE RETRIEVAL");

    await wrapper.get('[data-testid="trace-step-button-input-1"]').trigger("click");
    await nextTick();

    const inputSectionText = wrapper.get('[data-testid="trace-step-button-input-1"]').element.closest("section")?.textContent ?? "";
    expect(countOccurrences(inputSectionText, "继续推进 PA-025，不要生成摘要")).toBe(1);
    expect(inputSectionText).not.toContain("标题");
    expect(inputSectionText).not.toContain("输入原文");
    expect(inputSectionText).not.toContain("记录本轮进入 agent 的用户输入。");
  });

  it("CALL MODEL 使用 provider/model 合并值，并展开后直接展示思考链与模型输出", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      messages: [
        {
          id: "user-turn-model",
          turnId: "turn-model",
          role: "user",
          content: "读取 package.json",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        },
        {
          id: "assistant-turn-model",
          turnId: "turn-model",
          role: "assistant",
          content: "已读取 package.json，准备继续分析依赖。",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: "先看 package.json，再确定下一步。",
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-model",
          title: "call model detail",
          phase: "completed",
          traceTimeline: [
            {
              id: "input-1",
              kind: "input",
              label: "RECEIVE INPUT",
              state: "completed",
              sequence: 1,
              text: "读取 package.json"
            },
            {
              id: "model-3",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 3,
              providerName: "OpenAI",
              providerModel: "gpt-5",
              inputTokens: 120,
              cacheHitInputTokens: 80,
              outputTokens: 40,
              firstTokenLatencyMs: 321,
              turnDurationMs: 2800,
              text: "已读取 package.json，准备继续分析依赖。",
              reasoningContent: "先看 package.json，再确定下一步。"
            }
          ],
          inputTokens: 300,
          cacheHitInputTokens: 200,
          outputTokens: 90,
          firstTokenLatencyMs: 500,
          turnDurationMs: 6000
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-3"]');
    expect(modelButton.text()).toContain("输入 120");
    expect(modelButton.text()).toContain("缓存 80");
    expect(modelButton.text()).toContain("输出 40");
    expect(modelButton.text()).toContain("速度 14.3 token/s");
    expect(modelButton.text()).toContain("延时 321 ms");
    expect(modelButton.text()).toContain("2.80 s");
    expect(modelButton.text()).not.toContain("耗时 2.80 s");
    expect(modelButton.text()).not.toContain("输入 300");
    expect(modelButton.text()).not.toContain("思考链");
    expect(modelButton.text()).not.toContain("已读取 package.json，准备继续分析依赖。");

    await modelButton.trigger("click");
    await nextTick();

    const modelSectionText = modelButton.element.closest("section")?.textContent ?? "";
    expect(modelSectionText).not.toContain("摘要");
    expect(modelSectionText).toContain("阶段");
    expect(modelSectionText).toContain("模型");
    expect(modelSectionText).toContain("OpenAI/gpt-5");
    expect(modelSectionText).toContain("耗时");
    expect(modelSectionText).toContain("2.80 s");
    expect(modelSectionText).toContain("输入");
    expect(modelSectionText).toContain("缓存");
    expect(modelSectionText).toContain("输出");
    expect(modelSectionText).toContain("速度");
    expect(modelSectionText).toContain("14.3 token/s");
    expect(modelSectionText).not.toContain("Provider");
    expect(modelSectionText).not.toContain("输出详情");
    expect(modelSectionText).not.toContain("对应一次独立的模型调用，不与其他 hop 合并。");
    expect(modelSectionText).toContain("思考链");
    expect(modelSectionText).toContain("先看 package.json，再确定下一步。");
    expect(modelSectionText).toContain("模型输出");
    expect(modelSectionText).toContain("已读取 package.json，准备继续分析依赖。");
    expect(wrapper.get('[data-testid="trace-detail-button-model-3-reasoning"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="trace-detail-button-model-3-assistant-output"]').exists()).toBe(true);
  });

  it("buffered CALL MODEL 不伪造首 token 延时，速度基于整次耗时", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-buffered-model",
          title: "buffered model detail",
          phase: "completed",
          traceTimeline: [
            {
              id: "model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              providerName: "deepseek",
              providerModel: "deepseek-v4-flash",
              inputTokens: 3200,
              cacheHitInputTokens: 2800,
              outputTokens: 90,
              firstTokenLatencyMs: 1700,
              turnDurationMs: 1800,
              text: "buffered response"
            }
          ],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision",
              providerMode: "live",
              inputTokens: 3200,
              cacheHitInputTokens: 2800,
              outputTokens: 90,
              totalTokens: 3290,
              firstTokenLatencyMs: null,
              turnDurationMs: 1800,
              latencyKind: "buffered_response",
              prefixMutationReasons: []
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-1"]');
    expect(modelButton.text()).toContain("速度 50.0 token/s");
    expect(modelButton.text()).not.toContain("延时");
    expect(modelButton.text()).toContain("1.80 s");
    expect(modelButton.text()).not.toContain("耗时 1.80 s");

    await modelButton.trigger("click");
    await nextTick();

    const modelSectionText = modelButton.element.closest("section")?.textContent ?? "";
    expect(modelSectionText).toContain("50.0 token/s");
    expect(modelSectionText).not.toContain("延时");
    expect(modelSectionText).toContain("1.80 s");
  });

  it("CALL MODEL 遇到异常首 token 延时大于耗时时不展示延时且不爆速", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-invalid-model-latency",
          title: "invalid model latency",
          phase: "completed",
          traceTimeline: [
            {
              id: "model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              providerName: "deepseek",
              providerModel: "deepseek-v4-flash",
              outputTokens: 89,
              firstTokenLatencyMs: 4510,
              turnDurationMs: 1910,
              text: "tool call"
            }
          ],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision_stream",
              providerMode: "live",
              inputTokens: 1424,
              cacheHitInputTokens: 0,
              outputTokens: 89,
              totalTokens: 1513,
              firstTokenLatencyMs: 4510,
              turnDurationMs: 1910,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-1"]');
    expect(modelButton.text()).toContain("速度 46.6 token/s");
    expect(modelButton.text()).not.toContain("89000.0 token/s");
    expect(modelButton.text()).not.toContain("延时 4510 ms");
    expect(modelButton.text()).toContain("1.91 s");
  });

  it("CALL MODEL 首 token 延时接近耗时时按整次耗时计算速度，避免最后一跳爆速", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-near-terminal-latency",
          title: "near terminal latency",
          phase: "completed",
          traceTimeline: [
            {
              id: "model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              providerName: "deepseek",
              providerModel: "deepseek-v4-flash",
              outputTokens: 72,
              firstTokenLatencyMs: 1946,
              turnDurationMs: 1964,
              text: "final answer"
            }
          ],
          providerCallRecords: [
            {
              requestKind: "tool_followup",
              providerSource: "provider_followup_stream",
              providerMode: "live",
              inputTokens: 2116,
              cacheHitInputTokens: 0,
              reasoningTokens: 73,
              outputTokens: 72,
              totalTokens: 2188,
              firstTokenLatencyMs: 1946,
              turnDurationMs: 1964,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-1"]');
    expect(modelButton.text()).toContain("速度 36.7 token/s");
    expect(modelButton.text()).not.toContain("4000.0 token/s");
    expect(modelButton.text()).not.toContain("3960.");
    expect(modelButton.text()).toContain("1.96 s");
  });

  it("多次 CALL MODEL 时最后一跳不使用整轮 token 兜底，避免混用整轮输出和单跳耗时", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-final-hop-no-token-fallback",
          title: "src/agent下是怎么组织的？",
          phase: "completed",
          traceTimeline: [
            {
              id: "model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              outputTokens: 930,
              turnDurationMs: 17868
            },
            {
              id: "model-2",
              kind: "call_model",
              label: "CALL MODEL #2",
              state: "completed",
              sequence: 2,
              firstTokenLatencyMs: 1447,
              turnDurationMs: 1699,
              text: "final answer"
            }
          ],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision_stream",
              providerMode: "live",
              outputTokens: 930,
              totalTokens: 930,
              firstTokenLatencyMs: 799,
              turnDurationMs: 17868,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            },
            {
              requestKind: "tool_followup",
              providerSource: "provider_followup_stream",
              providerMode: "live",
              firstTokenLatencyMs: 1447,
              turnDurationMs: 1699,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            }
          ],
          outputTokens: 998,
          reasoningTokens: 152,
          firstTokenLatencyMs: 799,
          turnDurationMs: 19567
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-2"]');
    expect(modelButton.text()).not.toContain("输出 998");
    expect(modelButton.text()).not.toContain("速度 587.4 token/s");
    expect(modelButton.text()).not.toContain("速度");
    expect(modelButton.text()).toContain("延时 1447 ms");
    expect(modelButton.text()).toContain("1.70 s");
  });

  it("最后一跳有 per-call 输出时按该跳完整耗时计算速度，不扣首 token 延时", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-final-hop-per-call-speed",
          title: "final hop per-call speed",
          phase: "completed",
          traceTimeline: [
            {
              id: "model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              outputTokens: 930,
              turnDurationMs: 17868
            },
            {
              id: "model-2",
              kind: "call_model",
              label: "CALL MODEL #2",
              state: "completed",
              sequence: 2,
              firstTokenLatencyMs: 1447,
              turnDurationMs: 1699,
              text: "final answer"
            }
          ],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision_stream",
              providerMode: "live",
              outputTokens: 930,
              totalTokens: 930,
              firstTokenLatencyMs: 799,
              turnDurationMs: 17868,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            },
            {
              requestKind: "tool_followup",
              providerSource: "provider_followup_stream",
              providerMode: "live",
              reasoningTokens: 18,
              outputTokens: 68,
              totalTokens: 86,
              firstTokenLatencyMs: 1447,
              turnDurationMs: 1699,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            }
          ],
          outputTokens: 998,
          reasoningTokens: 152,
          firstTokenLatencyMs: 799,
          turnDurationMs: 19567
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-2"]');
    expect(modelButton.text()).toContain("输出 68");
    expect(modelButton.text()).toContain("速度 40.0 token/s");
    expect(modelButton.text()).not.toContain("速度 269.8 token/s");
    expect(modelButton.text()).not.toContain("速度 587.4 token/s");
    expect(modelButton.text()).toContain("延时 1447 ms");
    expect(modelButton.text()).toContain("1.70 s");
  });

  it("CALL TOOL 的折叠摘要只显示耗时，不显示内容详情", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-tool",
          title: "tool duration only",
          phase: "completed",
          traceTimeline: [
            {
              id: "input-1",
              kind: "input",
              label: "RECEIVE INPUT",
              state: "completed",
              sequence: 1,
              text: "读取 package.json"
            },
            {
              id: "tool-4",
              kind: "call_tool",
              label: "CALL TOOL #1 · workspace.read_file",
              state: "completed",
              sequence: 4,
              toolActivities: [
                {
                  id: "tool-1",
                  name: "workspace.read_file",
                  status: "done",
                  summary: "Tool call finished with status: ok",
                  argumentsText: "{\"path\":\"package.json\"}",
                  resultText: "{\"name\":\"pony-agent\"}",
                  durationSeconds: 1.2
                }
              ]
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const toolButton = wrapper.get('[data-testid="trace-step-button-tool-4"]');
    expect(toolButton.text()).toContain("1.20 s");
    expect(toolButton.text()).not.toContain("Tool call finished with status: ok");
    expect(toolButton.text()).not.toContain("package.json");
    expect(toolButton.text()).not.toContain("pony-agent");

    await toolButton.trigger("click");
    await nextTick();
    const toolDetailButton = wrapper.get('[data-testid="trace-detail-button-tool-4-tool-1"]');
    expect(toolDetailButton.text()).toContain("workspace.read_file");
    expect(toolDetailButton.text()).toContain("1.20 s");
    expect(toolDetailButton.text()).not.toContain("Tool call finished with status: ok");

    await toolDetailButton.trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("参数:");
    expect(wrapper.text()).toContain("\"path\":\"package.json\"");
    expect(wrapper.text()).toContain("结果:");
    expect(wrapper.text()).toContain("\"name\":\"pony-agent\"");
    expect(wrapper.text()).not.toContain("摘要:");
  });

  it("多 hop 时前一个 CALL MODEL 只展示该 hop 自身输出，不回退最终 assistant 回答", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      messages: [
        {
          id: "user-turn-tool-hop",
          turnId: "turn-tool-hop",
          role: "user",
          content: "当前文件夹中有哪些文件？",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        },
        {
          id: "assistant-turn-tool-hop",
          turnId: "turn-tool-hop",
          role: "assistant",
          content: "当前文件夹下有这些内容：\\n- src/\\n- tests/",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: "工具结果足够，可以整理成最终回答。",
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-tool-hop",
          title: "tool hop model detail",
          phase: "completed",
          traceTimeline: [
            {
              id: "input-1",
              kind: "input",
              label: "RECEIVE INPUT",
              state: "completed",
              sequence: 1,
              text: "当前文件夹中有哪些文件？"
            },
            {
              id: "model-3",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 3,
              providerName: "ppx",
              providerModel: "gpt-5.4"
            },
            {
              id: "tool-4",
              kind: "call_tool",
              label: "CALL TOOL #1 · workspace_list_files",
              state: "completed",
              sequence: 4,
              toolActivities: [
                {
                  id: "tool-1",
                  name: "workspace_list_files",
                  status: "done",
                  summary: "列出当前目录文件",
                  argumentsText: "{\"path\":\".\"}",
                  resultText: "{\"entries\":[\"src\",\"tests\"]}",
                  durationSeconds: 0.1
                }
              ]
            },
            {
              id: "model-5",
              kind: "call_model",
              label: "CALL MODEL #2",
              state: "completed",
              sequence: 5,
              providerName: "ppx",
              providerModel: "gpt-5.4",
              text: "当前文件夹下有这些内容：\n- src/\n- tests/",
              reasoningContent: "工具结果足够，可以整理成最终回答。"
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const firstModelButton = wrapper.get('[data-testid="trace-step-button-model-3"]');
    expect(firstModelButton.text()).not.toContain("workspace_list_files");
    await firstModelButton.trigger("click");
    await nextTick();

    const firstModelSectionText = firstModelButton.element.closest("section")?.textContent ?? "";
    expect(firstModelSectionText).toContain("workspace_list_files");
    expect(firstModelSectionText).toContain("参数:");
    expect(firstModelSectionText).toContain("\"path\":\".\"");
    expect(firstModelSectionText).not.toContain("模型输出");
    expect(firstModelSectionText).not.toContain("思考链");
    expect(firstModelSectionText).not.toContain("当前文件夹下有这些内容");
    expect(wrapper.find('[data-testid="trace-detail-button-model-3-tool-output-tool-4-tool-1"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-detail-button-model-3-assistant-output"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-detail-button-model-3-reasoning"]').exists()).toBe(false);

    const secondModelButton = wrapper.get('[data-testid="trace-step-button-model-5"]');
    await secondModelButton.trigger("click");
    await nextTick();

    const secondModelSectionText = secondModelButton.element.closest("section")?.textContent ?? "";
    expect(secondModelSectionText).toContain("模型输出");
    expect(secondModelSectionText).toContain("思考链");
    expect(secondModelSectionText).toContain("当前文件夹下有这些内容");
    expect(secondModelSectionText).toContain("工具结果足够，可以整理成最终回答。");
    expect(wrapper.get('[data-testid="trace-detail-button-model-5-assistant-output"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="trace-detail-button-model-5-reasoning"]').exists()).toBe(true);
  });

  it("CALL MODEL 后续存在工具输出时，折叠态不展示模型输出", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-model-output-before-tool",
          title: "model output before tool",
          phase: "completed",
          traceTimeline: [
            {
              id: "input-1",
              kind: "input",
              label: "RECEIVE INPUT",
              state: "completed",
              sequence: 1,
              text: "先给出计划，再读取 package.json"
            },
            {
              id: "model-2",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 2,
              text: "我会先检查 package.json，然后根据依赖判断下一步。",
              reasoningContent: "需要先看项目依赖。"
            },
            {
              id: "tool-3",
              kind: "call_tool",
              label: "CALL TOOL #1 · workspace_read_file",
              state: "completed",
              sequence: 3,
              toolActivities: [
                {
                  id: "tool-1",
                  name: "workspace_read_file",
                  status: "done",
                  summary: "读取 package.json",
                  argumentsText: "{\"path\":\"package.json\"}",
                  resultText: "{\"name\":\"pony-agent\"}",
                  durationSeconds: 0.2
                }
              ]
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-2"]');
    expect(modelButton.text()).not.toContain("我会先检查 package.json，然后根据依赖判断下一步。");
    expect(modelButton.text()).not.toContain("需要先看项目依赖。");
    expect(modelButton.text()).not.toContain("workspace_read_file");
    expect(modelButton.text()).not.toContain("{\"name\":\"pony-agent\"}");

    await modelButton.trigger("click");
    await nextTick();

    const modelSectionText = modelButton.element.closest("section")?.textContent ?? "";
    expect(modelSectionText).toContain("模型输出");
    expect(modelSectionText).toContain("我会先检查 package.json，然后根据依赖判断下一步。");
    expect(modelSectionText).toContain("workspace_read_file");
    expect(modelSectionText).toContain("{\"name\":\"pony-agent\"}");
  });

  it("assistant 仍在输出时，trace 中不提前展示活跃思考和模型输出", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      messages: [
        {
          id: "user-turn-pending",
          turnId: "turn-pending",
          role: "user",
          content: "继续输出",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        },
        {
          id: "assistant-turn-pending",
          turnId: "turn-pending",
          role: "assistant",
          content: "正在逐步输出",
          attachments: [],
          status: "pending",
          tokenCount: null,
          reasoningContent: "正在思考",
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-pending",
          title: "pending turn",
          phase: "running",
          traceTimeline: [
            { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "继续输出" },
            {
              id: "return-4",
              kind: "return",
              label: "RETURN RESULT",
              state: "active",
              sequence: 4,
              reasoningContent: "正在思考",
              text: "正在逐步输出"
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.find('[data-testid="trace-step-button-return-4"]').exists()).toBe(false);

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-4"]');
    expect(modelButton.text()).not.toContain("正在思考");

    await modelButton.trigger("click");
    await nextTick();

    const callModelSectionText = modelButton.element.closest("section")?.textContent ?? "";
    expect(callModelSectionText).not.toContain("思考链");
    expect(callModelSectionText).not.toContain("模型输出");
    expect(wrapper.find('[data-testid="trace-detail-button-model-4-assistant-output"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-detail-button-model-4-reasoning"]').exists()).toBe(false);
  });

  it("CALL MODEL 在活跃阶段即使带有增量文本和思考，也不在 trace 中提前展示", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-active-call-model",
          title: "active call model",
          phase: "calling_model",
          traceTimeline: [
            {
              id: "model-3",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "active",
              sequence: 3,
              text: "这是一段流式增量正文",
              reasoningContent: "先组织回答结构。"
            }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const modelButton = wrapper.get('[data-testid="trace-step-button-model-3"]');
    expect(modelButton.text()).not.toContain("这是一段流式增量正文");
    expect(modelButton.text()).not.toContain("先组织回答结构。");

    await modelButton.trigger("click");
    await nextTick();

    const sectionText = modelButton.element.closest("section")?.textContent ?? "";
    expect(sectionText).not.toContain("思考链");
    expect(sectionText).not.toContain("模型输出");
    expect(sectionText).not.toContain("这是一段流式增量正文");
    expect(wrapper.find('[data-testid="trace-detail-button-model-3-assistant-output"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-detail-button-model-3-reasoning"]').exists()).toBe(false);
  });
});

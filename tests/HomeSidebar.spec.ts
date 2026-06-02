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
      goal: "梳理 retrieval boundary 并推进上层消费入口",
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
    { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "继续推进 PA-025，不要生成摘要" },
    { id: "context-2", kind: "context", label: "BUILD CONTEXT", state: "completed", sequence: 2 },
    { id: "model-3", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 3, firstTokenLatencyMs: 321 },
    { id: "return-4", kind: "return", label: "RETURN RESULT", state: "completed", sequence: 4, inputTokens: 120, cacheHitInputTokens: 80, reasoningTokens: 18, outputTokens: 40, totalTokens: 160, firstTokenLatencyMs: 321, turnDurationMs: 2800 }
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
    updatedAt: overrides.updatedAt ?? 1
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

  it("只在 turn 和 call model 展示 token 指标，并在展开输入原文时隐藏重复摘要", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      messages: [
        {
          id: "user-turn-token",
          turnId: "turn-token",
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
          id: "assistant-turn-token",
          turnId: "turn-token",
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
          turnId: "turn-token",
          title: "token metrics",
          phase: "completed",
          traceTimeline: [
            { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "继续推进 PA-025，不要生成摘要" },
            { id: "context-2", kind: "context", label: "BUILD CONTEXT", state: "completed", sequence: 2 },
            {
              id: "model-3",
              kind: "model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 3,
              inputTokens: 120,
              cacheHitInputTokens: 80,
              reasoningTokens: 18,
              outputTokens: 40,
              firstTokenLatencyMs: 321,
              turnDurationMs: 2800
            },
            { id: "return-4", kind: "return", label: "RETURN RESULT", state: "completed", sequence: 4, turnDurationMs: 2800 }
          ],
          inputTokens: 120,
          cacheHitInputTokens: 80,
          reasoningTokens: 18,
          outputTokens: 40,
          totalTokens: 160,
          firstTokenLatencyMs: 321,
          turnDurationMs: 2800
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.text()).toContain("延时");
    expect(wrapper.text()).not.toContain("首个增量");
    expect(wrapper.text()).toContain("输入 120");
    expect(wrapper.text()).toContain("命中缓存 80");
    expect(wrapper.text()).toContain("思考链 18");
    expect(wrapper.text()).toContain("输出 40");
    expect(wrapper.text()).not.toContain("总计 160");
    expect(wrapper.text()).toContain("2.80 s");

    expect(wrapper.get('[data-testid="trace-step-button-input-1"]').text()).toContain("RECEIVE INPUT");
    expect(wrapper.get('[data-testid="trace-step-button-input-1"]').text()).not.toContain("输入 120");
    expect(wrapper.get('[data-testid="trace-step-button-model-3"]').text()).toContain("输入 120");
    expect(wrapper.get('[data-testid="trace-step-button-model-3"]').text()).toContain("命中缓存 80");
    expect(wrapper.get('[data-testid="trace-step-button-model-3"]').text()).toContain("思考链 18");
    expect(wrapper.get('[data-testid="trace-step-button-model-3"]').text()).toContain("输出 40");
    expect(wrapper.get('[data-testid="trace-step-button-model-3"]').text()).toContain("16.1 token/s");
    expect(wrapper.get('[data-testid="trace-step-button-model-3"]').text()).toContain("延时 321 ms");
    expect(wrapper.find('[data-testid="trace-step-button-return-4"]').exists()).toBe(false);

    await wrapper.get('[data-testid="trace-step-button-input-1"]').trigger("click");
    await nextTick();

    const inputDetailButton = wrapper.get('[data-testid="trace-detail-button-input-1-input-message"]');
    expect(inputDetailButton.text()).toContain("继续推进 PA-025，不要生成摘要");

    await inputDetailButton.trigger("click");
    await nextTick();

    expect(inputDetailButton.text()).not.toContain("继续推进 PA-025，不要生成摘要");
    expect(wrapper.find('[data-testid="trace-detail-button-input-1-content"]').exists()).toBe(false);

    await wrapper.get('[data-testid="trace-step-button-model-3"]').trigger("click");
    await nextTick();

    const callModelSectionText = wrapper.get('[data-testid="trace-step-button-model-3"]').element.closest("section")?.textContent ?? "";
    expect(callModelSectionText).toContain("输入");
    expect(callModelSectionText).toContain("命中缓存");
    expect(callModelSectionText).toContain("思考链");
    expect(callModelSectionText).toContain("输出");
    expect(callModelSectionText).toContain("16.1 token/s");
    expect(callModelSectionText).toContain("总计");
  });

  it("仅在存在整轮耗时时显示 turn 耗时", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-no-duration",
          title: "duration source",
          phase: "completed",
          toolActivities: [
            {
              id: "tool-duration-only",
              name: "workspace.read_file",
              status: "done",
              summary: null,
              argumentsText: "{}",
              resultText: "{}",
              durationSeconds: 4.5
            }
          ],
          turnDurationMs: null
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const turnButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("duration source"));

    expect(turnButton).toBeDefined();
    expect(turnButton!.text()).not.toContain("4.50 s");
  });

  it("调整 call tool 和最终 call model 的折叠头信息", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      messages: [
        {
          id: "user-turn-tool",
          turnId: "turn-tool",
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
          id: "assistant-turn-tool",
          turnId: "turn-tool",
          role: "assistant",
          content: "已读取 package.json。",
          attachments: [],
          status: "done",
          tokenCount: null,
          reasoningContent: "先看 package.json 再判断下一步。",
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-tool",
          title: "tool layout",
          phase: "completed",
          traceTimeline: [
            { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "读取 package.json" },
            { id: "context-2", kind: "context", label: "BUILD CONTEXT", state: "completed", sequence: 2 },
            { id: "model-3", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 3, reasoningContent: "先看 package.json 再判断下一步。" },
            {
              id: "tool-4",
              kind: "tool",
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
            },
            { id: "return-5", kind: "return", label: "RETURN RESULT", state: "completed", sequence: 5, text: "已读取 package.json。", reasoningContent: "先看 package.json 再判断下一步。" }
          ],
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
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.find('[data-testid="trace-step-button-return-5"]').exists()).toBe(false);
    const finalModelStepButton = wrapper.get('[data-testid="trace-step-button-model-3"]');
    expect(finalModelStepButton.text()).toContain("已读取 package.json。");

    await wrapper.get('[data-testid="trace-step-button-tool-4"]').trigger("click");
    await nextTick();

    const toolButton = wrapper.get('[data-testid="trace-detail-button-tool-4-tool-1"]');
    expect(toolButton.text()).toContain("workspace.read_file");
    expect(toolButton.text()).toContain("1.20 s");
    expect(toolButton.text()).not.toContain("第 1 次工具调用");
    expect(toolButton.text()).not.toContain("Tool call finished with status: ok");

    await toolButton.trigger("click");
    await nextTick();

    expect(wrapper.text()).toContain("参数:");
    expect(wrapper.text()).toContain("\"path\":\"package.json\"");
    expect(wrapper.text()).toContain("结果:");
    expect(wrapper.text()).toContain("\"name\":\"pony-agent\"");

    await wrapper.get('[data-testid="trace-step-button-model-3"]').trigger("click");
    await nextTick();

    const finalModelSectionText = wrapper.get('[data-testid="trace-step-button-model-3"]').element.closest("section")?.textContent ?? "";
    expect(finalModelSectionText).toContain("思考链");
    expect(finalModelSectionText).toContain("模型输出");
    expect(finalModelSectionText).not.toContain("会话摘要");
    expect(finalModelSectionText.indexOf("思考链")).toBeLessThan(finalModelSectionText.indexOf("模型输出"));
  });

  it("按时序展示多次 call model / call tool，而不是合并为单步", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-multi-hop",
          title: "multi hop trace",
          traceTimeline: [
            { id: "input-1", kind: "input", label: "RECEIVE INPUT", state: "completed", sequence: 1, text: "继续推进" },
            { id: "retrieval-2", kind: "prepare_retrieval", label: "PREPARE RETRIEVAL", state: "completed", sequence: 2 },
            { id: "context-3", kind: "build_context", label: "BUILD CONTEXT", state: "completed", sequence: 3 },
            { id: "model-4", kind: "call_model", label: "CALL MODEL #1", state: "completed", sequence: 4 },
            { id: "tool-5", kind: "call_tool", label: "CALL TOOL #1 · workspace.list_files", state: "completed", sequence: 5, toolActivities: [] },
            { id: "model-6", kind: "call_model", label: "CALL MODEL #2", state: "completed", sequence: 6 },
            { id: "tool-7", kind: "call_tool", label: "CALL TOOL #2 · workspace.read_file", state: "completed", sequence: 7, toolActivities: [] },
            { id: "model-8", kind: "call_model", label: "CALL MODEL #3", state: "completed", sequence: 8 },
            { id: "return-9", kind: "return_result", label: "RETURN RESULT", state: "completed", sequence: 9, outputTokens: 20, turnDurationMs: 1200 }
          ]
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    const traceText = wrapper.text();
    expect(traceText).toContain("CALL MODEL #1");
    expect(traceText).toContain("CALL TOOL #1 · workspace.list_files");
    expect(traceText).toContain("CALL MODEL #2");
    expect(traceText).toContain("CALL TOOL #2 · workspace.read_file");
    expect(traceText).toContain("CALL MODEL #3");
  });

  it("assistant 仍在输出时将活跃输出归到 call model", async () => {
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
    await wrapper.get('[data-testid="trace-step-button-model-4"]').trigger("click");
    await nextTick();

    const callModelSectionText = wrapper.get('[data-testid="trace-step-button-model-4"]').element.closest("section")?.textContent ?? "";
    expect(callModelSectionText).toContain("思考链");
    expect(callModelSectionText).toContain("模型输出");
    expect(wrapper.find('[data-testid="trace-detail-button-model-4-assistant-output"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-detail-button-model-4-reasoning"]').exists()).toBe(true);
  });
});

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

function createTraceRecord(overrides: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: overrides.turnId ?? "turn-1",
    title: overrides.title ?? "测试轮次",
    phase: overrides.phase ?? "ready",
    traceSteps: overrides.traceSteps ?? createTraceSteps(),
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
    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("false");

    await wrapper.get('[data-testid="status-panel-toggle"]').trigger("click");
    await nextTick();

    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="trace-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("false");
  });

  it("将 build context 观测折叠进 trace step，而不是独立卡片", async () => {
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

    const retrievedSummary = wrapper.get('[data-testid="retrieved-context-summary"]');
    const tracePanelText = wrapper.get('[data-testid="trace-panel-toggle"]').element.closest("section")?.textContent ?? "";

    expect(retrievedSummary.text()).toContain("Current retrieval state");
    expect(retrievedSummary.text()).toContain("Recent history");
    expect(wrapper.get('[data-testid="retrieved-run-goal"]').text()).toContain("retrieval boundary");
    expect(wrapper.get('[data-testid="retrieved-active-task"]').text()).toContain("PA-018");
    expect(wrapper.get('[data-testid="retrieved-last-file"]').text()).toContain("src-tauri/src/agent/context.rs");
    expect(wrapper.get('[data-testid="retrieved-memory-list"]').text()).toContain("Reply in Chinese.");

    expect(wrapper.find('[data-testid="turn-build-context"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-step-button-step-context"]').exists()).toBe(true);

    await wrapper.get('[data-testid="trace-step-button-step-context"]').trigger("click");
    await nextTick();

    const contextStepText = wrapper.get('[data-testid="trace-step-button-step-context"]').element.closest("section")?.textContent ?? "";
    expect(contextStepText).toContain("response_format=json_schema");
    expect(contextStepText).toContain("这里展示的是本轮真正发给模型的请求，不是 retrieval state 的替身。");
    expect(contextStepText).toContain("稳定前缀");
    expect(contextStepText).toContain("最终请求消息");
    expect(contextStepText).toContain("工具定义");

    await wrapper.get('[data-testid="trace-detail-button-step-context-stable"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("stable capability prefix");

    await wrapper.get('[data-testid="trace-detail-button-step-context-semi"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("retrieval summary from host");

    await wrapper.get('[data-testid="trace-detail-button-step-context-volatile"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("latest screenshot");

    await wrapper.get('[data-testid="trace-detail-button-step-context-messages"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("system: summarize retrieval state");

    await wrapper.get('[data-testid="trace-detail-button-step-context-tools"]').trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("workspace.read_file(path: string)");

    expect(retrievedSummary.text()).not.toContain("system: summarize retrieval state");
    expect(retrievedSummary.text()).not.toContain("stable capability prefix");
    expect(tracePanelText).toContain("Current retrieval state");
    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "").not.toContain("Current retrieval state");
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

    expect(wrapper.get('[data-testid="trace-step-button-step-plan"]').text()).toContain("RECEIVE INPUT");
    expect(wrapper.get('[data-testid="trace-step-button-step-plan"]').text()).not.toContain("输入 120");
    expect(wrapper.get('[data-testid="trace-step-button-step-return"]').text()).not.toContain("输出 40");
    expect(wrapper.get('[data-testid="trace-step-button-step-call-model"]').text()).toContain("输入 120");
    expect(wrapper.get('[data-testid="trace-step-button-step-call-model"]').text()).toContain("命中缓存 80");
    expect(wrapper.get('[data-testid="trace-step-button-step-call-model"]').text()).toContain("思考链 18");
    expect(wrapper.get('[data-testid="trace-step-button-step-call-model"]').text()).toContain("输出 40");
    expect(wrapper.get('[data-testid="trace-step-button-step-call-model"]').text()).not.toContain("总计 160");
    expect(wrapper.get('[data-testid="trace-step-button-step-call-model"]').text()).toContain("延时 321 ms");

    await wrapper.get('[data-testid="trace-step-button-step-plan"]').trigger("click");
    await nextTick();

    const inputDetailButton = wrapper.get('[data-testid="trace-detail-button-step-plan-input-message"]');
    expect(inputDetailButton.text()).toContain("继续推进 PA-025，不要生成摘要");

    await inputDetailButton.trigger("click");
    await nextTick();

    expect(inputDetailButton.text()).not.toContain("继续推进 PA-025，不要生成摘要");

    await wrapper.get('[data-testid="trace-step-button-step-call-model"]').trigger("click");
    await nextTick();

    const callModelSectionText = wrapper.get('[data-testid="trace-step-button-step-call-model"]').element.closest("section")?.textContent ?? "";
    expect(callModelSectionText).toContain("输入");
    expect(callModelSectionText).toContain("命中缓存");
    expect(callModelSectionText).toContain("思考链");
    expect(callModelSectionText).toContain("输出");
    expect(callModelSectionText).not.toContain("总计");
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

  it("调整 call tool 和 return result 的折叠头信息", async () => {
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
          traceSteps: createTraceSteps(),
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

    const returnStepButton = wrapper.get('[data-testid="trace-step-button-step-return"]');
    expect(returnStepButton.text()).toContain("已读取 package.json。");

    await wrapper.get('[data-testid="trace-step-button-step-call-tool"]').trigger("click");
    await nextTick();

    const toolButton = wrapper.get('[data-testid="trace-detail-button-step-call-tool-tool-1"]');
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

    await wrapper.get('[data-testid="trace-step-button-step-return"]').trigger("click");
    await nextTick();

    const returnSectionText = wrapper.get('[data-testid="trace-step-button-step-return"]').element.closest("section")?.textContent ?? "";
    expect(returnSectionText).toContain("思考链");
    expect(returnSectionText).toContain("最终回复");
    expect(returnSectionText).not.toContain("会话摘要");
    expect(returnSectionText.indexOf("思考链")).toBeLessThan(returnSectionText.indexOf("最终回复"));
    expect(wrapper.get('[data-testid="trace-step-button-step-return"]').text()).not.toContain("已读取 package.json。");
  });

  it("assistant 仍在输出时不提前渲染 return result 详情", async () => {
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
          traceSteps: createTraceSteps()
        })
      ]
    });

    const wrapper = mountSidebar();
    await flushAll();

    await wrapper.get('[data-testid="trace-step-button-step-return"]').trigger("click");
    await nextTick();

    const returnSectionText = wrapper.get('[data-testid="trace-step-button-step-return"]').element.closest("section")?.textContent ?? "";
    expect(returnSectionText).not.toContain("思考链");
    expect(returnSectionText).not.toContain("最终回复");
    expect(wrapper.find('[data-testid="trace-detail-button-step-return-assistant-output"]').exists()).toBe(false);
  });
});

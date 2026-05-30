import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSidebar from "@/components/HomeSidebar.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";
import type { RetrievedContextState } from "@/types/runtime";

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
      userMessage: "继续推进 PA-018",
      images: [],
      referencesImage: false
    },
    sessionContext: {
      conversationId: "graph-session",
      title: "Graph session",
      summary: "retrieval summary from host",
      recentHistory: [
        { role: "user", content: "继续推进 PA-018" },
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

  it("保留原有状态、工具、轨迹面板结构", async () => {
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

  it("retrieval 信息整合进 trace 面板，而不是状态面板主体", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionSummary: "legacy summary should be shadowed",
      retrievedContext: createRetrievedContext()
    });

    const wrapper = mountSidebar();
    await flushAll();

    expect(wrapper.get('[data-testid="retrieved-context-summary"]').text()).toContain("Recent history");
    expect(wrapper.get('[data-testid="retrieved-run-goal"]').text()).toContain("retrieval boundary");
    expect(wrapper.get('[data-testid="retrieved-active-task"]').text()).toContain("PA-018");
    expect(wrapper.get('[data-testid="retrieved-last-file"]').text()).toContain("src-tauri/src/agent/context.rs");
    expect(wrapper.get('[data-testid="retrieved-memory-list"]').text()).toContain("Reply in Chinese.");
    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "").not.toContain("Retrieval");
  });
});

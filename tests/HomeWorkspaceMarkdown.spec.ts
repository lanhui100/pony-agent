import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";

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

const ButtonStub = defineComponent({
  props: {
    disabled: {
      type: Boolean,
      default: false
    }
  },
  emits: ["click"],
  template: "<button class=\"button-stub\" type=\"button\" :disabled=\"disabled\" @click=\"$emit('click')\"><slot /></button>"
});

const SwitchStub = defineComponent({
  props: {
    modelValue: {
      type: Boolean,
      default: false
    }
  },
  template: '<input class="switch-stub" type="checkbox" :checked="modelValue" />'
});

async function flushMarkdownRender() {
  // MarkdownRenderer 使用异步渲染（debounce 90ms+setTimeout(0)），
  // 这里用多重 tick + 足够的时间窗口确保渲染完成。
  for (let i = 0; i < 5; i++) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 60));
    await nextTick();
  }
}

function createProviderRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-ppx",
    providers: [
      {
        id: "provider-ppx",
        name: "ppx",
        protocol: "openai",
        baseUrl: "https://example.com/v1",
        apiKeyEnvVar: "PPX_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-gpt-5-4",
        models: [
          {
            id: "model-gpt-5-4",
            name: "GPT 5.4",
            model: "gpt-5.4",
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

describe("HomeWorkspace markdown rendering", () => {
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
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders assistant markdown as html instead of raw source", async () => {
    const providerStore = useProviderStore();
    providerStore.$patch({
      registry: createProviderRegistry(),
      selectedReasoningEffort: null
    });

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      phase: "ready",
      sessionOperation: null,
      error: null,
      messages: [
        {
          id: "user-1",
          turnId: "turn-1",
          role: "user",
          content: "给我一份 README",
          status: "done",
          tokenCount: null
        },
        {
          id: "assistant-1",
          turnId: "turn-1",
          role: "assistant",
          content: [
            "```md",
            "# 标题",
            "",
            "> 引用",
            "",
            "- 列表项",
            "",
            "**加粗**",
            "```"
          ].join("\n"),
          status: "done",
          tokenCount: null,
          modelName: "ppx/gpt-5.4"
        }
      ]
    });

    const wrapper = mount({
      render() {
        return h(TooltipProvider, null, {
          default: () => h(HomeWorkspace)
        });
      }
    }, {
      global: {
        stubs: {
          ScrollArea: ScrollAreaStub,
          Button: ButtonStub,
          Switch: SwitchStub,
          Transition: false,
          TransitionGroup: false
        }
      }
    });

    await nextTick();
    await flushMarkdownRender();

    expect(wrapper.html()).toContain("<h1>标题</h1>");
    expect(wrapper.html()).toContain("<blockquote>");
    expect(wrapper.html()).toContain("<strong>加粗</strong>");
    expect(wrapper.html()).not.toContain("```md");
  }, 15000);

  it("shows plain text while assistant is streaming and renders markdown when streaming completes", async () => {
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "20");
    const providerStore = useProviderStore();
    providerStore.$patch({
      registry: createProviderRegistry(),
      selectedReasoningEffort: null
    });

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      phase: "streaming_response",
      activeTurnId: "turn-stream-markdown",
      isSubmitting: true,
      sessionOperation: null,
      error: null,
      messages: [
        {
          id: "user-stream-markdown",
          turnId: "turn-stream-markdown",
          role: "user",
          content: "继续",
          status: "done",
          tokenCount: null
        },
        {
          id: "assistant-stream-markdown",
          turnId: "turn-stream-markdown",
          role: "assistant",
          content: "**流式加粗**",
          status: "pending",
          tokenCount: null,
          modelName: "ppx/gpt-5.4"
        }
      ]
    });

    const wrapper = mount({
      render() {
        return h(TooltipProvider, null, {
          default: () => h(HomeWorkspace)
        });
      }
    }, {
      global: {
        stubs: {
          ScrollArea: ScrollAreaStub,
          Button: ButtonStub,
          Switch: SwitchStub,
          Transition: false,
          TransitionGroup: false
        }
      }
    });

    await flushMarkdownRender();

    // 流式期间：含 Markdown 语法的内容走增量 Markdown 渲染
    const streamingFlow = wrapper.get('[data-testid="assistant-streaming-flow"]');
    expect(streamingFlow.find('.markdown-body').text()).toBe("流式加粗");

    // 流式完成时：保留最终 markdown 渲染
    const assistantMessage = runtimeStore.messages.find((m) => m.role === "assistant");
    if (assistantMessage) {
      assistantMessage.status = "done";
    }
    await nextTick();
    await flushMarkdownRender();

    expect(wrapper.find('.markdown-body').exists()).toBe(true);
    expect(wrapper.html()).toContain("<strong>流式加粗</strong>");

    wrapper.unmount();
  }, 15000);

  /**
   * wave-59 F3 契约演进（Lead 授权迁移，原意图不变、仅迁移断言锚点）：
   * 「流式正文逐字符 span（.assistant-streaming-char）释放」的原断言迁移为 F3-4：
   * 内容区不得出现逐字符节点；原语义「正文随释放节奏逐步可见、不走 v-motion 包裹」保留。
   */
  it("reveals streaming assistant content with continuous release (no per-character nodes, F3-4)", async () => {
    const providerStore = useProviderStore();
    providerStore.$patch({
      registry: createProviderRegistry(),
      selectedReasoningEffort: null
    });

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      phase: "streaming_response",
      activeTurnId: "turn-stream",
      isSubmitting: true,
      sessionOperation: null,
      error: null,
      messages: [
        {
          id: "user-stream",
          turnId: "turn-stream",
          role: "user",
          content: "继续",
          status: "done",
          tokenCount: null
        },
        {
          id: "assistant-stream",
          turnId: "turn-stream",
          role: "assistant",
          content: "hello",
          status: "pending",
          tokenCount: null,
          modelName: "ppx/gpt-5.4"
        }
      ]
    });

    const wrapper = mount({
      render() {
        return h(TooltipProvider, null, {
          default: () => h(HomeWorkspace)
        });
      }
    }, {
      global: {
        stubs: {
          ScrollArea: ScrollAreaStub,
          Button: ButtonStub,
          Switch: SwitchStub,
          Transition: false,
          TransitionGroup: false
        }
      }
    });

    await nextTick();
    const streamingFlow = wrapper.find('[data-testid="assistant-streaming-flow"]');
    expect(streamingFlow.exists()).toBe(true);

    // F3-4：内容区不得出现逐字符节点（D-59-2：逐字淡入已移除）
    expect(wrapper.findAll('.assistant-streaming-char')).toHaveLength(0);

    // 原语义保留：正文随释放节奏逐步可见（展示层为完整内容的前缀）
    await vi.waitFor(() => {
      expect(streamingFlow.text().length).toBeGreaterThan(0);
    }, { timeout: 5000, interval: 20 });
    expect('hello'.startsWith(streamingFlow.text())).toBe(true);

    // No v-motion opacity wrapper on streaming content
    expect(streamingFlow.classes()).not.toContain('assistant-response-panel');
    expect(wrapper.find('[data-testid="assistant-streaming-fade-batch"]').exists()).toBe(false);

    wrapper.unmount();
  }, 15000);

  /**
   * A-59-2 回归（契约 §7，审查 review-join-wave59 CORRECTNESS）：
   * `streamingRenderPathLock` 的 `false`（纯文本路径）不得延续到完成态。
   * 窄场景：纯文本流式期间锁已被置 false，**最后一个 delta 才引入 markdown 语法**且期间无渲染机会，
   * 若完成态继续信任旧锁，`**` 会被原样当纯文本显示。
   * 契约取舍：该窄场景下完成帧允许一次子树替换（正确性优先，登记为 RC5 的已知例外）。
   */
  it("A-59-2: 纯文本流式（锁 false）在最后才出现 markdown 语法时，完成态必须渲染 <strong> 而非原样 **", async () => {
    const providerStore = useProviderStore();
    providerStore.$patch({
      registry: createProviderRegistry(),
      selectedReasoningEffort: null
    });

    const runtimeStore = useRuntimeStore();
    const turnId = "turn-lock-done";
    const userMessage = {
      id: "user-lock-done",
      turnId,
      role: "user" as const,
      content: "继续",
      status: "done" as const,
      tokenCount: null
    };
    runtimeStore.$patch({
      phase: "streaming_response",
      activeTurnId: turnId,
      isSubmitting: true,
      sessionOperation: null,
      error: null,
      messages: [
        userMessage,
        {
          id: "assistant-lock-done",
          turnId,
          role: "assistant",
          content: "纯文本第一段",
          status: "pending",
          tokenCount: null,
          modelName: "ppx/gpt-5.4"
        }
      ] as never
    });

    const wrapper = mount({
      render() {
        return h(TooltipProvider, null, {
          default: () => h(HomeWorkspace)
        });
      }
    }, {
      global: {
        stubs: {
          ScrollArea: ScrollAreaStub,
          Button: ButtonStub,
          Switch: SwitchStub,
          Transition: false,
          TransitionGroup: false
        }
      }
    });

    await flushMarkdownRender();
    // 自检：流式期间是纯文本（无 markdown 语法）——此时锁被置为 false
    const sectionStreaming = wrapper.get('[data-testid="workspace-content-column"] section[data-turn-id]');
    expect(sectionStreaming.find("strong").exists()).toBe(false);
    expect(sectionStreaming.text()).toContain("纯文本第一段");

    // Act：同一 tick 内补齐 markdown 语法并把消息转 done（期间不给渲染机会）
    await runtimeStore.$patch({
      isSubmitting: false,
      phase: "ready",
      messages: [
        userMessage,
        {
          id: "assistant-lock-done",
          turnId,
          role: "assistant",
          content: "纯文本第一段 **加粗收尾**",
          status: "done",
          tokenCount: null,
          modelName: "ppx/gpt-5.4"
        }
      ] as never
    });
    await nextTick();
    await flushMarkdownRender();

    // Assert（完成态必须按 markdown 渲染，不得原样暴露 **）
    const section = wrapper.get('[data-testid="workspace-content-column"] section[data-turn-id]');
    const bodyText = section.text();
    expect(
      section.find("strong").exists(),
      `A-59-2：完成态必须渲染 <strong>（锁 false 不得延续到完成态）；实际正文：${JSON.stringify(bodyText)}`
    ).toBe(true);
    expect(bodyText).toContain("加粗收尾");
    expect(
      bodyText,
      `A-59-2：完成态不得把 markdown 语法原样显示；实际正文：${JSON.stringify(bodyText)}`
    ).not.toContain("**");

    wrapper.unmount();
  }, 15000);
});

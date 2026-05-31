import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderReasoningEffort, ProviderRegistry } from "@/types/provider";
import type { ChatMessage } from "@/types/runtime";

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
    }
  },
  emits: ["click"],
  template:
    '<button class="button-stub" type="button" :disabled="disabled" @click="$emit(\'click\')"><slot /></button>'
});

const SwitchStub = defineComponent({
  props: {
    modelValue: {
      type: Boolean,
      default: false
    }
  },
  emits: ["update:modelValue"],
  template:
    '<input class="switch-stub" type="checkbox" :checked="modelValue" @change="$emit(\'update:modelValue\', $event.target.checked)" />'
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
        Switch: SwitchStub,
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

    expect(wrapper.text()).toContain("tool chain exploded");
    expect((wrapper.get("textarea").element as HTMLTextAreaElement).disabled).toBe(false);
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
      phase: "ready",
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

  it("renders pending assistant content as inline streaming text before final markdown", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
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

    expect(wrapper.find(".assistant-streaming-content").exists()).toBe(true);
    expect(wrapper.find(".assistant-streaming-content").text()).toContain("**正在** 输出中");
    expect(wrapper.find(".markdown-stub").exists()).toBe(false);
  });

  it("switches from streaming text to final markdown when assistant completes", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: null,
      phase: "running",
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

  it("opens provider menu, selects another model, and closes afterwards", async () => {
    const providerStore = useProviderStore();
    const selectModelSpy = vi.spyOn(providerStore, "selectModel");

    const wrapper = mountWorkspace({
      registry: createMultiProviderRegistry()
    });
    await nextTick();

    const [providerTrigger] = wrapper.findAll("button.composer-select-trigger");
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

    const [providerTrigger, reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
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

  it("syncs reasoning menu selection and toggle persistence", async () => {
    window.localStorage.setItem("pony-agent.ui.show-reasoning-content", "true");

    const providerStore = useProviderStore();
    const setReasoningSpy = vi.spyOn(providerStore, "setCurrentReasoningEffort");

    const wrapper = mountWorkspace();
    await nextTick();

    const switchInput = wrapper.get("input.switch-stub");
    expect((switchInput.element as HTMLInputElement).checked).toBe(true);

    const [, reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
    await reasoningTrigger.trigger("click");
    await nextTick();

    const highButton = wrapper.findAll("button").find((node) => node.text().includes("high"));
    expect(highButton).toBeDefined();

    await highButton?.trigger("click");
    await nextTick();

    expect(setReasoningSpy).toHaveBeenCalledWith("high");
    expect(providerStore.currentReasoningEffort).toBe("high");
    expect(wrapper.text()).not.toContain("minimal");

    await switchInput.setValue(false);
    expect(window.localStorage.getItem("pony-agent.ui.show-reasoning-content")).toBe("false");
  });

  it("disables reasoning controls for models without reasoning support", async () => {
    const wrapper = mountWorkspace({
      registry: createProviderRegistry({ supportsReasoning: false })
    });
    await nextTick();

    const [, reasoningTrigger] = wrapper.findAll("button.composer-select-trigger");
    expect(reasoningTrigger.attributes("disabled")).toBeDefined();

    await reasoningTrigger.trigger("click");
    await nextTick();

    expect(wrapper.text()).not.toContain("minimal");
    expect(wrapper.text()).not.toContain("medium");
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

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.find(".assistant-streaming-content").classes()).toContain("text-stone-400");

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

  it("keeps tool calls and reasoning disclosures collapsed by default with semantic headings", async () => {
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
    expect(disclosures).toHaveLength(2);
    expect(disclosures.every((node) => node.attributes("open") === undefined)).toBe(true);

    const summaries = wrapper.findAll("summary");
    expect(summaries).toHaveLength(2);
    expect(summaries[0].text()).toContain("工具调用");
    expect(summaries[0].text()).toContain("1 项");
    expect(summaries[0].html()).toContain("lucide-wrench");
    expect(summaries[1].text()).toContain("思考过程");
    expect(summaries[1].html()).toContain("lucide-brain");
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
});

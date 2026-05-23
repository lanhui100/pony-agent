import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
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

const MarkdownRendererStub = defineComponent({
  props: {
    content: {
      type: String,
      default: ""
    }
  },
  template: '<div class="markdown-stub">{{ content }}</div>'
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

function mountWorkspace() {
  const providerStore = useProviderStore();
  providerStore.$patch({
    registry: createProviderRegistry(),
    selectedReasoningEffort: null
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

  it("在切换会话时展示横幅并禁用输入区", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      draftMessage: "继续推进",
      sessionOperation: "switching",
      sessionError: null,
      phase: "idle",
      error: null,
      messages: []
    });

    const wrapper = mountWorkspace();
    await nextTick();

    expect(wrapper.text()).toContain("正在切换对话…");
    expect((wrapper.get("textarea").element as HTMLTextAreaElement).disabled).toBe(true);
    expect((wrapper.get("button.button-stub").element as HTMLButtonElement).disabled).toBe(true);
  });

  it("在运行失败时展示错误横幅", async () => {
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
});

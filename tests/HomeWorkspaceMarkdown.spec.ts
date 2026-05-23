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

    const wrapper = mount(HomeWorkspace, {
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

    expect(wrapper.html()).toContain("<h1>标题</h1>");
    expect(wrapper.html()).toContain("<blockquote>");
    expect(wrapper.html()).toContain("<strong>加粗</strong>");
    expect(wrapper.html()).not.toContain("```md");
  });
});

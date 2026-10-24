import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { TooltipProvider } from "reka-ui";
import { renderMarkdown, isSimpleTextContent, autoCloseBoundaries, renderPartialMarkdown } from "@/lib/markdown";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
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
  template: '<button class="button-stub" type="button" :disabled="disabled" @click="$emit(\'click\')"><slot /></button>'
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

async function flushStreamingRender(waitMs = 120) {
  for (let i = 0; i < 3; i++) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, waitMs));
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

describe("Markdown Bold & Streaming Smooth Acceptance", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
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
  describe("1. Markdown Bold with Chinese Quotes and Punctuation", () => {
    it("renders bold text enclosing Chinese double quotes and multiple clauses correctly without leaking asterisks", async () => {
      // 场景 1: “建筑工地的砌砖工”，迅速进化为“工地的总监理”和“施工机器人的总指挥官” 等情况
      const input1 = "从**“建筑工地的砌砖工”**，迅速进化为**“工地的总监理”**和**“施工机器人的总指挥官”**。";
      const html1 = await renderMarkdown(input1);

      // 断言：所有的“...”均被正确包裹在 <strong> 中，且不能出现未解析的裸露 **
      expect(html1).not.toContain("**");
      expect(html1).toContain("<strong>“建筑工地的砌砖工”</strong>");
      expect(html1).toContain("<strong>“工地的总监理”</strong>");
      expect(html1).toContain("<strong>“施工机器人的总指挥官”</strong>");
    });

    it("renders whole phrase bolded including Chinese quotes and punctuation", async () => {
      const input2 = "**“建筑工地的砌砖工”，迅速进化为“工地的总监理”和“施工机器人的总指挥官”**";
      const html2 = await renderMarkdown(input2);

      expect(html2).not.toContain("**");
      expect(html2).toContain("<strong>“建筑工地的砌砖工”，迅速进化为“工地的总监理”和“施工机器人的总指挥官”</strong>");
    });

    it("identifies bold markdown with Chinese quotes as non-simple content", () => {
      expect(isSimpleTextContent("从“建筑工地的砌砖工”，迅速进化为“工地的总监理”")).toBe(true);
      expect(isSimpleTextContent("从**“建筑工地的砌砖工”**，迅速进化为**“工地的总监理”**")).toBe(false);
      expect(isSimpleTextContent("**“全句加粗”**")).toBe(false);
    });
  });

  describe("2. Streaming Route Lock Degradation Guard", () => {
    it("does not permanently lock streaming message into plain text raw path when bold appears later", async () => {
      // 场景：在 HomeWorkspace 组件中，用户问了一个问题，
      // 流式初始阶段只吐出普通文本 "当前桌面端进化过程："
      // 此时属于 isSimpleTextContent = true。
      // 随后流式追加 "**“建筑工地的砌砖工”**，迅速进化为**“工地的总监理”**"。
      // 验收断言：流式判定不能把该 message.id 永久锁死在纯文本分支，必须能在后续内容中识别出 markdown 并切换或渲染为 MarkdownRenderer。
      
      const providerStore = useProviderStore();
      providerStore.$patch({
        registry: createProviderRegistry(),
        selectedReasoningEffort: null
      });

      const runtimeStore = useRuntimeStore();
      runtimeStore.$patch({
        phase: "streaming_response",
        activeTurnId: "turn-stream-dynamic-markdown",
        isSubmitting: true,
        sessionOperation: null,
        error: null,
        messages: [
          {
            id: "user-stream-dynamic",
            turnId: "turn-stream-dynamic-markdown",
            role: "user",
            content: "测试动态升级",
            status: "done",
            tokenCount: null
          },
          {
            id: "assistant-stream-dynamic",
            turnId: "turn-stream-dynamic-markdown",
            role: "assistant",
            content: "当前桌面端进化过程：",
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
      await flushStreamingRender(100);

      // 阶段 1：此时纯文本
      // 阶段 2：流式追加加粗 Markdown
      const assistantMessage = runtimeStore.messages.find((m) => m.id === "assistant-stream-dynamic")!;
      assistantMessage.content += "迅速进化为**“工地的总监理”**";

      await nextTick();
      await flushStreamingRender(150);

      // 验收核心：不能因为流式开头是普通文本就被锁定，后面出现加粗时，界面中必须出现 Markdown 渲染器（.assistant-markdown 或 .markdown-body），而不是裸露的 ** 逐字 spans
      const streamingFlow = wrapper.find('[data-testid="assistant-streaming-flow"]');
      expect(streamingFlow.exists()).toBe(true);
      expect(streamingFlow.find(".markdown-body").exists()).toBe(true);
      expect(streamingFlow.text()).not.toContain("**“工地的总监理”**");

      wrapper.unmount();
    });
  });

  describe("3. Streaming Markdown Smooth Rendering without Jitter / Raw Exposure", () => {
    it("renders streaming partial markdown smoothly without raw unrendered-suffix jitter", async () => {
      // 验证 MarkdownRenderer 在流式推进时的表现：
      // 在流式 markdown 渲染中，当新的 token 逐步到达时，不应出现全量清空又替换的剧烈抖动，
      // 且不应在 DOM 中把 ** 作为 raw 字符暴露给用户
      const wrapper = mount(MarkdownRenderer, {
        props: {
          content: "这是",
          streaming: true,
          forceMarkdownStreaming: true,
          wrapperClass: "assistant-markdown"
        }
      });

      await nextTick();

      // 追加包含 **加粗** 的内容
      await wrapper.setProps({
        content: "这是**“建筑工地的砌砖工”**，迅速进化为**“工地的总监理”**"
      });

      // 等待渲染调度
      await new Promise<void>((resolve) => window.setTimeout(resolve, 60));
      await nextTick();

      // 断言：在任何流式瞬间，都不应该出现裸露的 **（如果作为 markdown 渲染中，应该被解析为 strong 或平滑显示）
      const html = wrapper.html();
      expect(html).not.toContain("**“工地的总监理”**");
      expect(html).toContain("<strong>“建筑工地的砌砖工”</strong>");
      expect(html).toContain("<strong>“工地的总监理”</strong>");
    });

    it("auto-closes unclosed bold markers without destructive DOM mutations", async () => {
      // 流式进行中，字符可能在 ** 内部中断：
      const partial = "正在演进为**施工机器人的总指挥";
      const closed = autoCloseBoundaries(partial);
      expect(closed).toBe("正在演进为**施工机器人的总指挥**");

      const html = await renderPartialMarkdown(partial);
      expect(html).toContain("<strong>施工机器人的总指挥</strong>");
      expect(html).not.toContain("**");
    });
  });
});

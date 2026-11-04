// wave-64 黑盒验收（独立测试者，红相先写）：F1 终态状态行可见性 + F2 消息区不得渲染「错误详情」。
//
// 契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 F1 / F2。
// 判据（断言契约目标行为）：
// 1. 终态状态行可见：`stopped` 与 `error` 时 `workspace-runtime-status-slot[data-visible="true"]`，
//    位置在消息区末尾（`workspace-runtime-status-row`）。
// 2. `stopped` 的状态行图标为 lucide Pause + `data-mode="stopped"`，label 为「已停止」。
// 3. `error` 的状态行承载真实错误文案（status bar），消息区**不含**「错误详情」文本/面板。
// 4. 失败轮已产出的正文照常渲染（不得被 error 分支抑制）。
// 5. 新一轮提交（isSubmitting 变 true）后终态状态行不再以终态模式呈现。
//
// 纪律（test-expert L2）：真实断言、AAA、禁止 skip / catch 吞异常 / 空断言。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";
import type { ChatMessage, TurnStreamEvent } from "@/types/runtime";

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

const CANCELLED_PLACEHOLDER = "本轮已停止。";
const STREAMED_TEXT = "这是已经流式输出出来的正文";
const REAL_ERROR = "tool chain exploded: EACCES on /tmp/secret";

type EventHandler = (event: { payload: unknown }) => void;

let rafTimers = new Map<number, ReturnType<typeof setTimeout>>();
let rafSeqId = 0;
let latestViewportEl: HTMLElement | null = null;

function createViewportElement() {
  const viewportEl = document.createElement("div");
  Object.defineProperty(viewportEl, "scrollHeight", { configurable: true, get: () => 1000 });
  Object.defineProperty(viewportEl, "clientHeight", { configurable: true, get: () => 400 });
  Object.defineProperty(viewportEl, "scrollTop", {
    configurable: true,
    get: () => 700,
    set: () => {}
  });
  Object.defineProperty(viewportEl, "scrollTo", {
    configurable: true,
    value: () => {}
  });
  latestViewportEl = viewportEl;
  return viewportEl;
}

const ScrollAreaStub = defineComponent({
  setup(_props, { slots, expose }) {
    const viewportEl = createViewportElement();
    expose({ viewportEl, scrollToBottom: () => {} });
    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

const MarkdownRendererStub = defineComponent({
  props: {
    content: { type: String, default: "" },
    streaming: { type: Boolean, default: false },
    wrapperClass: { type: String, default: "" },
    toneClass: { type: String, default: "" },
    forceMarkdownStreaming: { type: Boolean, default: false }
  },
  emits: ["render-complete"],
  template:
    '<div class="markdown-stub" :class="[wrapperClass, toneClass]" :streaming="streaming ? \'true\' : undefined">{{ content }}</div>'
});

const ButtonStub = defineComponent({
  props: {
    disabled: { type: Boolean, default: false },
    title: { type: String, default: "" }
  },
  emits: ["click"],
  template:
    '<button class="button-stub" type="button" :disabled="disabled" :title="title" @click="$emit(\'click\')"><slot /></button>'
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

function createMessage(partial: Partial<ChatMessage>): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? "turn-stop",
    role: partial.role ?? "user",
    content: partial.content ?? "",
    attachments: partial.attachments ?? [],
    status: partial.status ?? "done",
    tokenCount: partial.tokenCount ?? null,
    reasoningContent: partial.reasoningContent ?? null,
    modelName: partial.modelName ?? null,
    toolName: partial.toolName ?? null,
    detail: partial.detail ?? null,
    durationSeconds: partial.durationSeconds ?? null,
    errorDetail: partial.errorDetail ?? null
  };
}

function installEventCapture(): Map<string, EventHandler> {
  const handlers = new Map<string, EventHandler>();
  tauriMocks.mockSafeListen.mockImplementation(async (name: string, handler: unknown) => {
    handlers.set(name, handler as EventHandler);
    return () => {};
  });
  return handlers;
}

function seedStreamingTurn(store: ReturnType<typeof useRuntimeStore>, assistantContent: string) {
  store.$patch({
    sessionId: "session-stop",
    sessionOperation: null,
    isSubmitting: true,
    activeTurnId: "turn-stop",
    activeRunId: null,
    phase: "calling_model",
    error: null,
    sessionError: null,
    sessionNotice: null,
    messages: [
      createMessage({ id: "user-1", turnId: "turn-stop", role: "user", content: "请解释一下" }),
      // id 必须是 store 的 ensureAssistantMessage 键（`assistant-${turnId}`），
      // 否则终态事件会作用到**另一条**消息上，断言会假绿。
      createMessage({
        id: "assistant-turn-stop",
        turnId: "turn-stop",
        role: "assistant",
        content: assistantContent,
        status: "streaming"
      })
    ]
  });
}

function mountWorkspace() {
  const providerStore = useProviderStore();
  providerStore.$patch({ registry: createProviderRegistry(), selectedReasoningEffort: null });

  return mount(
    {
      render() {
        return h(TooltipProvider, null, { default: () => h(HomeWorkspace) });
      }
    },
    {
      global: {
        directives: {
          motion: {
            mounted() {},
            updated() {},
            beforeUpdate() {}
          }
        },
        stubs: {
          ScrollArea: ScrollAreaStub,
          MarkdownRenderer: MarkdownRendererStub,
          Button: ButtonStub,
          Transition: true,
          TransitionGroup: true
        }
      }
    }
  );
}

enableAutoUnmount(afterEach);

beforeEach(() => {
  setActivePinia(createPinia());
  window.localStorage.clear();
  tauriMocks.mockSafeInvoke.mockReset();
  tauriMocks.mockSafeListen.mockReset();
  tauriMocks.mockIsTauriAvailable.mockReset();
  tauriMocks.mockSafeInvoke.mockResolvedValue(null);
  tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.stubGlobal(
    "requestAnimationFrame",
    ((callback: FrameRequestCallback) => {
      const rafId = ++rafSeqId;
      const timer = setTimeout(() => {
        rafTimers.delete(rafId);
        callback(performance.now());
      }, 16);
      rafTimers.set(rafId, timer);
      return rafId;
    }) as typeof requestAnimationFrame
  );
  vi.stubGlobal(
    "cancelAnimationFrame",
    vi.fn((rafId: number) => {
      const timer = rafTimers.get(rafId);
      if (timer) {
        clearTimeout(timer);
        rafTimers.delete(rafId);
      }
    })
  );
  vi.stubGlobal(
    "ResizeObserver",
    class ResizeObserver {
      // 只用于「内容尺寸变化」回调，本文件的断言不依赖它。
      constructor(_callback: ResizeObserverCallback) {}
      observe() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver
  );
  latestViewportEl = null;
  rafTimers = new Map();
});

afterEach(async () => {
  await nextTick();
  await Promise.resolve();
  for (const timer of rafTimers.values()) {
    clearTimeout(timer);
  }
  rafTimers.clear();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("wave-64 F1 终态状态行 + F2 消息区不得渲染错误详情", () => {
  it("f1_stopped_terminal_status_row_is_visible_with_pause_icon", async () => {
    // Arrange：造一个流式轮 → 后端取消终态
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store, STREAMED_TEXT);
    await store.initializeTurnEvents();
    handlers.get("turn:cancelled")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "cancelled",
        phase: "cancelled",
        text: "用户终止，发送消息可继续。",
        error: "stopped_by_user",
        stopReason: "user"
      } satisfies TurnStreamEvent
    });

    // Act
    const wrapper = mountWorkspace();
    await nextTick();

    // Assert：终态状态行可见（stopped 不被「正文流式时隐藏」规则吞掉）
    const slot = wrapper.get('[data-testid="workspace-runtime-status-slot"]');
    expect(wrapper.find('[data-testid="workspace-runtime-status-row"]').exists()).toBe(true);
    expect(slot.attributes("data-visible"), "F1：stopped 终态状态行必须可见").toBe("true");

    // Assert：状态行承载 Pause + 已停止
    const bar = slot.get('[data-testid="agent-runtime-status-bar"]');
    expect(bar.attributes("data-mode")).toBe("stopped");
    expect(slot.get('[data-testid="agent-runtime-status-label"]').text()).toBe("已停止");
    const icon = slot.get('[data-testid="agent-runtime-status-icon"]');
    expect(icon.element.tagName.toLowerCase()).toBe("svg");
    expect(icon.classes().join(" ")).toContain("lucide-pause");

    // Assert：消息区不得另起「本轮已停止。」占位消息
    expect(wrapper.text()).not.toContain(CANCELLED_PLACEHOLDER);
  });

  it("f2_failed_turn_status_row_carries_error_and_message_area_has_no_error_panel", async () => {
    // Arrange：失败轮已产出正文
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store, STREAMED_TEXT);
    await store.initializeTurnEvents();
    handlers.get("turn:failed")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "failed",
        phase: "failed",
        error: REAL_ERROR
      } satisfies TurnStreamEvent
    });

    // Act
    const wrapper = mountWorkspace();
    await nextTick();
    const text = wrapper.text();

    // Assert：错误由状态行承载
    const slot = wrapper.get('[data-testid="workspace-runtime-status-slot"]');
    expect(slot.attributes("data-visible")).toBe("true");
    expect(slot.get('[data-testid="agent-runtime-status-bar"]').attributes("data-mode")).toBe("error");
    expect(slot.get('[data-testid="agent-runtime-status-label"]').text()).toBe(REAL_ERROR);

    // Assert：消息区不再渲染「错误详情」折叠面板
    expect(text, "F2：消息区不得再出现「错误详情」面板").not.toContain("错误详情");
    expect(wrapper.find('[data-testid="workspace-error-detail"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-error-copy-turn-stop"]').exists()).toBe(false);

    // Assert：失败轮已产出的正文照常渲染
    expect(text).toContain(STREAMED_TEXT);
  });

  it("f1_terminal_status_row_leaves_terminal_mode_after_new_submission", async () => {
    // Arrange：先落到 stopped 终态
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store, STREAMED_TEXT);
    await store.initializeTurnEvents();
    handlers.get("turn:cancelled")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "cancelled",
        phase: "cancelled",
        error: "stopped_by_user",
        stopReason: "user"
      } satisfies TurnStreamEvent
    });
    const wrapper = mountWorkspace();
    await nextTick();
    expect(
      wrapper.get('[data-testid="agent-runtime-status-bar"]').attributes("data-mode")
    ).toBe("stopped");

    // Act：新一轮提交（isSubmitting 变 true）
    store.$patch({
      isSubmitting: true,
      activeTurnId: "turn-next",
      phase: "connecting",
      messages: [
        ...store.messages,
        createMessage({ id: "user-2", turnId: "turn-next", role: "user", content: "再来一次" })
      ]
    });
    await nextTick();
    await nextTick();

    // Assert：终态状态行消失（不再以 stopped 呈现）
    const bars = wrapper.findAll('[data-testid="agent-runtime-status-bar"]');
    for (const bar of bars) {
      expect(bar.attributes("data-mode")).not.toBe("stopped");
    }
  });
});

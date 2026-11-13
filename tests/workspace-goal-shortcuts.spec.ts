/**
 * 桌面端 Goal 面板快捷键（Ctrl/Cmd+G）与快捷键帮助面板（Ctrl/Cmd+H）验收测试。
 *
 * 覆盖口径：
 * - Ctrl/Cmd+G：打开 / 切换右侧 Goal 面板（workspacePanels.activePanel === "goal"）。
 * - Ctrl/Cmd+H：打开 / 关闭快捷键帮助弹窗（Portal 渲染于 body，含「快捷键指南」）。
 * - `/goal`（无参数）执行成功后：自动展开右侧 Goal 面板。
 *
 * 说明：与 tests/slash-commands.spec.ts 同构，复用「真键盘事件 + 挂载 HomeWorkspace」的
 * 组件级验收范式；快捷键处理器与斜杠命令处理器均为用户真实可达交互路径。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useGoalStore } from "@/stores/goal";
import { useRuntimeStore } from "@/stores/runtime";
import { useWorkspacePanelsStore } from "@/stores/workspace-panels";

// ── Tauri IPC 替身：与 slash-commands.spec.ts 同口径 ──────────────────────────
const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

const tauriCoreMocks = vi.hoisted(() => ({
  mockInvoke: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tauri-apps/api/core")>();
  return { ...actual, invoke: tauriCoreMocks.mockInvoke };
});

// ── 渲染桩：只消除与快捷键无关的渲染依赖 ─────────────────────────────────────

const ScrollAreaStub = defineComponent({
  name: "ScrollAreaStub",
  setup(_props, { slots, expose }) {
    expose({
      viewportEl: document.createElement("div"),
      scrollToBottom: () => {}
    });
    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

const MarkdownRendererStub = defineComponent({
  name: "MarkdownRendererStub",
  props: {
    content: { type: String, default: "" },
    streaming: { type: Boolean, default: false }
  },
  emits: ["render-complete"],
  setup(props, { emit }) {
    return () => {
      window.setTimeout(
        () =>
          emit("render-complete", {
            contentLength: props.content.length,
            streaming: props.streaming
          }),
        0
      );
      return h("div", { class: "markdown-stub" }, props.content);
    };
  }
});

const ButtonStub = defineComponent({
  name: "ButtonStub",
  props: {
    disabled: { type: Boolean, default: false },
    title: { type: String, default: "" }
  },
  emits: ["click"],
  setup(props, { emit, slots }) {
    return () =>
      h(
        "button",
        {
          class: "button-stub",
          type: "button",
          disabled: props.disabled,
          title: props.title,
          onClick: () => emit("click")
        },
        slots.default ? slots.default() : []
      );
  }
});

function mountWorkspace(options: { attachTo?: HTMLElement } = {}) {
  return mount(
    {
      render() {
        return h(TooltipProvider, null, { default: () => h(HomeWorkspace) });
      }
    },
    {
      ...(options.attachTo ? { attachTo: options.attachTo } : {}),
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

/** 确定性推进异步：多轮 flush 微任务 + 宏任务 + nextTick。 */
async function flushAsyncWork(rounds = 4) {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.resolve();
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
    await nextTick();
  }
}

/** 模拟全局 window 键盘事件（与 HomeWorkspace.handleWindowKeydown 同路径）。 */
async function pressGlobalKey(init: KeyboardEventInit) {
  window.dispatchEvent(new KeyboardEvent("keydown", init));
  await flushAsyncWork();
}

const SESSION_ID = "goal-shortcuts-session-1";

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  document.body.innerHTML = "";
  setActivePinia(createPinia());
  tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  tauriMocks.mockSafeInvoke.mockResolvedValue(undefined);
  tauriCoreMocks.mockInvoke.mockResolvedValue(undefined);
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.stubGlobal(
    "ResizeObserver",
    class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver
  );
  vi.stubGlobal(
    "requestAnimationFrame",
    ((callback: FrameRequestCallback) => {
      return window.setTimeout(() => callback(performance.now()), 0);
    }) as typeof requestAnimationFrame
  );
  vi.stubGlobal("cancelAnimationFrame", ((id: number) => window.clearTimeout(id)) as typeof cancelAnimationFrame);
});

afterEach(async () => {
  document.body.innerHTML = "";
  await nextTick();
  await Promise.resolve();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

enableAutoUnmount(afterEach);

describe("Ctrl/Cmd+G：打开/切换 Goal 面板", () => {
  it("无目标时按 Ctrl+G 展开 goal 面板并给出引导 notice", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const panelsStore = useWorkspacePanelsStore();

    expect(panelsStore.activePanel).toBe("");
    mountWorkspace();
    await flushAsyncWork();

    await pressGlobalKey({ key: "g", ctrlKey: true });

    expect(panelsStore.activePanel).toBe("goal");
    expect(runtimeStore.sessionNotice).toContain("暂无目标");

    // 再次按 Ctrl+G 收起：与点击标题共用同一 toggle 语义。
    await pressGlobalKey({ key: "g", ctrlKey: true });
    expect(panelsStore.activePanel).toBe("");
  });

  it("有目标时按 Ctrl+G 展开 goal 面板且不重复产生引导 notice", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const goalStore = useGoalStore();
    goalStore.$patch({
      currentGoal: {
        id: "goal-1",
        session_id: SESSION_ID,
        objective: "完成迁移并跑通门禁",
        phase: "active",
        activation: "armed",
        revision: 1,
        roundsStarted: 0,
        maxGoalRounds: 10,
        blocked_reason: null
      }
    });
    const panelsStore = useWorkspacePanelsStore();

    mountWorkspace();
    await flushAsyncWork();

    await pressGlobalKey({ key: "g", ctrlKey: true });
    expect(panelsStore.activePanel).toBe("goal");
    expect(runtimeStore.sessionNotice ?? "").not.toContain("暂无目标");
  });
});

describe("Ctrl/Cmd+H：打开/关闭快捷键帮助面板", () => {
  it("按 Ctrl+H 弹出快捷键指南（Portal 渲染），再次按关闭", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });

    mountWorkspace();
    await flushAsyncWork();

    await pressGlobalKey({ key: "h", ctrlKey: true });
    const bodyText = document.body.textContent || "";
    expect(bodyText).toContain("快捷键指南");
    expect(bodyText).toContain("Ctrl");
    expect(bodyText).toContain("G");
    expect(bodyText).toContain("/goal");

    // 再次触发为关闭（toggle 语义）
    await pressGlobalKey({ key: "h", ctrlKey: true });
    await flushAsyncWork();
    expect(document.body.textContent || "").not.toContain("快捷键指南");
  });
});

/**
 * 可编辑区豁免回归（与 Ctrl+Z 同口径，Ctrl+H 为唯一例外）：
 * 焦点在 textarea（且它就是 document.activeElement）时，
 * - Ctrl/Cmd+G 不得劫持 goal 面板切换；
 * - Ctrl/Cmd+H 仍必须呼出快捷键帮助面板。
 */
describe("可编辑区焦点下的快捷键口径", () => {
  /** 把焦点放进 composer textarea，并从该元素冒泡派发 keydown（window 监听同路径）。 */
  async function focusComposerAndPress(
    wrapper: ReturnType<typeof mountWorkspace>,
    init: KeyboardEventInit
  ) {
    const textarea = wrapper.get("textarea");
    const el = textarea.element as HTMLTextAreaElement;
    el.focus();
    expect(document.activeElement).toBe(el);
    el.dispatchEvent(new KeyboardEvent("keydown", { ...init, bubbles: true }));
    await flushAsyncWork();
    return el;
  }

  it("焦点在 textarea 时按 Ctrl+G 不劫持 goal 面板", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const panelsStore = useWorkspacePanelsStore();

    const wrapper = mountWorkspace({ attachTo: document.body });
    await flushAsyncWork();

    await focusComposerAndPress(wrapper, { key: "g", ctrlKey: true });

    expect(panelsStore.activePanel).toBe("");
    expect(runtimeStore.sessionNotice ?? "").not.toContain("暂无目标");
  });

  it("焦点在 textarea 时按 Ctrl+H 仍呼出快捷键帮助面板", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });

    const wrapper = mountWorkspace({ attachTo: document.body });
    await flushAsyncWork();

    await focusComposerAndPress(wrapper, { key: "h", ctrlKey: true });

    const bodyText = document.body.textContent || "";
    expect(bodyText).toContain("快捷键指南");
    expect(bodyText).toContain("快捷键帮助面板");
  });
});

describe("/goal 斜杠命令：执行后自动展开 Goal 面板", () => {
  it("/goal（无参数）命中后 activePanel 切到 goal", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const panelsStore = useWorkspacePanelsStore();

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "goal_command") {
        return {
          action: "show",
          message: "当前会话暂无目标",
          goal: null
        };
      }
      return undefined;
    });

    const wrapper = mountWorkspace();
    await flushAsyncWork();

    runtimeStore.draftMessage = "/goal";
    await nextTick();
    const textarea = wrapper.get("textarea");
    await textarea.trigger("keydown", { key: "Enter", shiftKey: false });
    await flushAsyncWork();

    expect(panelsStore.activePanel).toBe("goal");
    expect(runtimeStore.draftMessage).toBe("");
  });
});
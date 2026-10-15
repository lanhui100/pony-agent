/**
 * Wave-11 验收契约（组件级）— 消息流下方状态条 PA-STATUS-FLOW
 *
 * 权威契约：`.dev-team/contracts/wave-11-status-flow.md` §1（M1–M6）/ §2（DOM 与行为契约）
 * UI-NFR：`.dev-team/nfr-baseline-ui-wave11.json`
 *
 * 本文件为 vitest + jsdom 组件级契约；jsdom 不进行真实布局计算，
 * 因此「物理高度极差 / 逐帧位移 / 真实动画计算值 / Console Error」等物理量
 * 由 `tests/e2e/status-flow-stability.spec.ts` 在真实 Chromium 中断言（M1–M5）。
 * 本文件断言的是可在 DOM 层机械判定的结构契约：槽位常驻、槽位归属、
 * data-visible 语义、单行 nowrap、状态无关的高度类、错误态 AlertCircle、
 * 以及 reduced-motion 的 CSS 契约。
 */
import { mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { TooltipProvider } from "reka-ui";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import statusBarSource from "@/components/chat/AgentRuntimeStatusBar.vue?raw";
import { useRuntimeStore } from "@/stores/runtime";

const SLOT = '[data-testid="workspace-runtime-status-slot"]';
const BAR = '[data-testid="agent-runtime-status-bar"]';
const LABEL = '[data-testid="agent-runtime-status-label"]';
const ICON = '[data-testid="agent-runtime-status-icon"]';
const CONTENT_COLUMN = '[data-testid="workspace-content-column"]';
const COMPOSER_SHELL = '[data-testid="workspace-composer-shell"]';

const TURN_ID = "turn-wave11-contract";

type RuntimeStore = ReturnType<typeof useRuntimeStore>;

let activeWrapper: VueWrapper | null = null;

function setupWorkspace(): { store: RuntimeStore; wrapper: VueWrapper } {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  const wrapper = mount(
    {
      components: { HomeWorkspace, TooltipProvider },
      template: `<TooltipProvider><HomeWorkspace /></TooltipProvider>`
    },
    { global: { plugins: [pinia] }, attachTo: document.body }
  );
  activeWrapper = wrapper;
  return { store, wrapper };
}

/** 等待 Vue 渲染队列与 presentation ticker 的宏任务排空（不使用任意时长 sleep）。 */
async function flush(times = 4) {
  for (let i = 0; i < times; i += 1) {
    await nextTick();
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
  await nextTick();
}

function patchBase(store: RuntimeStore, extra: Record<string, unknown> = {}) {
  store.$patch({
    sessionId: "sess-wave11",
    sessionOperation: null,
    sessionError: null,
    error: null,
    isSubmitting: false,
    activeTurnId: null,
    activeTurnStartedAtMs: null,
    phase: "idle",
    toolActivities: [],
    messages: [],
    ...extra
  });
}

function patchStreaming(store: RuntimeStore, content = "这是已经流式输出的第一段可见内容。") {
  patchBase(store, {
    isSubmitting: true,
    activeTurnId: TURN_ID,
    activeTurnStartedAtMs: Date.now(),
    phase: "calling_model",
    messages: [
      { id: "w11-user", turnId: TURN_ID, role: "user", content: "请开始回答" },
      { id: "w11-assistant", turnId: TURN_ID, role: "assistant", content, status: "pending" }
    ]
  });
}

function patchThinking(store: RuntimeStore) {
  patchBase(store, {
    isSubmitting: true,
    activeTurnId: TURN_ID,
    activeTurnStartedAtMs: Date.now(),
    phase: "calling_model",
    messages: [
      { id: "w11-user", turnId: TURN_ID, role: "user", content: "请开始回答" },
      { id: "w11-assistant", turnId: TURN_ID, role: "assistant", content: "", status: "pending" }
    ]
  });
}

function patchToolRunning(store: RuntimeStore) {
  patchBase(store, {
    isSubmitting: true,
    activeTurnId: TURN_ID,
    activeTurnStartedAtMs: Date.now(),
    phase: "calling_tool",
    toolActivities: [
      { id: "w11-act", turnId: TURN_ID, name: "read_file", canonicalToolName: "Read", displayNameZh: "读取文件", status: "running" }
    ],
    messages: [
      { id: "w11-user", turnId: TURN_ID, role: "user", content: "读取文件" },
      { id: "w11-assistant", turnId: TURN_ID, role: "assistant", content: "", status: "pending" }
    ]
  });
}

function patchError(store: RuntimeStore) {
  patchBase(store, {
    isSubmitting: false,
    error: "响应超时，已停止等待",
    messages: [{ id: "w11-user", turnId: TURN_ID, role: "user", content: "长时任务" }]
  });
}

function patchSessionOperation(store: RuntimeStore) {
  patchBase(store, { sessionOperation: "switching" });
}

/** 从组件 <style> 源码中提取 @media (prefers-reduced-motion: reduce) 块体。 */
function extractReducedMotionBlock(css: string): string | null {
  const marker = "prefers-reduced-motion";
  const markerIndex = css.indexOf(marker);
  if (markerIndex < 0) return null;
  const openIndex = css.indexOf("{", markerIndex);
  if (openIndex < 0) return null;
  let depth = 0;
  for (let i = openIndex; i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    else if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(openIndex + 1, i);
    }
  }
  return null;
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  document.body.innerHTML = "";
});

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
});

describe("Wave-11 状态条契约（组件级）", () => {
  it("契约 §2.1/§2.2 槽位常驻：agentRuntimeStatus 非空期间始终存在于 DOM（含流式隐藏期）", async () => {
    const { store, wrapper } = setupWorkspace();

    patchStreaming(store);
    await flush();
    const slotDuringStreaming = wrapper.find(SLOT);
    expect(slotDuringStreaming.exists()).toBe(true);

    const streamingElement = slotDuringStreaming.element;

    patchToolRunning(store);
    await flush();
    const slotDuringTool = wrapper.find(SLOT);
    expect(slotDuringTool.exists()).toBe(true);

    // 常驻 = 同一个 DOM 节点被复用，而非卸载后重建（消除挂载/卸载引发的布局突变）
    expect(slotDuringTool.element).toBe(streamingElement);
    expect(document.body.contains(streamingElement)).toBe(true);
  });

  it("契约 §2.1 槽位归属：状态条不再位于滚动内容列内，而是归属 composer 区域", async () => {
    const { store, wrapper } = setupWorkspace();

    patchStreaming(store);
    await flush();

    const contentColumn = wrapper.find(CONTENT_COLUMN);
    const composerShell = wrapper.find(COMPOSER_SHELL);
    const slot = wrapper.find(SLOT);

    // 前置条件：内容列与 composer 均必须真实渲染，否则本断言无意义
    expect(contentColumn.exists()).toBe(true);
    expect(composerShell.exists()).toBe(true);
    expect(slot.exists()).toBe(true);

    expect(contentColumn.element.contains(slot.element)).toBe(false);
    expect(composerShell.element.contains(slot.element)).toBe(true);
  });

  it("契约 §2.4 data-visible 语义：流式有可见内容→false；思考中/工具执行/错误/会话操作→true", async () => {
    const { store, wrapper } = setupWorkspace();

    patchStreaming(store);
    await flush();
    expect(wrapper.find(SLOT).attributes("data-visible")).toBe("false");

    patchThinking(store);
    await flush();
    expect(wrapper.find(SLOT).attributes("data-visible")).toBe("true");

    patchToolRunning(store);
    await flush();
    expect(wrapper.find(SLOT).attributes("data-visible")).toBe("true");

    patchError(store);
    await flush();
    expect(wrapper.find(SLOT).attributes("data-visible")).toBe("true");

    patchSessionOperation(store);
    await flush();
    expect(wrapper.find(SLOT).attributes("data-visible")).toBe("true");
  });

  // L3-A 审查给出的**可达反例**：会话操作 / 错误态与「末个 turn 的 assistant 为 pending
  // 且有可见流式内容」同时成立时，槽位必须仍然可见（契约 §2.4：只有「无会话操作、无错误、
  // 无工具活动」的纯流式窗口才允许隐藏）。既有 patchSessionOperation/patchError 用 messages:[]
  // 覆盖不到该组合（`runtime.ts` 会话操作窗口 + 缓存 pending assistant）。
  it("契约 §2.4 反例回归：会话操作/错误态 + 末个 turn 仍有 pending 可见内容 → 槽位必须可见", async () => {
    const { store, wrapper } = setupWorkspace();

    patchBase(store, {
      sessionOperation: "initializing",
      isSubmitting: false,
      messages: [
        { id: "w11-so-user", turnId: TURN_ID, role: "user", content: "初始化前的问题" },
        {
          id: "w11-so-assistant",
          turnId: TURN_ID,
          role: "assistant",
          content: "已流式输出的可见内容",
          status: "pending"
        }
      ]
    });
    await flush();
    expect(
      wrapper.find(SLOT).attributes("data-visible"),
      "sessionOperation=initializing 时槽位必须可见"
    ).toBe("true");

    patchBase(store, {
      isSubmitting: false,
      error: "响应超时，已停止等待",
      messages: [
        { id: "w11-err-user", turnId: TURN_ID, role: "user", content: "超时前的问题" },
        {
          id: "w11-err-assistant",
          turnId: TURN_ID,
          role: "assistant",
          content: "已流式输出的可见内容",
          status: "pending"
        }
      ]
    });
    await flush();
    expect(
      wrapper.find(SLOT).attributes("data-visible"),
      "错误态下槽位必须可见"
    ).toBe("true");
  });

  it("契约 §2.3 隐藏语义：data-visible=false 时 aria-hidden=true 且 pointer-events 为 none", async () => {
    const { store, wrapper } = setupWorkspace();

    patchStreaming(store);
    await flush();

    const slot = wrapper.find(SLOT);
    expect(slot.attributes("data-visible")).toBe("false");
    expect(slot.attributes("aria-hidden")).toBe("true");

    const pointerEvents = (slot.element as HTMLElement).style.pointerEvents
      || window.getComputedStyle(slot.element).pointerEvents;
    expect(pointerEvents).toBe("none");

    patchToolRunning(store);
    await flush();
    const visibleSlot = wrapper.find(SLOT);
    expect(visibleSlot.attributes("data-visible")).toBe("true");
    expect(visibleSlot.attributes("aria-hidden")).not.toBe("true");
  });

  it("契约 §2.5 单行不换行：状态条文本链路上存在 nowrap/截断声明且标签文本不含换行", async () => {
    const { store, wrapper } = setupWorkspace();

    patchStreaming(store);
    await flush();

    const nowrapCarriers = [SLOT, BAR, LABEL]
      .map((selector) => wrapper.find(selector))
      .filter((node) => node.exists())
      .filter((node) =>
        node.classes().some((name) => name === "whitespace-nowrap" || name === "truncate")
      );

    expect(nowrapCarriers.length).toBeGreaterThan(0);
    expect(wrapper.find(LABEL).text()).not.toMatch(/[\r\n]/);
  });

  it("契约 §2.5 高度恒定：状态条根节点高度类不随状态变化（物理高度极差见 e2e）", async () => {
    const { store, wrapper } = setupWorkspace();

    const readHeightClasses = () =>
      wrapper
        .find(BAR)
        .classes()
        .filter((name) => /^(min-h-|max-h-|h-)/.test(name))
        .sort()
        .join("|");

    const scenarios: Array<() => void> = [
      () => patchThinking(store),
      () => patchStreaming(store),
      () => patchToolRunning(store),
      () => patchError(store),
      () => patchSessionOperation(store)
    ];

    const observed: string[] = [];
    for (const apply of scenarios) {
      apply();
      await flush();
      expect(wrapper.find(BAR).exists()).toBe(true);
      observed.push(readHeightClasses());
    }

    expect(observed.every((entry) => entry === observed[0])).toBe(true);
    expect(observed[0].length).toBeGreaterThan(0);
  });

  it("契约 §2.8 错误态：保留 AlertCircle 图标且不带 orbit 动效类", async () => {
    const { store, wrapper } = setupWorkspace();

    patchError(store);
    await flush();

    const icon = wrapper.find(ICON);
    expect(icon.exists()).toBe(true);
    expect(icon.classes().some((name) => /lucide-(alert-circle|circle-alert)/.test(name))).toBe(true);
    expect(icon.classes()).not.toContain("runtime-status-orbit");
    expect(wrapper.find(LABEL).classes()).toContain("text-rose-600");
  });

  it("契约 §2.6/§3 reduced-motion：样式表内存在 prefers-reduced-motion 覆盖且对 orbit 动效置 animation:none", () => {
    const block = extractReducedMotionBlock(statusBarSource);
    expect(block).not.toBeNull();
    const normalized = (block ?? "").replace(/\s+/g, " ");
    expect(normalized).toMatch(/animation(-name)?:\s*none/);
    expect(normalized).toContain("runtime-status-orbit");
  });
});

/**
 * 桌面端斜杠命令验收测试（契约 C-8/C-9；清单 T6~T9）。
 *
 * 契约真值源：`.dev-team/contract-matrix-slash-commands.md` §2。
 * 本文件由 Test Agent 编写并冻结；实现侧（exec-web）只读，不得就地改断言。
 *
 * 覆盖口径：
 * - T6 `parseSlashCommand` 的命中/不命中矩阵（纯函数，直接调用目标模块）。
 * - T7 `/plan` 命中时 `runtimeStore.submitTurn` 不被调用且草稿被清空。
 * - T8 `/compact` 触发 `compact_session`（`{ sessionId, force: true }`）并写入归因 notice 文案；
 *      `compacted:false` 时文案必须与之不同（不得谎报已压缩）。
 * - T9 未知 `/xxx` 仍走普通提交路径（`submitTurn` 被调用）。
 *
 * 说明（为什么 T7~T9 是「挂载 HomeWorkspace + 真键盘 Enter」的组件级用例）：
 * 契约 C-8 约束的拦截点是 **HomeWorkspace 的提交流程**，而不是某个私有函数；只有从用户可见的
 * 输入框按 Enter 提交才能证明「命中命令不提交 turn、草稿被清空」。用桩替换子组件
 * （ScrollArea/MarkdownRenderer/Button）只消除渲染噪音，不改变被测行为。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { DESKTOP_SLASH_COMMANDS, parseSlashCommand } from "@/lib/chat/slash-commands";
import { usePlanStore } from "@/stores/plan";
import { useRuntimeStore } from "@/stores/runtime";
import { useWorkspacePanelsStore } from "@/stores/workspace-panels";

// ── Tauri IPC 替身：同时覆盖仓库约定的 `@/lib/tauri.safeInvoke` 与底层 `invoke`，
//    避免用例把「实现选了哪个封装」当成通过条件（契约只约束命令名与参数）。
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

// ── 渲染桩：只消除与斜杠命令无关的渲染依赖（不改变提交路径） ──────────────────────────

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
    // 与真实组件一致：内容渲染完成后回发 render-complete（部分父级流程依赖该事件）。
    return () => {
      window.setTimeout(() => emit("render-complete", {
        contentLength: props.content.length,
        streaming: props.streaming
      }), 0);
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

function mountWorkspace() {
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

/** 确定性推进异步（无固定 sleep 阈值、无盲等）：多轮 flush 微任务 + 宏任务 + nextTick。 */
async function flushAsyncWork(rounds = 4) {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.resolve();
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
    await nextTick();
  }
}

/**
 * 用真实交互提交草稿：写入 store 草稿 → 在输入框按 Enter（与用户路径一致，
 * 且走 HomeWorkspace 的 `handleComposerKeydown`）。
 */
async function submitDraftByEnter(wrapper: ReturnType<typeof mount>, text: string) {
  const runtimeStore = useRuntimeStore();
  runtimeStore.draftMessage = text;
  await nextTick();
  const textarea = wrapper.get("textarea");
  await textarea.trigger("keydown", { key: "Enter", shiftKey: false });
  await flushAsyncWork();
}

/** 收集本次用例中所有 `compact_session` 调用参数（合并 safeInvoke / invoke 两条封装路径）。 */
function compactSessionCalls(): Array<Record<string, unknown> | undefined> {
  const calls: Array<Record<string, unknown> | undefined> = [];
  for (const [command, args] of tauriMocks.mockSafeInvoke.mock.calls) {
    if (command === "compact_session") {
      calls.push(args as Record<string, unknown> | undefined);
    }
  }
  for (const [command, args] of tauriCoreMocks.mockInvoke.mock.calls) {
    if (command === "compact_session") {
      calls.push(args as Record<string, unknown> | undefined);
    }
  }
  return calls;
}

const SESSION_ID = "slash-compact-session-1";

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  setActivePinia(createPinia());
  tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  // 默认 IPC 结果：未显式编排的命令返回 undefined（调用被记录但不产生副作用）。
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
  vi.stubGlobal("requestAnimationFrame", ((callback: FrameRequestCallback) => {
    return window.setTimeout(() => callback(performance.now()), 0);
  }) as typeof requestAnimationFrame);
  vi.stubGlobal("cancelAnimationFrame", ((id: number) => window.clearTimeout(id)) as typeof cancelAnimationFrame);
});

afterEach(async () => {
  await nextTick();
  await Promise.resolve();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

enableAutoUnmount(afterEach);

// ── T6：parseSlashCommand 命中/不命中矩阵 ────────────────────────────────────────────

describe("T6 parseSlashCommand 命中矩阵", () => {
  it("命中：精确命令、大小写不敏感、首尾空白、尾随参数", () => {
    // Arrange + Act + Assert（每行都是独立的真实断言）
    expect(parseSlashCommand("/plan")).toBe("plan");
    expect(parseSlashCommand("/PLAN")).toBe("plan");
    expect(parseSlashCommand("  /plan  ")).toBe("plan");
    expect(parseSlashCommand("/tasks")).toBe("tasks");
    expect(parseSlashCommand("/TASKS")).toBe("tasks");
    expect(parseSlashCommand("/compact")).toBe("compact");
    expect(parseSlashCommand("\t/Compact  \n")).toBe("compact");
    // 契约 C-8：「允许尾随空白/参数」——命令语义优先于参数。
    expect(parseSlashCommand("/compact now")).toBe("compact");
    expect(parseSlashCommand("/plan 今天做什么")).toBe("plan");
  });

  it("不命中：路径式输入、前缀相同词、裸斜杠、空串、无斜杠", () => {
    // `/etc/hosts` 类输入必须保持「按普通消息发送」的既有行为。
    expect(parseSlashCommand("/etc/hosts 是什么")).toBeNull();
    expect(parseSlashCommand("/etc/hosts")).toBeNull();
    expect(parseSlashCommand("/tasks2")).toBeNull();
    expect(parseSlashCommand("/planner")).toBeNull();
    expect(parseSlashCommand("/unknown")).toBeNull();
    expect(parseSlashCommand("/")).toBeNull();
    expect(parseSlashCommand("")).toBeNull();
    expect(parseSlashCommand("   ")).toBeNull();
    expect(parseSlashCommand("plan")).toBeNull();
    expect(parseSlashCommand("hello /plan")).toBeNull();
  });

  it("命令元数据：恰好三条，usage 与 name 一一对应，描述非空", () => {
    const names = DESKTOP_SLASH_COMMANDS.map((item) => item.name);
    expect(names).toEqual(["plan", "tasks", "compact"]);
    for (const item of DESKTOP_SLASH_COMMANDS) {
      expect(item.usage).toBe(`/${item.name}`);
      expect(typeof item.description).toBe("string");
      expect(item.description.trim().length).toBeGreaterThan(0);
      // usage 自身必须能被解析回同名命令（元数据与解析口径同源）。
      expect(parseSlashCommand(item.usage)).toBe(item.name);
    }
  });
});

// ── T7：命中命令不得提交 turn，且草稿被清空 ─────────────────────────────────────────

describe("T7 命中斜杠命令时的提交拦截", () => {
  it("/plan 命中：submitTurn 不被调用且草稿被清空", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/plan");

    expect(submitSpy).not.toHaveBeenCalled();
    expect(runtimeStore.draftMessage).toBe("");
  });

  it("/tasks 命中：submitTurn 不被调用且草稿被清空", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/TASKS");

    expect(submitSpy).not.toHaveBeenCalled();
    expect(runtimeStore.draftMessage).toBe("");
  });

  // ── G-1（Lead 裁决追加，ADVISORY 缺口）：上面两条用例的 store 为空，`/plan` 永远走
  //    「当前会话暂无计划」分支，`openPanel("plan")` 从未被执行过（审查实测：删掉该行仍全绿）。
  //    下面两条用具真实计划数据把 openPanel 拉到覆盖面上，并按契约 C-8「切换」语义锁住收起行为。

  /** mount & flush 之后再播种计划数据：避免 onMounted 的 plan_list/todo_list 异步加载清空种子。 */
  async function seedPlanData() {
    const planStore = usePlanStore();
    planStore.$patch({
      todos: [{ content: "接入斜杠命令", status: "pending" }]
    });
    await nextTick();
    // 前置条件（真实断言）：确有计划数据，否则 /plan 走 notice 分支、openPanel 不可达。
    expect(planStore.todos.length).toBeGreaterThan(0);
  }

  it("/plan 命中且计划面板有数据：真实切换到「计划」面板（openPanel 被执行）", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    const panelsStore = useWorkspacePanelsStore();

    const wrapper = mountWorkspace();
    await nextTick();
    await flushAsyncWork();
    await seedPlanData();

    await submitDraftByEnter(wrapper, "/plan");

    expect(panelsStore.activePanel).toBe("plan");
    expect(submitSpy).not.toHaveBeenCalled();
    expect(runtimeStore.draftMessage).toBe("");
  });

  it("/plan 第二次执行按「切换」语义收起面板（契约 C-8 用词是切换，非幂等展开）", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    const panelsStore = useWorkspacePanelsStore();

    const wrapper = mountWorkspace();
    await nextTick();
    await flushAsyncWork();
    await seedPlanData();

    await submitDraftByEnter(wrapper, "/plan");
    expect(panelsStore.activePanel).toBe("plan");

    await submitDraftByEnter(wrapper, "/plan");
    expect(panelsStore.activePanel).toBe("");
  });
});

// ── T8：/compact 走真实压缩 IPC 并写入归因 notice ────────────────────────────────────

describe("T8 /compact 的 IPC 与反馈文案", () => {
  it("触发 compact_session(sessionId, force:true) 并在 notice 中给出真实计数", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "compact_session") {
        return {
          compacted: true,
          reason: "compacted",
          compressedCount: 7,
          keptCount: 5,
          beforeTokens: 9000,
          afterTokens: 3000
        };
      }
      return undefined;
    });

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/compact");

    // 契约 C-8：/compact 是控制面动作，不得退化成普通消息提交。
    expect(submitSpy).not.toHaveBeenCalled();
    expect(runtimeStore.draftMessage).toBe("");

    // 契约 C-8：invoke("compact_session", { sessionId, force: true })（force 必须为 true）。
    const calls = compactSessionCalls();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({ sessionId: SESSION_ID, force: true });

    // 契约 C-8/C-9：成功文案必须含真实 compressedCount / keptCount，且走 notice 通道。
    const notice = runtimeStore.sessionNotice;
    expect(typeof notice).toBe("string");
    expect(notice).not.toBeNull();
    expect(notice).toContain("7");
    expect(notice).toContain("5");
    expect(notice).toMatch(/压缩/);
    // 不得把成功文案塞进错误通道（语义错误）。
    expect(runtimeStore.sessionError ?? null).toBeNull();
  });

  it("compacted:false 时文案与成功文案不同，且不得谎报已压缩", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "compact_session") {
        return {
          compacted: false,
          reason: "history_too_short",
          compressedCount: 0,
          keptCount: 24,
          beforeTokens: 100,
          afterTokens: 100
        };
      }
      return undefined;
    });

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/compact");

    const calls = compactSessionCalls();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({ sessionId: SESSION_ID, force: true });

    const notice = runtimeStore.sessionNotice;
    expect(typeof notice).toBe("string");
    expect(notice).not.toBeNull();
    expect(String(notice).trim().length).toBeGreaterThan(0);
    // 成功但未压缩必须如实说明（契约 C-7 的「不得谎报已压缩」同源要求）。
    expect(notice).not.toContain("已压缩上下文");
    expect(notice).toMatch(/历史过短|无需压缩/);
  });
});

// ── T9：未知 /xxx 仍走普通提交路径 ───────────────────────────────────────────────────

describe("T9 未知斜杠输入走普通提交", () => {
  it("/unknown 未被识别：submitTurn 被调用", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/unknowncmd 你好");

    expect(submitSpy).toHaveBeenCalledTimes(1);
    expect(compactSessionCalls()).toHaveLength(0);
  });

  it("/etc/hosts 这类路径式输入：submitTurn 被调用", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [] });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/etc/hosts 是什么");

    expect(submitSpy).toHaveBeenCalledTimes(1);
    expect(compactSessionCalls()).toHaveLength(0);
  });
});

// ── W-1/W-2/W-3：运行中（isSubmitting）斜杠命令必须被统一拒绝 ─────────────────────────
//
// Lead 裁决追加（回归护栏，对应实现侧刚修的缺陷：Enter 路径曾缺 isSubmitting 守卫）。
// 冻结口径：
// - 守卫只有一份（`executeSlashCommand()` 顶部），Enter 与发送按钮共用；
// - 运行中被拒时**不清空草稿**，并给出统一提示；
// - 三条命令（/compact、/plan、/tasks）同权拦截。

const BUSY_NOTICE = "当前正在运行，请等待本轮结束后再执行斜杠命令";

describe("W 系列：运行中（isSubmitting）斜杠命令统一拒绝", () => {
  it("W-1 运行中 Enter 命中 /compact：不触发 IPC、notice 为统一提示、草稿保留", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [], isSubmitting: true });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/compact");

    // 运行中不得执行真实压缩（否则会在本轮进行中改写会话历史）。
    expect(compactSessionCalls()).toHaveLength(0);
    expect(runtimeStore.sessionNotice).toBe(BUSY_NOTICE);
    // 被拒时不清空草稿（用户运行结束后可直接重试）。
    expect(runtimeStore.draftMessage).toBe("/compact");
    expect(submitSpy).not.toHaveBeenCalled();
  });

  it("W-2 运行中点击发送按钮命中 /plan：面板不切换、notice 与 Enter 路径同源", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [], isSubmitting: true });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    const panelsStore = useWorkspacePanelsStore();

    const wrapper = mountWorkspace();
    await nextTick();
    runtimeStore.draftMessage = "/plan";
    await nextTick();

    // 真实点击「发送」按钮（运行中且有草稿时它仍是 workspace-submit-action，非停止键）。
    const submitButton = wrapper.get('[data-testid="workspace-submit-action"]');
    await submitButton.trigger("click");
    await flushAsyncWork();

    expect(panelsStore.activePanel).toBe("");
    // 锁住「Enter 与发送按钮两处守卫不得漂移」：文案必须完全一致。
    expect(runtimeStore.sessionNotice).toBe(BUSY_NOTICE);
    expect(runtimeStore.draftMessage).toBe("/plan");
    expect(submitSpy).not.toHaveBeenCalled();
  });

  it("W-3 运行中 /tasks 同样被拒（三条命令同权）", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: SESSION_ID, messages: [], isSubmitting: true });
    const submitSpy = vi.spyOn(runtimeStore, "submitTurn").mockResolvedValue(true);
    const panelsStore = useWorkspacePanelsStore();

    const wrapper = mountWorkspace();
    await nextTick();
    await submitDraftByEnter(wrapper, "/tasks");

    expect(panelsStore.activePanel).toBe("");
    expect(runtimeStore.sessionNotice).toBe(BUSY_NOTICE);
    expect(runtimeStore.draftMessage).toBe("/tasks");
    expect(submitSpy).not.toHaveBeenCalled();
    expect(compactSessionCalls()).toHaveLength(0);
  });
});

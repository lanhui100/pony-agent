/**
 * Wave-17 · task-5 · L2-AT 对抗破坏用例（Adversarial QA）
 *
 * 契约（只读）：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 **C4**
 *   - C4-1 事件对象边界穿透：`event === undefined` / `isComposing === undefined` /
 *     `keyCode === 0` / 传入 `MouseEvent` 时守卫不得抛异常，且非组合态仍提交。
 *   - C4-2 组合序列畸形：`compositionstart → keydown(Enter,isComposing=true) → compositionend`
 *     交错序列下，仅非组合态回车产生提交。
 *   - C4-3 确定性并发：固定迭代 200 次交替注入组合/非组合回车，提交计数**精确等于**
 *     非组合态次数（无 sleep、无概率等待、无随机）。
 *
 * 零假测试律：本文件每个 `it` 均含真实状态突变断言（store 调用次数 / emit 载荷 /
 * 守卫返回值），无 `skip` / `todo` / 空断言 / 吞异常。
 *
 * 确定性来源（C4-3）：迭代次数硬编码 200；组合/非组合由 `index % 2` 纯函数决定；
 * 事件同步派发（`dispatchEvent` → Vue handler 同步执行至首个 `await`），提交计数在
 * 派发返回前即已落定；全程零 timer、零 random、零 sleep。
 *
 * 探针结论（写测试前已实测，见 `.dev-team/wave17-red-receipts-l2at.md`）：
 * jsdom 26.1.0 的 `KeyboardEvent` 构造器**真实透传** `isComposing` 与 `keyCode`
 * （`@vue/test-utils` 的 `trigger("keydown", {...})` 同样透传），故无需 defineProperty
 * 兜底；但仍保留一条 `Object.defineProperty` 显式定义 `isComposing === undefined` 的
 * 敌意事件形状用例（C4-1）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick, toRaw } from "vue";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import SessionRow from "@/components/HomeSessionSidebarRow.vue";
import DropdownMenu from "@/components/ui/DropdownMenu.vue";
import type { DropdownMenuItemSpec } from "@/components/ui/DropdownMenu.vue";
import { isImeComposingEvent } from "@/lib/ime";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import { useAskStore } from "@/stores/ask";
import { useRuntimeStore } from "@/stores/runtime";
import { useUpdateStore } from "@/stores/update";
import type { PendingAsk } from "@/types/ask-plan";
import type { SessionOverview } from "@/types/runtime";

// ────────────────────────────────────────────────────────────────────────────
// 共享夹具
// ────────────────────────────────────────────────────────────────────────────

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

vi.mock("@/lib/runtime/workspace-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/runtime/workspace-api")>()),
  pickExistingDirectory: vi.fn().mockResolvedValue(null)
}));

function createPendingAsk(partial: Partial<PendingAsk> = {}): PendingAsk {
  return {
    requestId: partial.requestId ?? "ask-1",
    requestKind: partial.requestKind ?? "interaction",
    sessionId: partial.sessionId ?? "session-test",
    runId: partial.runId ?? "run-1",
    turnId: partial.turnId ?? "turn-1",
    callId: partial.callId ?? "call-1",
    descriptorSnapshotId: partial.descriptorSnapshotId ?? "snapshot-1",
    descriptorId: partial.descriptorId ?? "builtin:ask",
    finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
    policyDigest: partial.policyDigest ?? "policy-digest",
    nonce: partial.nonce ?? "nonce-1",
    version: partial.version ?? 1,
    expiresAtMs: partial.expiresAtMs ?? Date.now() + 60_000,
    state: partial.state ?? "pending",
    prompt: partial.prompt ?? "请选择接下来的操作：",
    options: partial.options ?? null
  };
}

type ComposerWrapper = ReturnType<typeof mountComposer>;

function mountComposer(pinia: ReturnType<typeof createPinia>) {
  setActivePinia(pinia);
  return mount(WorkspaceComposer, {
    props: {
      showReasoningContent: false,
      canUndoLastTurn: false,
      undoShortcutLabel: "Ctrl+Z",
      handleComposerKeydown: () => {},
      handlePrimaryAction: () => {},
      handleUndoLastTurn: () => {},
      setComposerShellRef: () => {}
    },
    global: {
      plugins: [pinia],
      stubs: {
        TooltipRoot: { template: "<div><slot /></div>" },
        TooltipTrigger: { template: "<div><slot /></div>" },
        TooltipPortal: { template: "<div><slot /></div>" },
        TooltipContent: { template: "<div><slot /></div>" }
      }
    }
  });
}

/** 有选项的 ASK（渲染 `composer-ask-option-item-N` + `composer-ask-custom-input`）。 */
function mountAskWithOptions(options: unknown[] = ["Webpack", "Vite"]) {
  const pinia = createPinia();
  const runtimeStore = useRuntimeStore(pinia);
  runtimeStore.sessionId = "session-test";
  const askStore = useAskStore(pinia);
  const ask = createPendingAsk({ sessionId: "session-test", options });
  askStore.pendingAsks = [ask];
  const answerSpy = vi.spyOn(askStore, "answer").mockResolvedValue(true);
  const wrapper = mountComposer(pinia);
  return { wrapper, askStore, ask, answerSpy };
}

/** 无选项的 ASK（渲染 `composer-ask-textarea`）。 */
function mountAskWithoutOptions() {
  const pinia = createPinia();
  const runtimeStore = useRuntimeStore(pinia);
  runtimeStore.sessionId = "session-test";
  const askStore = useAskStore(pinia);
  const ask = createPendingAsk({ sessionId: "session-test", options: null });
  askStore.pendingAsks = [ask];
  const answerSpy = vi.spyOn(askStore, "answer").mockResolvedValue(true);
  const wrapper = mountComposer(pinia);
  return { wrapper, askStore, ask, answerSpy };
}

function seedSidebarState() {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionId: "current-1",
    sessionOperation: null,
    isSubmitting: false,
    messages: [
      {
        id: "m-1",
        turnId: "t-1",
        role: "user" as const,
        content: "seed",
        status: "done" as const,
        tokenCount: null,
        reasoningContent: null,
        modelName: null,
        toolName: null,
        detail: null,
        durationSeconds: null
      }
    ],
    sessionList: [
      {
        conversationId: "s-idle",
        title: "空闲会话",
        summary: "Summary",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: 4000,
        workspaceId: "ws-a"
      } satisfies SessionOverview
    ],
    workspaceList: [
      { id: DEFAULT_WORKSPACE_ID, name: "默认工作区" },
      { id: "ws-a", name: "项目A" }
    ],
    workspaceListLoaded: true
  });
  return runtimeStore;
}

function mountSidebar() {
  return mount(HomeSessionSidebar, {
    props: { currentPage: "home" as const, forceCollapsed: false },
    global: { stubs: { ScrollArea: { template: "<div><slot /></div>" }, teleport: true } },
    attachTo: document.body
  });
}

/** 显式条件轮询（无固定 sleep）：等待 DOM 出现目标元素。 */
async function waitForElement(
  probe: () => Element | null,
  label: string,
  maxRounds = 40
): Promise<Element> {
  for (let round = 0; round < maxRounds; round++) {
    const found = probe();
    if (found) return found;
    await flushPromises();
    await nextTick();
  }
  throw new Error(`waitForElement timeout: ${label}`);
}

/** 在 wrapper 内按 testid 轮询等待元素（DOMWrapper.element 在空集上会抛异常，须先 exists）。 */
function findTestId(root: { find: (selector: string) => { exists: () => boolean; element: Element } }, testId: string) {
  const probe = () => {
    const found = root.find(`[data-testid="${testId}"]`);
    return found.exists() ? found.element : null;
  };
  return waitForElement(probe, testId);
}

function sessionRow(wrapper: ReturnType<typeof mountSidebar>, conversationId: string) {
  const row = wrapper
    .findAllComponents(SessionRow)
    .find((candidate) => (candidate.props("session") as SessionOverview).conversationId === conversationId);
  if (!row) throw new Error(`session row not found: ${conversationId}`);
  return row;
}

function workspaceRenameMenu(wrapper: ReturnType<typeof mountSidebar>) {
  const menu = wrapper.findAllComponents(DropdownMenu).find((candidate) => {
    const items = candidate.props("items") as DropdownMenuItemSpec[] | undefined;
    return Array.isArray(items) && items.length === 2 && (items[0] as DropdownMenuItemSpec)?.id === "rename";
  });
  if (!menu) throw new Error("workspace rename menu not found");
  return menu;
}

/** 打开模型菜单并取出 Teleport 到 body 的「显示思考」菜单项。 */
async function openReasoningToggle(wrapper: ComposerWrapper): Promise<HTMLElement> {
  await wrapper.get('[data-testid="workspace-model-menu-trigger"]').trigger("click");
  await nextTick();
  await nextTick();
  return (await waitForElement(
    () => document.querySelector('[data-testid="reasoning-visibility-toggle"]'),
    "reasoning-visibility-toggle"
  )) as HTMLElement;
}

function reasoningToggleEmits(wrapper: ComposerWrapper) {
  return wrapper.emitted("update:showReasoningContent") ?? [];
}

/** 以显式属性定义构造敌意事件（属性形状不受构造器默认值保护）。 */
function defineEventProps<T extends Event>(
  event: T,
  props: Record<string, unknown>
): T {
  for (const [key, value] of Object.entries(props)) {
    Object.defineProperty(event, key, { value, configurable: true, writable: true });
  }
  return event;
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  setActivePinia(createPinia());
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
    if (command === "ask_list_pending") return Promise.resolve(useAskStore().pendingAsks);
    return Promise.resolve(null);
  });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver
  );
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.spyOn(useUpdateStore(), "initialize").mockResolvedValue();
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

// ────────────────────────────────────────────────────────────────────────────
// C4-1 事件对象边界穿透
// ────────────────────────────────────────────────────────────────────────────

describe("C4-1 事件对象边界穿透（守卫不得抛异常）", () => {
  it("undefined / null / MouseEvent 入参时守卫必须返回 false 且不抛异常", () => {
    expect(() => isImeComposingEvent(undefined)).not.toThrow();
    expect(isImeComposingEvent(undefined)).toBe(false);

    expect(() => isImeComposingEvent(null)).not.toThrow();
    expect(isImeComposingEvent(null)).toBe(false);

    const mouseEvent = new MouseEvent("click", { bubbles: true });
    expect(() => isImeComposingEvent(mouseEvent)).not.toThrow();
    expect(isImeComposingEvent(mouseEvent)).toBe(false);
  });

  it("isComposing === undefined（含显式 defineProperty 置 undefined）时守卫返回 false 且不抛异常", () => {
    const explicitUndefined = defineEventProps(
      new KeyboardEvent("keydown", { key: "Enter" }),
      { isComposing: undefined, keyCode: 0 }
    );
    expect(() => isImeComposingEvent(explicitUndefined)).not.toThrow();
    expect(isImeComposingEvent(explicitUndefined)).toBe(false);

    // 缺失字段的裸对象（isComposing 与 keyCode 均为 undefined）
    const bare = { key: "Enter" } as unknown as Event;
    expect(() => isImeComposingEvent(bare)).not.toThrow();
    expect(isImeComposingEvent(bare)).toBe(false);

    const empty = {} as unknown as Event;
    expect(() => isImeComposingEvent(empty)).not.toThrow();
    expect(isImeComposingEvent(empty)).toBe(false);
  });

  it("keyCode === 0 视为非组合态；isComposing=true / keyCode=229 视为组合态", () => {
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 0 }))).toBe(false);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: false, keyCode: 13 }))).toBe(false);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true }))).toBe(true);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229 }))).toBe(true);
  });

  it("边界穿透后非组合态回车仍必须提交（防吞反向断言）", async () => {
    const { wrapper, ask, answerSpy } = mountAskWithOptions();
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("候选词");

    // keyCode === 0 的正常回车
    await input.trigger("keydown", { key: "Enter", isComposing: false, keyCode: 0 });
    expect(answerSpy).toHaveBeenCalledTimes(1);
    expect(answerSpy).toHaveBeenLastCalledWith(ask, "候选词");

    // 敌意事件形状：isComposing / keyCode 被显式定义为 undefined
    const hostile = defineEventProps(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
      { isComposing: undefined, keyCode: undefined }
    );
    input.element.dispatchEvent(hostile);
    await nextTick();
    expect(answerSpy).toHaveBeenCalledTimes(2);
    expect(answerSpy).toHaveBeenLastCalledWith(ask, "候选词");

    wrapper.unmount();
  });

  it("组合态（isComposing=true / keyCode=229）回车不得提交", async () => {
    const { wrapper, answerSpy } = mountAskWithOptions();
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("候选词");

    await input.trigger("keydown", { key: "Enter", isComposing: true });
    expect(answerSpy, "isComposing=true 的回车不得提交").toHaveBeenCalledTimes(0);

    await input.trigger("keydown", { key: "Enter", keyCode: 229 });
    expect(answerSpy, "keyCode=229 的回车不得提交").toHaveBeenCalledTimes(0);

    await input.trigger("keydown", { key: "Enter", isComposing: true, keyCode: 229 });
    expect(answerSpy, "isComposing+keyCode 双标记不得提交").toHaveBeenCalledTimes(0);

    wrapper.unmount();
  });
});

// ────────────────────────────────────────────────────────────────────────────
// C4-2 组合序列畸形
// ────────────────────────────────────────────────────────────────────────────

describe("C4-2 组合序列畸形（交错序列下仅非组合态提交）", () => {
  it("compositionstart → 组合态回车 → compositionend 交错序列仅非组合态回车提交", async () => {
    const { wrapper, ask, answerSpy } = mountAskWithOptions();
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("候选词");

    // 1) 进入组合
    await input.trigger("compositionstart");
    expect(answerSpy, "compositionstart 本身不得提交").toHaveBeenCalledTimes(0);

    // 2) 组合态回车（确认候选词）—— 误触高发点
    await input.trigger("keydown", { key: "Enter", isComposing: true });
    expect(answerSpy, "组合态回车不得提交").toHaveBeenCalledTimes(0);

    // 3) 交错：连续组合态回车
    await input.trigger("keydown", { key: "Enter", isComposing: true });
    await input.trigger("keydown", { key: "Enter", isComposing: true, keyCode: 229 });
    expect(answerSpy, "连续组合态回车不得提交").toHaveBeenCalledTimes(0);

    // 4) 组合结束
    await input.trigger("compositionend");
    expect(answerSpy, "compositionend 本身不得提交").toHaveBeenCalledTimes(0);

    // 5) Safari 尾回车：compositionend 之后紧跟 isComposing=true 的 keydown
    await input.trigger("keydown", { key: "Enter", isComposing: true });
    expect(answerSpy, "compositionend 尾回车不得提交").toHaveBeenCalledTimes(0);

    // 6) 孤立 compositionend（无 compositionstart）+ keyCode 229
    await input.trigger("compositionend");
    await input.trigger("keydown", { key: "Enter", keyCode: 229 });
    expect(answerSpy, "孤立 compositionend 后的组合态回车不得提交").toHaveBeenCalledTimes(0);

    // 7) 真正的提交回车（非组合态）
    await input.trigger("keydown", { key: "Enter", isComposing: false });
    expect(answerSpy, "非组合态回车必须提交").toHaveBeenCalledTimes(1);
    expect(answerSpy).toHaveBeenLastCalledWith(ask, "候选词");

    // 8) 提交后继续交错：非组合态仍提交，组合态仍被吞
    await input.trigger("keydown", { key: "Enter", isComposing: true });
    expect(answerSpy).toHaveBeenCalledTimes(1);
    await input.trigger("keydown", { key: "Enter" });
    expect(answerSpy).toHaveBeenCalledTimes(2);

    wrapper.unmount();
  });

  it("ASK textarea 在组合序列下不得提交，非组合态必须提交", async () => {
    const { wrapper, ask, answerSpy } = mountAskWithoutOptions();
    const textarea = wrapper.get('[data-testid="composer-ask-textarea"]');
    await textarea.setValue("postgresql://localhost:5432/mydb");

    await textarea.trigger("compositionstart");
    await textarea.trigger("keydown", { key: "Enter", isComposing: true });
    expect(answerSpy, "textarea 组合态回车不得提交").toHaveBeenCalledTimes(0);

    await textarea.trigger("keydown", { key: "Enter", keyCode: 229 });
    expect(answerSpy, "textarea keyCode=229 不得提交").toHaveBeenCalledTimes(0);

    await textarea.trigger("compositionend");
    await textarea.trigger("keydown", { key: "Enter", isComposing: true });
    expect(answerSpy, "textarea compositionend 尾回车不得提交").toHaveBeenCalledTimes(0);

    await textarea.trigger("keydown", { key: "Enter", isComposing: false });
    expect(answerSpy, "textarea 非组合态回车必须提交").toHaveBeenCalledTimes(1);
    expect(answerSpy).toHaveBeenLastCalledWith(ask, "postgresql://localhost:5432/mydb");

    wrapper.unmount();
  });

  it("ASK 选项按钮组合态回车不得触发 handleOptionEnter（选中态不得变化）", async () => {
    const { wrapper, answerSpy } = mountAskWithOptions();
    const option = wrapper.get('[data-testid="composer-ask-option-item-0"]');
    expect(option.classes()).toContain("bg-stone-100/60");

    await option.trigger("compositionstart");
    await option.trigger("keydown", { key: "Enter", isComposing: true });

    expect(answerSpy, "选项组合态回车不得提交").toHaveBeenCalledTimes(0);
    expect(option.classes(), "选项组合态回车不得改变选中态").toContain("bg-stone-100/60");
    expect(option.classes()).not.toContain("bg-stone-200/90");

    await option.trigger("keydown", { key: "Enter", keyCode: 229 });
    expect(answerSpy, "选项 keyCode=229 不得提交").toHaveBeenCalledTimes(0);
    expect(option.classes()).toContain("bg-stone-100/60");

    // 反向断言：非组合态回车必须走 handleOptionEnter 并提交
    await option.trigger("compositionend");
    await option.trigger("keydown", { key: "Enter", isComposing: false });
    expect(option.classes(), "非组合态回车必须选中该选项").toContain("bg-stone-200/90");
    expect(answerSpy, "非组合态回车必须提交").toHaveBeenCalledTimes(1);

    wrapper.unmount();
  });

  it("会话重命名输入框组合态回车不得 emit submit-rename", async () => {
    seedSidebarState();
    const runtimeStore = useRuntimeStore();
    const renameSpy = vi.spyOn(runtimeStore, "renameSession").mockResolvedValue({ ok: true });
    const wrapper = mountSidebar();

    const row = sessionRow(wrapper, "s-idle");
    row.vm.$emit("menu-select", "rename");
    const inputEl = (await findTestId(wrapper, "session-rename-input")) as HTMLInputElement;
    const input = wrapper.get('[data-testid="session-rename-input"]');
    expect(inputEl.tagName).toBe("INPUT");

    await input.setValue("新标题");
    await input.trigger("compositionstart");
    await input.trigger("keydown", { key: "Enter", isComposing: true });
    await flushPromises();
    expect(renameSpy, "会话重命名组合态回车不得提交").toHaveBeenCalledTimes(0);

    await input.trigger("keydown", { key: "Enter", keyCode: 229 });
    await flushPromises();
    expect(renameSpy, "会话重命名 keyCode=229 不得提交").toHaveBeenCalledTimes(0);

    await input.trigger("compositionend");
    await input.trigger("keydown", { key: "Enter", isComposing: false });
    await flushPromises();
    expect(renameSpy, "会话重命名非组合态回车必须提交").toHaveBeenCalledTimes(1);
    expect(renameSpy).toHaveBeenCalledWith("s-idle", "新标题");

    wrapper.unmount();
  });

  it("工作区重命名输入框组合态回车不得提交 submitRename", async () => {
    seedSidebarState();
    const runtimeStore = useRuntimeStore();
    const renameSpy = vi.spyOn(runtimeStore, "renameWorkspace").mockResolvedValue({ ok: true });
    const wrapper = mountSidebar();

    workspaceRenameMenu(wrapper).vm.$emit("select", "rename");
    const input = await findTestId(wrapper, "workspace-rename-input");
    const inputWrapper = wrapper.get('[data-testid="workspace-rename-input"]');
    expect((input as HTMLInputElement).value).toBe("项目A");

    await inputWrapper.setValue("项目A改");
    await inputWrapper.trigger("compositionstart");
    await inputWrapper.trigger("keydown", { key: "Enter", isComposing: true });
    await flushPromises();
    expect(renameSpy, "工作区重命名组合态回车不得提交").toHaveBeenCalledTimes(0);

    await inputWrapper.trigger("keydown", { key: "Enter", keyCode: 229 });
    await flushPromises();
    expect(renameSpy, "工作区重命名 keyCode=229 不得提交").toHaveBeenCalledTimes(0);

    await inputWrapper.trigger("compositionend");
    await inputWrapper.trigger("keydown", { key: "Enter", isComposing: false });
    await flushPromises();
    expect(renameSpy, "工作区重命名非组合态回车必须提交").toHaveBeenCalledTimes(1);
    expect(renameSpy).toHaveBeenCalledWith("ws-a", "项目A改");

    wrapper.unmount();
  });

  it("「显示思考」菜单项组合态回车不得切换", async () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    const wrapper = mountComposer(pinia);

    const toggle = await openReasoningToggle(wrapper);
    expect(reasoningToggleEmits(wrapper), "初始不得有切换 emit").toHaveLength(0);

    toggle.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }));
    await nextTick();
    expect(reasoningToggleEmits(wrapper), "组合态回车不得切换").toHaveLength(0);

    toggle.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true } as unknown as KeyboardEventInit)
    );
    await nextTick();
    expect(reasoningToggleEmits(wrapper), "keyCode=229 不得切换").toHaveLength(0);

    // 反向断言：非组合态回车必须切换
    toggle.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await nextTick();
    expect(reasoningToggleEmits(wrapper), "非组合态回车必须切换").toHaveLength(1);
    expect(reasoningToggleEmits(wrapper)[0]).toEqual([true]);

    wrapper.unmount();
  });
});

// ────────────────────────────────────────────────────────────────────────────
// C4-3 确定性并发
// ────────────────────────────────────────────────────────────────────────────

describe("C4-3 确定性并发（固定 200 次交替注入，零 sleep / 零随机）", () => {
  const ITERATIONS = 200;
  const EXPECTED_SUBMITS = 100;
  const COMPOSING_INJECTIONS = 100;

  it("200 次交替注入组合/非组合回车：提交计数精确等于 100", async () => {
    const { wrapper, ask, answerSpy } = mountAskWithOptions();
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("候选词");

    let composingInjected = 0;
    let nonComposingInjected = 0;
    for (let index = 0; index < ITERATIONS; index++) {
      // 纯函数决定注入类型：偶数索引 = 组合态，奇数索引 = 非组合态
      const isComposing = index % 2 === 0;
      if (isComposing) composingInjected++;
      else nonComposingInjected++;
      await input.trigger("keydown", { key: "Enter", isComposing, keyCode: isComposing ? 229 : 0 });
    }

    expect(composingInjected).toBe(COMPOSING_INJECTIONS);
    expect(nonComposingInjected).toBe(EXPECTED_SUBMITS);
    expect(answerSpy, "提交次数必须精确等于非组合态注入次数").toHaveBeenCalledTimes(
      EXPECTED_SUBMITS
    );

    for (const call of answerSpy.mock.calls) {
      // ── 冻结测试缺陷修订（Lead 授权最小解冻，仅本处 + 下方第二处身份断言）──────────────
      // 缺陷：`ask` 经 `WorkspaceComposer.vue` 的 `activeAsk` computed 从 Pinia state 读出，
      // 提交给 store 的是 **Vue reactive Proxy**；本 spec 持有的却是 `createPendingAsk()`
      // 返回的**原始对象** → `Object.is(proxy, raw) === false`，`toBe` 恒假
      // （失败形态：`Received: serializes to the same string`）。
      // 探针实测（executor-a）：`{"storeSlotIsRaw":false,"activeAskIsRaw":false,
      //   "activeAskIsProxy":true,"toRawActiveAskIsRaw":true,"deepEqual":true}`。
      // 修订：改为 `toRaw` **身份等价** —— 先剥离 reactive 包装再按引用比对，语义强度不降
      // （仍是同一对象的引用同一性，不是浅比较/深比较的弱化）。
      // 红相有效性不受影响：红相时本断言之前已先在 `toHaveBeenCalledTimes(100)` 失败
      // （`got 200 times`），身份断言从未被执行。
      expect(
        toRaw(call[0]),
        "提交的必须是同一个 ask 对象（剥离 reactive 包装后按引用比对）"
      ).toBe(ask);
      expect(call[1]).toBe("候选词");
    }

    // 负向自检（防"恒真断言"）：证明上面这条身份断言**具备证伪能力** ——
    // 把期望值换成另一个 ask（诱饵）时，断言必须抛错。
    // 缺此自检，`toBe` 类身份断言可能因对象恒等性退化而悄悄变成恒真。
    const decoyAsk = createPendingAsk({ requestId: "ask-decoy" });
    expect(
      () => expect(toRaw(answerSpy.mock.calls[0]![0])).toBe(decoyAsk),
      "负向自检：诱饵 ask 必须使身份断言失败（否则该断言是恒真的假测试）"
    ).toThrow();

    wrapper.unmount();
  });

  it("第二轮 200 次交替注入：计数仍精确等于 100（确定性可复现）", async () => {
    const { wrapper, ask, answerSpy } = mountAskWithOptions();
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("候选词");

    const runRound = async () => {
      for (let index = 0; index < ITERATIONS; index++) {
        const isComposing = index % 2 === 0;
        await input.trigger("keydown", { key: "Enter", isComposing, keyCode: isComposing ? 229 : 0 });
      }
    };

    await runRound();
    expect(answerSpy, "第一轮提交计数").toHaveBeenCalledTimes(EXPECTED_SUBMITS);

    answerSpy.mockClear();
    await runRound();
    expect(answerSpy, "第二轮提交计数必须与第一轮完全一致").toHaveBeenCalledTimes(
      EXPECTED_SUBMITS
    );
    // 冻结测试缺陷修订（Lead 授权最小解冻，第二处身份断言）：同 `:582` —— 提交实参是
    // reactive Proxy，须 `toRaw` 后再按引用比对，否则 `every` 判据恒为 false。
    expect(
      answerSpy.mock.calls.every((call) => toRaw(call[0]) === ask && call[1] === "候选词")
    ).toBe(true);

    // 负向自检（防"恒真/恒假断言"）：把判据中的期望对象换成诱饵 ask 时，`every` 必须翻转为
    // false —— 证明该判据真的在比对对象身份，而不是恒真或恒假。
    const decoyAsk = createPendingAsk({ requestId: "ask-decoy" });
    expect(
      answerSpy.mock.calls.every((call) => toRaw(call[0]) === decoyAsk),
      "负向自检：诱饵 ask 必须使身份判据为 false（否则该判据是恒真的假测试）"
    ).toBe(false);

    wrapper.unmount();
  });

  it("反向压力：200 次全非组合态注入必须精确产生 200 次提交（守卫不得过度拦截）", async () => {
    const { wrapper, answerSpy } = mountAskWithOptions();
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("候选词");

    for (let index = 0; index < ITERATIONS; index++) {
      await input.trigger("keydown", { key: "Enter", isComposing: false, keyCode: 13 });
    }

    expect(answerSpy, "全非组合态注入必须全部提交").toHaveBeenCalledTimes(ITERATIONS);

    wrapper.unmount();
  });
});

/**
 * Wave-17 冻结契约红相验收（L2-T）— C1：IME 组合态回车守卫
 *
 * 契约源（只读）：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 C1-1 ~ C1-8。
 *
 * 缺陷类：Vue 的 `.enter` 修饰符只比对 `event.key`，不检查 `event.isComposing`；
 * 中文/日文/韩文输入法选词确认的那一次回车会被误当作"提交/确认"。
 *
 * 覆盖矩阵：
 *   C1-1 composer-ask-custom-input  组合态回车 → 不得提交（askStore.answer 未被调用）
 *   C1-2 composer-ask-textarea      组合态回车 → 不得提交
 *   C1-3 composer-ask-option-item-N 组合态回车 → 不得触发 handleOptionEnter
 *   C1-4 keyCode === 229（旧式 IME 标记）与 isComposing === true 等价
 *   C1-5 反向断言（防吞）：isComposing === false 的 Enter 必须照常提交
 *   C1-6 session-rename-input       组合态回车 → 不得 emit `submit-rename`
 *   C1-7 workspace-rename-input     组合态回车 → 不得调用 store.renameWorkspace
 *   C1-8 reasoning-visibility-toggle 组合态回车 → 不得 emit `update:showReasoningContent`
 *
 * 零假测试律：本文件所有用例均有状态突变断言；无 skip、无吞异常、无同义反复断言。
 * 事件构造统一走原生 `new KeyboardEvent(...)` + `dispatchEvent`（见 §PROBE 自检块，
 * 先证明 jsdom/VTU 真的把 `isComposing` / `keyCode` 透传到监听器，否则本文件自身即为假测试）。
 *
 * 作者：test-agent（L2-T）。红相锚定后本文件冻结只读，任何修改需 Lead 批准。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import SessionRow from "@/components/HomeSessionSidebarRow.vue";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import DropdownMenu from "@/components/ui/DropdownMenu.vue";
import type { DropdownMenuItemSpec } from "@/components/ui/DropdownMenu.vue";
import { useAskStore } from "@/stores/ask";
import { useRuntimeStore } from "@/stores/runtime";
import { useUpdateStore } from "@/stores/update";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import type { PendingAsk } from "@/types/ask-plan";
import type { SessionOverview } from "@/types/runtime";

// ── 事件构造（唯一入口，保证所有用例的事件形状一致且可审计）─────────────────

/** 组合态回车：现代浏览器路径（`KeyboardEvent.isComposing === true`）。 */
function composingEnter(): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key: "Enter",
    isComposing: true,
    bubbles: true,
    cancelable: true
  });
}

/** 旧式 IME 标记：`keyCode === 229`，等价于组合态。 */
function legacyImeEnter(): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key: "Enter",
    keyCode: 229,
    bubbles: true,
    cancelable: true
  });
}

/** 组合已结束的普通回车：必须照常提交（C1-5 反向断言）。 */
function plainEnter(): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key: "Enter",
    isComposing: false,
    bubbles: true,
    cancelable: true
  });
}

/** 派发并等待 Vue 的异步处理链（submitAskAnswer / submitRename 均为 async）。 */
async function press(el: Element, event: KeyboardEvent): Promise<void> {
  el.dispatchEvent(event);
  await nextTick();
  await flushPromises();
  await nextTick();
}

// ── 探针：先证明测试工具链本身不是假测试 ──────────────────────────────────

describe("wave-17 C1 前置探针 — jsdom / VTU 事件属性透传自检", () => {
  it("PROBE: isComposing 与 keyCode 必须真实抵达 DOM 监听器（原生构造器 + VTU trigger 双路径）", async () => {
    // Arrange：原生监听器 + VTU 挂载的组件监听器，两条路径都记录收到的属性。
    const host = document.createElement("div");
    document.body.appendChild(host);
    const seen: Array<{ key: string; isComposing: boolean; keyCode: number }> = [];
    host.addEventListener("keydown", (event) => {
      const keyboardEvent = event as KeyboardEvent;
      seen.push({
        key: keyboardEvent.key,
        isComposing: keyboardEvent.isComposing,
        keyCode: keyboardEvent.keyCode
      });
    });

    const Probe = defineComponent({ template: '<input data-testid="probe-input" />' });
    const probe = mount(Probe, { attachTo: host });

    // Act 1：原生构造器路径（本 spec 组合态事件的构造方式）。
    host.dispatchEvent(composingEnter());
    host.dispatchEvent(legacyImeEnter());
    // Act 2：VTU trigger 路径（与 tests/HomeWorkspace.spec.ts:2806 的冻结写法同源）。
    await probe.get('[data-testid="probe-input"]').trigger("keydown", { key: "Enter", isComposing: true });
    await probe.get('[data-testid="probe-input"]').trigger("keydown", { key: "Enter", keyCode: 229 });

    probe.unmount();
    host.remove();

    // Assert：4 条记录，属性逐条精确匹配。任一为 false 说明测试工具链吞掉了属性，
    // 此时后续所有"组合态不提交"的断言都会因事件根本不是组合态而变成假绿。
    expect(seen, "必须捕获 4 次 keydown").toHaveLength(4);
    expect(seen[0], "原生构造器：isComposing 必须为 true").toMatchObject({
      key: "Enter",
      isComposing: true
    });
    expect(seen[1], "原生构造器：keyCode 必须为 229").toMatchObject({
      key: "Enter",
      keyCode: 229
    });
    expect(seen[2], "VTU trigger：isComposing 必须为 true").toMatchObject({
      key: "Enter",
      isComposing: true
    });
    expect(seen[3], "VTU trigger：keyCode 必须为 229").toMatchObject({
      key: "Enter",
      keyCode: 229
    });
  });
});

// ── C1-1 ~ C1-5：WorkspaceComposer 的 ASK 三处回车入口 ──────────────────────

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

/** 按 tests/workspace-composer-ask.spec.ts 的既有写法构造 activeAsk 场景。 */
function mountComposerWithAsk(ask: PendingAsk) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const runtimeStore = useRuntimeStore(pinia);
  runtimeStore.sessionId = ask.sessionId ?? "session-test";
  const askStore = useAskStore(pinia);
  askStore.pendingAsks = [ask];
  const answerSpy = vi.spyOn(askStore, "answer").mockResolvedValue(true);

  const wrapper = mount(WorkspaceComposer, {
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
        teleport: true,
        TooltipRoot: { template: "<div><slot /></div>" },
        TooltipTrigger: { template: "<div><slot /></div>" },
        TooltipPortal: { template: "<div><slot /></div>" },
        TooltipContent: { template: "<div><slot /></div>" }
      }
    }
  });

  return { wrapper, askStore, answerSpy };
}

describe("wave-17 C1：WorkspaceComposer ASK 输入框 IME 组合态守卫", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve(useAskStore().pendingAsks);
      }
      return Promise.resolve(null);
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("C1-1: composer-ask-custom-input 组合态回车不得触发回答提交", async () => {
    // Arrange
    const ask = createPendingAsk({
      requestId: "ask-c1-1",
      prompt: "请选择构建方式：",
      options: ["Webpack", "Vite"]
    });
    const { wrapper, answerSpy } = mountComposerWithAsk(ask);
    const input = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await input.setValue("Rollup 打包");

    // Act：中文输入法选词确认的那一次回车。
    await press(input.element, composingEnter());

    // Assert：SUT 未提交。
    expect(
      answerSpy,
      "组合态（isComposing=true）回车被误判为提交：askStore.answer 不应被调用"
    ).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it("C1-2: composer-ask-textarea 组合态回车不得触发回答提交", async () => {
    // Arrange：无预设选项 → 渲染 textarea 分支。
    const ask = createPendingAsk({
      requestId: "ask-c1-2",
      prompt: "请提供数据库连接串：",
      options: null
    });
    const { wrapper, answerSpy } = mountComposerWithAsk(ask);
    const textarea = wrapper.get('[data-testid="composer-ask-textarea"]');
    await textarea.setValue("postgresql://localhost:5432/mydb");

    // Act
    await press(textarea.element, composingEnter());

    // Assert
    expect(
      answerSpy,
      "组合态（isComposing=true）回车被误判为提交：askStore.answer 不应被调用"
    ).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it("C1-3: composer-ask-option-item-N 组合态回车不得触发 handleOptionEnter", async () => {
    // Arrange
    const ask = createPendingAsk({
      requestId: "ask-c1-3",
      prompt: "请选择架构方案：",
      options: ["方案 A", "方案 B"]
    });
    const { wrapper, answerSpy } = mountComposerWithAsk(ask);
    const option = wrapper.get('[data-testid="composer-ask-option-item-0"]');

    // Act：焦点在选项按钮上时，输入法组合态回车（选项 label 常含中文，用户会先输入筛选）。
    await press(option.element, composingEnter());

    // Assert
    expect(
      answerSpy,
      "组合态回车穿透到选项按钮：handleOptionEnter → submitAskAnswer 不应被触发"
    ).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it("C1-4: keyCode === 229 旧式 IME 标记与 isComposing 等价生效（custom-input 与 textarea 双入口）", async () => {
    // Arrange A：有选项 → custom-input
    const askWithOptions = createPendingAsk({
      requestId: "ask-c1-4a",
      prompt: "请选择构建方式：",
      options: ["Webpack", "Vite"]
    });
    const first = mountComposerWithAsk(askWithOptions);
    const customInput = first.wrapper.get('[data-testid="composer-ask-custom-input"]');
    await customInput.setValue("Rollup 打包");

    // Act A
    await press(customInput.element, legacyImeEnter());

    // Assert A
    expect(
      first.answerSpy,
      "keyCode=229 未被识别为组合态：custom-input 不应提交"
    ).not.toHaveBeenCalled();
    first.wrapper.unmount();

    // Arrange B：无选项 → textarea
    const askNoOptions = createPendingAsk({
      requestId: "ask-c1-4b",
      prompt: "请提供数据库连接串：",
      options: null
    });
    const second = mountComposerWithAsk(askNoOptions);
    const textarea = second.wrapper.get('[data-testid="composer-ask-textarea"]');
    await textarea.setValue("postgresql://localhost:5432/mydb");

    // Act B
    await press(textarea.element, legacyImeEnter());

    // Assert B
    expect(
      second.answerSpy,
      "keyCode=229 未被识别为组合态：textarea 不应提交"
    ).not.toHaveBeenCalled();
    second.wrapper.unmount();
  });

  it("C1-5（反向断言·防吞）: isComposing === false 的 Enter 必须照常提交（守卫不得过度拦截）", async () => {
    // Arrange A：有选项 + 自定义输入
    const askWithOptions = createPendingAsk({
      requestId: "ask-c1-5a",
      prompt: "请选择构建方式：",
      options: ["Webpack", "Vite"]
    });
    const first = mountComposerWithAsk(askWithOptions);
    const customInput = first.wrapper.get('[data-testid="composer-ask-custom-input"]');
    await customInput.setValue("Rollup 打包");

    // Act A：组合已结束的普通回车。
    await press(customInput.element, plainEnter());

    // Assert A：必须提交，且载荷正确。
    expect(
      first.answerSpy,
      "守卫过度拦截：非组合态回车未提交（isComposing=false 必须照常回答）"
    ).toHaveBeenCalledTimes(1);
    expect(first.answerSpy).toHaveBeenCalledWith(askWithOptions, "Rollup 打包");
    first.wrapper.unmount();

    // Arrange B：无选项 → textarea
    const askNoOptions = createPendingAsk({
      requestId: "ask-c1-5b",
      prompt: "请提供数据库连接串：",
      options: null
    });
    const second = mountComposerWithAsk(askNoOptions);
    const textarea = second.wrapper.get('[data-testid="composer-ask-textarea"]');
    await textarea.setValue("postgresql://localhost:5432/mydb");

    // Act B
    await press(textarea.element, plainEnter());

    // Assert B
    expect(
      second.answerSpy,
      "守卫过度拦截：非组合态回车未提交（isComposing=false 必须照常回答）"
    ).toHaveBeenCalledTimes(1);
    expect(second.answerSpy).toHaveBeenCalledWith(askNoOptions, "postgresql://localhost:5432/mydb");
    second.wrapper.unmount();
  });
});

// ── C1-8：WorkspaceComposer 的「显示思考」菜单项 ────────────────────────────

describe("wave-17 C1-8：「显示思考」菜单项 IME 组合态守卫", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mountComposerWithReasoningMenu() {
    const pinia = createPinia();
    setActivePinia(pinia);
    const wrapper = mount(WorkspaceComposer, {
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
          // Teleport 置为内联渲染，使 `reasoning-visibility-toggle` 落在 wrapper 子树内可定位。
          teleport: true,
          TooltipRoot: { template: "<div><slot /></div>" },
          TooltipTrigger: { template: "<div><slot /></div>" },
          TooltipPortal: { template: "<div><slot /></div>" },
          TooltipContent: { template: "<div><slot /></div>" }
        }
      }
    });
    return wrapper;
  }

  it("C1-8: 组合态回车不得切换「显示思考」；非组合态回车必须切换（防吞）", async () => {
    // Arrange：打开模型/思考浮窗菜单。
    const wrapper = mountComposerWithReasoningMenu();
    await wrapper.get('[data-testid="workspace-model-menu-trigger"]').trigger("click");
    await nextTick();
    const toggle = wrapper.get('[data-testid="reasoning-visibility-toggle"]');
    expect(toggle.text()).toContain("显示思考");

    // Act 1：组合态回车。
    await press(toggle.element, composingEnter());

    // Assert 1：守卫必须拦截，未 emit。
    expect(
      wrapper.emitted("update:showReasoningContent"),
      "组合态回车穿透到「显示思考」：toggleReasoningVisibility 不应被触发"
    ).toBeUndefined();

    // Act 2（反向断言·防吞）：非组合态回车必须照常切换。
    await press(toggle.element, plainEnter());

    // Assert 2
    const emitted = wrapper.emitted("update:showReasoningContent");
    expect(
      emitted,
      "守卫过度拦截：非组合态回车未能切换「显示思考」"
    ).toBeDefined();
    expect(emitted).toHaveLength(1);
    expect(emitted?.[0]).toEqual([true]);

    wrapper.unmount();
  });
});

// ── C1-6：HomeSessionSidebarRow 会话重命名输入框 ────────────────────────────

describe("wave-17 C1-6：会话重命名输入框 IME 组合态守卫", () => {
  function mountRenamingRow() {
    return mount(SessionRow, {
      props: {
        session: {
          conversationId: "s-idle",
          title: "空闲会话",
          summary: "Summary",
          turnCount: 1,
          lastReferencedFile: null,
          updatedAtMs: 4000,
          workspaceId: "ws-a",
          archived: false
        } satisfies SessionOverview,
        menuItems: [],
        selected: false,
        renaming: true,
        renameDraft: "空闲会话",
        renameError: "",
        renamingBusy: false
      },
      attachTo: document.body
    });
  }

  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("C1-6: session-rename-input 组合态回车不得 emit submit-rename；非组合态必须 emit（防吞）", async () => {
    // Arrange
    const wrapper = mountRenamingRow();
    const input = wrapper.get('[data-testid="session-rename-input"]');
    await wrapper.get('[data-testid="session-rename-input"]').setValue("空闲会话改");

    // Act 1：组合态回车。
    await press(input.element, composingEnter());

    // Assert 1
    expect(
      wrapper.emitted("submit-rename"),
      "组合态回车穿透会话重命名：不应 emit submit-rename"
    ).toBeUndefined();

    // Act 2（反向断言·防吞）：非组合态回车必须提交。
    await press(input.element, plainEnter());

    // Assert 2
    const emitted = wrapper.emitted("submit-rename");
    expect(emitted, "守卫过度拦截：非组合态回车未能提交重命名").toBeDefined();
    expect(emitted).toHaveLength(1);

    wrapper.unmount();
  });

  it("C1-6: keyCode === 229 同样不得 emit submit-rename", async () => {
    // Arrange
    const wrapper = mountRenamingRow();
    const input = wrapper.get('[data-testid="session-rename-input"]');
    await input.setValue("空闲会话改");

    // Act
    await press(input.element, legacyImeEnter());

    // Assert
    expect(
      wrapper.emitted("submit-rename"),
      "keyCode=229 未被识别为组合态：不应 emit submit-rename"
    ).toBeUndefined();

    wrapper.unmount();
  });
});

// ── C1-7：HomeSessionSidebar 工作区重命名输入框 ─────────────────────────────

const apiMocks = vi.hoisted(() => ({
  mockPickExistingDirectory: vi.fn()
}));

vi.mock("@/lib/runtime/workspace-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/runtime/workspace-api")>()),
  pickExistingDirectory: (...args: unknown[]) => apiMocks.mockPickExistingDirectory(...args)
}));

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

function seedWorkspaceTree(): void {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionId: "current-1",
    sessionOperation: null,
    isSubmitting: false,
    messages: [
      {
        id: "msg-1",
        turnId: "turn-1",
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
        workspaceId: "ws-a",
        archived: false
      } satisfies SessionOverview
    ],
    workspaceList: [
      { id: DEFAULT_WORKSPACE_ID, name: "默认工作区" },
      { id: "ws-a", name: "项目A" }
    ],
    workspaceListLoaded: true
  });
}

function workspaceRenameMenus(wrapper: ReturnType<typeof mount>) {
  return wrapper.findAllComponents(DropdownMenu).filter((menu) => {
    const items = menu.props("items") as DropdownMenuItemSpec[] | undefined;
    return (
      Array.isArray(items)
      && items.length === 2
      && (items[0] as DropdownMenuItemSpec)?.id === "rename"
    );
  });
}

async function flushSidebar(times = 5): Promise<void> {
  for (let index = 0; index < times; index += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  await nextTick();
}

describe("wave-17 C1-7：工作区重命名输入框 IME 组合态守卫", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    apiMocks.mockPickExistingDirectory.mockReset();
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
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function openWorkspaceRename() {
    seedWorkspaceTree();
    const runtimeStore = useRuntimeStore();
    const renameSpy = vi.spyOn(runtimeStore, "renameWorkspace").mockResolvedValue({ ok: true });
    const wrapper = mount(HomeSessionSidebar, {
      props: { currentPage: "home", forceCollapsed: false },
      global: { stubs: { ScrollArea: ScrollAreaStub, teleport: true } },
      attachTo: document.body
    });
    await flushSidebar();
    const menus = workspaceRenameMenus(wrapper);
    expect(menus.length, "工作区菜单（rename/delete 两项）必须存在").toBeGreaterThan(0);
    menus[0]!.vm.$emit("select", "rename");
    await flushSidebar();
    const input = wrapper.get('[data-testid="workspace-rename-input"]');
    expect((input.element as HTMLInputElement).value).toBe("项目A");
    return { wrapper, input, renameSpy };
  }

  it("C1-7: workspace-rename-input 组合态回车不得调用 store.renameWorkspace；非组合态必须调用（防吞）", async () => {
    // Arrange
    const { wrapper, input, renameSpy } = await openWorkspaceRename();
    await input.setValue("项目A改");

    // Act 1：组合态回车。
    await press(input.element, composingEnter());

    // Assert 1
    expect(
      renameSpy,
      "组合态回车穿透工作区重命名：store.renameWorkspace 不应被调用"
    ).not.toHaveBeenCalled();

    // Act 2（反向断言·防吞）：非组合态回车必须提交。
    await press(input.element, plainEnter());

    // Assert 2
    expect(renameSpy, "守卫过度拦截：非组合态回车未能提交工作区重命名").toHaveBeenCalledTimes(1);
    expect(renameSpy).toHaveBeenCalledWith("ws-a", "项目A改");

    wrapper.unmount();
  });

  it("C1-7: keyCode === 229 同样不得调用 store.renameWorkspace", async () => {
    // Arrange
    const { wrapper, input, renameSpy } = await openWorkspaceRename();
    await input.setValue("项目A改");

    // Act
    await press(input.element, legacyImeEnter());

    // Assert
    expect(
      renameSpy,
      "keyCode=229 未被识别为组合态：store.renameWorkspace 不应被调用"
    ).not.toHaveBeenCalled();

    wrapper.unmount();
  });
});

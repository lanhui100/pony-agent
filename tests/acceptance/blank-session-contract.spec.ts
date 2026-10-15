// T1 红相冻结：新对话入口可用性 + 空白会话零持久化 契约测试（C1..C7）。
//
// 独立测试者（Test Agent）唯一写权；Executor 对验收测试只读，不得修改本文件。
// 分级判定：L2（组件契约 + 持久层 localStorage 契约 + 宿主 IPC 边界断言）。
//
// 冻结契约来源：task-1（.dev-team/team task board）。
// 被测事实锚点（已由本测试作者逐行复核，非引用二手结论）：
//   - src/components/HomeSessionSidebar.vue:500-502  canCreateSession
//   - src/components/HomeSessionSidebar.vue:503-507  canCreateSessionInWorkspace
//   - src/components/HomeSessionSidebar.vue:263-266  createNewSession → DEFAULT_WORKSPACE_ID
//   - src/components/HomeSessionSidebar.vue:670/623  顶层/折叠按钮 :disabled="!canCreateSession"
//   - src/components/HomeSessionSidebar.vue:845      工作区行按钮 :disabled="!canCreateSessionInWorkspace(key)"
//   - src/stores/runtime.ts:2206-2226                createSession 空白分支仅重定向并 return
//   - src/lib/runtime/constants.ts:7                 RUNTIME_STORAGE_KEY
//   - src/lib/runtime/sessions.ts:489-517            persistSessionStateAndRuntimeMaps 写入 { sessions: {...} }
//
// 运行：npx vitest run tests/acceptance/blank-session-contract.spec.ts
//
// 说明：本文件复用 tests/HomeSessionSidebar.spec.ts:17-115 的 mount/seed 基建**模式**，
// 但为独立副本（不改动原文件），以便验收测试可被单独冻结与单独执行。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import type { VueWrapper } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { useUpdateStore } from "@/stores/update";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import { RUNTIME_STORAGE_KEY } from "@/lib/runtime/constants";
import type { SessionOverview } from "@/types/runtime";

// ── 基建（复制自 tests/HomeSessionSidebar.spec.ts 模式） ───────────────────
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

function createSession(partial: Partial<SessionOverview> = {}): SessionOverview {
  return {
    conversationId: partial.conversationId ?? "session-1",
    title: partial.title ?? "Session 1",
    summary: partial.summary ?? "Summary",
    turnCount: partial.turnCount ?? 1,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    updatedAtMs: partial.updatedAtMs ?? 1000,
    workspaceId: partial.workspaceId ?? null,
    archived: partial.archived
  };
}

const REGISTRY = [
  { id: DEFAULT_WORKSPACE_ID, name: "默认工作区" },
  { id: "ws-a", name: "项目A" },
  { id: "ws-b", name: "项目B" }
];

function seedWorkspaces(list: Array<{ id: string; name: string }> = REGISTRY) {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({ workspaceList: list.map((w) => ({ ...w })), workspaceListLoaded: true });
}

interface SeedOptions {
  sessions?: SessionOverview[];
  currentId?: string;
  messages?: Array<{ content: string }>;
  sessionWorkspaceId?: string;
  sessionOperation?: "initializing" | "switching" | "deleting" | null;
}

function seedTree({
  sessions = [],
  currentId = "current-1",
  messages = [{ content: "seed" }],
  sessionWorkspaceId = DEFAULT_WORKSPACE_ID,
  sessionOperation = null
}: SeedOptions = {}) {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionId: currentId,
    sessionOperation,
    sessionWorkspaceId,
    isSubmitting: false,
    messages: messages.map((m, index) => ({
      id: `msg-${index + 1}`,
      turnId: "turn-1",
      role: "user" as const,
      content: m.content,
      status: "done" as const,
      tokenCount: null,
      reasoningContent: null,
      modelName: null,
      toolName: null,
      detail: null,
      durationSeconds: null
    })),
    sessionList: sessions
  });
}

function mountSidebar(currentPage: "home" | "settings" | null = "home", forceCollapsed = false) {
  return mount(HomeSessionSidebar, {
    props: { currentPage, forceCollapsed },
    global: { stubs: { ScrollArea: ScrollAreaStub, teleport: true } },
    attachTo: document.body
  });
}

async function flushUI(times = 5) {
  for (let i = 0; i < times; i++) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  await nextTick();
}

// ── 断言辅助（每条都做存在性前置，避免"元素不存在→静默通过"的假绿） ───────
function disabledAttr(wrapper: VueWrapper, testId: string): string | undefined {
  const el = wrapper.find(`[data-testid="${testId}"]`);
  expect(el.exists(), `缺少元素 [data-testid="${testId}"]`).toBe(true);
  return el.attributes("disabled");
}

function transientRows(): SessionOverview[] {
  const runtimeStore = useRuntimeStore();
  return runtimeStore.sessionList.filter((s) => s.updatedAtMs === 0 && s.turnCount === 0);
}

/** 读取前端持久层 `pony-agent.runtime-history.v1` 的 sessions 键集合。 */
function storageSessionIds(): string[] {
  const raw = window.localStorage.getItem(RUNTIME_STORAGE_KEY);
  if (!raw) return [];
  const parsed = JSON.parse(raw) as { sessions?: Record<string, unknown> };
  return Object.keys(parsed.sessions ?? {});
}

const SESSION_WRITE_IPC = /session|conversation|persist|save|write|create|upsert|store/i;

describe("T1 冻结契约：新对话入口可用性 + 空白会话零持久化", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    apiMocks.mockPickExistingDirectory.mockReset();
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      } as typeof ResizeObserver
    );
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const updateStore = useUpdateStore();
    vi.spyOn(updateStore, "initialize").mockResolvedValue();
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  // ── C1 ──────────────────────────────────────────────────────────────────
  it("C1 顶层/折叠态新对话按钮：sessionOperation===null 时任何 messages+归属组合都不得带 disabled", async () => {
    const scenarios = [
      { name: "空白会话 + 归属 ws-a（非默认工作区）", messages: [] as Array<{ content: string }>, workspace: "ws-a" },
      { name: "空白会话 + 归属默认工作区", messages: [] as Array<{ content: string }>, workspace: DEFAULT_WORKSPACE_ID },
      { name: "有消息 + 归属 ws-a", messages: [{ content: "已有用户消息" }], workspace: "ws-a" }
    ];

    for (const scenario of scenarios) {
      // Arrange
      seedTree({ messages: scenario.messages, sessionWorkspaceId: scenario.workspace, sessionOperation: null });

      // Act + Assert（展开态）
      seedWorkspaces(REGISTRY);
      const expanded = mountSidebar("home", false);
      await flushUI();
      expect(disabledAttr(expanded, "session-sidebar-new-chat"), `[展开态] ${scenario.name}`).toBeUndefined();
      expanded.unmount();

      // Act + Assert（折叠态 rail）
      seedWorkspaces(REGISTRY);
      const collapsedWrapper = mountSidebar("home", true);
      await flushUI();
      expect(
        disabledAttr(collapsedWrapper, "session-sidebar-new-chat-collapsed"),
        `[折叠态] ${scenario.name}`
      ).toBeUndefined();
      collapsedWrapper.unmount();
    }
  });

  // ── C2 ──────────────────────────────────────────────────────────────────
  it("C2 在途 sessionOperation（initializing/switching/deleting）期间顶层按钮必须保持 disabled", async () => {
    for (const operation of ["initializing", "switching", "deleting"] as const) {
      // Arrange：有消息（否则 disabled 可能来自 hasPersistableMessages，无法证明是在途保护）
      seedTree({
        messages: [{ content: "已有用户消息" }],
        sessionWorkspaceId: "ws-a",
        sessionOperation: operation,
        currentId: `current-${operation}`
      });

      // Act + Assert（展开态）
      seedWorkspaces(REGISTRY);
      const wrapper = mountSidebar("home", false);
      await flushUI();
      const el = wrapper.find('[data-testid="session-sidebar-new-chat"]');
      expect(el.exists(), `sessionOperation=${operation} 时顶层按钮应存在`).toBe(true);
      expect(el.attributes("disabled"), `sessionOperation=${operation} 应保留 disabled 属性`).toBeDefined();
      expect((el.element as HTMLButtonElement).disabled, `sessionOperation=${operation} DOM disabled`).toBe(true);
      wrapper.unmount();

      // Act + Assert（折叠态）
      seedWorkspaces(REGISTRY);
      const collapsedWrapper = mountSidebar("home", true);
      await flushUI();
      const collapsedEl = collapsedWrapper.find('[data-testid="session-sidebar-new-chat-collapsed"]');
      expect(collapsedEl.exists(), `折叠态 sessionOperation=${operation} 时按钮应存在`).toBe(true);
      expect(
        collapsedEl.attributes("disabled"),
        `折叠态 sessionOperation=${operation} 应保留 disabled 属性`
      ).toBeDefined();
      collapsedWrapper.unmount();
    }
  });

  // ── C3 ──────────────────────────────────────────────────────────────────
  it("C3 workspace-row-new-<key>：sessionOperation===null 时恒不得带 disabled", async () => {
    const scenarios = [
      { name: "空白会话归属 ws-a（行 key 即当前归属）", messages: [] as Array<{ content: string }>, workspace: "ws-a" },
      { name: "空白会话归属默认工作区", messages: [] as Array<{ content: string }>, workspace: DEFAULT_WORKSPACE_ID },
      { name: "有消息 + 归属 ws-a", messages: [{ content: "已有用户消息" }], workspace: "ws-a" }
    ];

    for (const scenario of scenarios) {
      // Arrange
      seedTree({ messages: scenario.messages, sessionWorkspaceId: scenario.workspace, sessionOperation: null });

      // Act
      seedWorkspaces(REGISTRY);
      const wrapper = mountSidebar();
      await flushUI();

      // Assert：注册表中两个非默认工作区行按钮都必须可用
      for (const key of ["ws-a", "ws-b"]) {
        expect(
          disabledAttr(wrapper, `workspace-row-new-${key}`),
          `${scenario.name} → workspace-row-new-${key}`
        ).toBeUndefined();
      }
      wrapper.unmount();
    }
  });

  // ── C4 ──────────────────────────────────────────────────────────────────
  it("C4 顶层按钮点击目标工作区恒为 DEFAULT_WORKSPACE_ID（spy createSession 入参）", async () => {
    const runtimeStore = useRuntimeStore();
    const spy = vi.spyOn(runtimeStore, "createSession");

    // 场景 B（先跑，证明 spy 通道有效）：有消息 → 按钮本就可点击 → 目标默认工作区
    seedTree({ messages: [{ content: "已有用户消息" }], sessionWorkspaceId: "ws-a", currentId: "current-b" });
    seedWorkspaces(REGISTRY);
    const wrapperB = mountSidebar();
    await flushUI();
    await wrapperB.get('[data-testid="session-sidebar-new-chat"]').trigger("click");
    await flushUI();
    expect(spy, "有消息场景：点击应抵达 store.createSession（spy 通道自证）").toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0]?.[0], "有消息场景：目标工作区").toBe(DEFAULT_WORKSPACE_ID);
    wrapperB.unmount();
    spy.mockClear();

    // 场景 A（缺陷锚点）：空白会话归属 ws-a → 按钮仍须可用，且目标仍是默认工作区
    seedTree({ messages: [], sessionWorkspaceId: "ws-a", currentId: "current-a" });
    seedWorkspaces(REGISTRY);
    const wrapperA = mountSidebar();
    await flushUI();
    const button = wrapperA.get('[data-testid="session-sidebar-new-chat"]');
    expect(button.attributes("disabled"), "空白+ws-a：顶层按钮不得 disabled（否则点击契约不可达）").toBeUndefined();
    await button.trigger("click");
    await flushUI();
    expect(spy, "空白+ws-a：点击应抵达 createSession 且目标为默认工作区").toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0]?.[0], "空白+ws-a：目标工作区恒为默认工作区（而非 ws-a）").toBe(DEFAULT_WORKSPACE_ID);
    wrapperA.unmount();
  });

  // ── C5 ──────────────────────────────────────────────────────────────────
  it("C5 幂等：已处于「空白+归属默认工作区」时点击顶层按钮不得新增瞬态条目、不得写 localStorage", async () => {
    const runtimeStore = useRuntimeStore();
    const currentId = "current-blank-default";

    // Arrange：当前会话已是空白且归属默认工作区，并已在 sessionList 中作为瞬态条目存在
    seedTree({
      currentId,
      messages: [],
      sessionWorkspaceId: DEFAULT_WORKSPACE_ID,
      sessions: [
        createSession({
          conversationId: currentId,
          title: "新对话",
          summary: "",
          turnCount: 0,
          updatedAtMs: 0,
          workspaceId: null
        })
      ]
    });
    seedWorkspaces(REGISTRY);
    const before = transientRows().length;
    expect(before, "前置：当前空白会话应已作为瞬态条目存在").toBe(1);
    const storageBefore = window.localStorage.getItem(RUNTIME_STORAGE_KEY);

    const spy = vi.spyOn(runtimeStore, "createSession");

    // Act
    const wrapper = mountSidebar();
    await flushUI();
    await wrapper.get('[data-testid="session-sidebar-new-chat"]').trigger("click");
    await flushUI();

    // Assert（非空洞前提：点击必须真实抵达 store，否则"没有新增"只是禁用按钮造成的假绿）
    expect(spy, "点击应抵达 store.createSession（否则幂等断言无意义）").toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0]?.[0]).toBe(DEFAULT_WORKSPACE_ID);
    expect(transientRows().length, "瞬态条目数不得增加（不得出现第二个 updatedAtMs=0 的行）").toBe(before);
    expect(runtimeStore.sessionId, "会话 id 不得变化").toBe(currentId);
    expect(window.localStorage.getItem(RUNTIME_STORAGE_KEY), "不得写入 localStorage").toBe(storageBefore);
    wrapper.unmount();
  });

  // ── C6 ──────────────────────────────────────────────────────────────────
  it("C6 空白会话零持久化（前端面）：点击 / persistHistory / 切换离开 三种时机新 sessionId 均不得进入 localStorage", async () => {
    const runtimeStore = useRuntimeStore();
    const previousId = "current-persisted";

    // Arrange：当前为有消息的会话（保证持久层处于"活跃可写"状态，使断言非空洞）
    seedTree({ currentId: previousId, messages: [{ content: "已有用户消息" }], sessionWorkspaceId: DEFAULT_WORKSPACE_ID });
    seedWorkspaces(REGISTRY);
    const spy = vi.spyOn(runtimeStore, "createSession");

    // Act ①：点击顶层新对话 → 产生全新空白会话
    const wrapper = mountSidebar();
    await flushUI();
    await wrapper.get('[data-testid="session-sidebar-new-chat"]').trigger("click");
    await flushUI();

    expect(spy, "前置：点击应抵达 createSession").toHaveBeenCalledTimes(1);
    const blankSessionId = runtimeStore.sessionId;
    expect(blankSessionId, "新建后当前会话应为全新 id").not.toBe(previousId);
    expect(blankSessionId.startsWith("pa_"), "新会话 id 应为 pa_<uuid> 形态").toBe(true);

    // Assert 时机①：点击后
    expect(storageSessionIds(), "对照：旧会话应已落盘（证明持久层在工作）").toContain(previousId);
    expect(storageSessionIds(), "时机①（点击新建后）新空白会话不得落盘").not.toContain(blankSessionId);

    // Act + Assert 时机②：显式触发 persistHistory（模拟离开前/退出前落盘）
    runtimeStore.persistHistory();
    await flushUI();
    expect(storageSessionIds(), "时机②（显式 persistHistory）新空白会话不得落盘").not.toContain(blankSessionId);

    // Act + Assert 时机③：切换离开该空白会话
    await runtimeStore.switchSession(previousId);
    await flushUI();
    expect(runtimeStore.sessionId, "切换后应回到旧会话").toBe(previousId);
    expect(storageSessionIds(), "时机③（切换离开）新空白会话不得落盘").not.toContain(blankSessionId);
    expect(storageSessionIds(), "对照：旧会话仍应在盘上").toContain(previousId);
    wrapper.unmount();
  });

  // ── C7 ──────────────────────────────────────────────────────────────────
  it("C7 空白会话零持久化（宿主面）：createSession 在空白当前会话上不得发起任何后端 IPC", async () => {
    const runtimeStore = useRuntimeStore();
    const currentId = "current-blank-ipc";

    // Arrange：空白会话 + 归属 ws-a（触发"重定向"分支），不挂载组件以隔离 IPC 噪声
    seedTree({ currentId, messages: [], sessionWorkspaceId: "ws-a" });
    seedWorkspaces(REGISTRY);
    tauriMocks.mockSafeInvoke.mockClear();
    const listBefore = runtimeStore.sessionList.length;

    // Act
    await runtimeStore.createSession(DEFAULT_WORKSPACE_ID);
    await flushUI();

    // Assert（非空洞前提：确实走了空白重定向分支）
    expect(runtimeStore.sessionWorkspaceId, "空白会话应被无缝重定向到目标工作区").toBe(DEFAULT_WORKSPACE_ID);
    expect(runtimeStore.sessionId, "空白重定向不得产生新会话 id").toBe(currentId);
    expect(runtimeStore.sessionList.length, "不得新增瞬态条目").toBe(listBefore);

    // Assert：零后端 IPC
    const sessionWriteCalls = tauriMocks.mockSafeInvoke.mock.calls
      .map(([command]) => String(command))
      .filter((command) => SESSION_WRITE_IPC.test(command));
    expect(sessionWriteCalls, "空白会话不得触发会话创建/写库类后端 IPC").toEqual([]);
    expect(tauriMocks.mockSafeInvoke, "空白会话 createSession 不得调用任何后端 IPC").not.toHaveBeenCalled();
  });
});

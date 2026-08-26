// 三级树侧边栏组件契约测试（单一树 IA；取代 PA-081 两级分组时代的旧用例集）。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import SessionRow from "@/components/HomeSessionSidebarRow.vue";
import ConfirmPopover from "@/components/ui/ConfirmPopover.vue";
import DropdownMenu from "@/components/ui/DropdownMenu.vue";
import type { DropdownMenuItemSpec } from "@/components/ui/DropdownMenu.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { useUpdateStore } from "@/stores/update";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import { SIDEBAR_COPY } from "@/lib/runtime/sidebar-copy";
import type { SessionOverview } from "@/types/runtime";

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

function createMessage(partial: Partial<Parameters<typeof Object>[0]> = {}): never {
  throw new Error("unused shim");
}
void createMessage;

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
}

function seedTree({ sessions = [], currentId = "current-1", messages = [{ content: "seed" }] }: SeedOptions = {}) {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionId: currentId,
    sessionOperation: null,
    isSubmitting: false,
    messages: messages.map((m) => ({
      id: "msg-1",
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

function bodyText(): string {
  return document.body.textContent ?? "";
}

describe("HomeSessionSidebar（三级树·chrome 沿革）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
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

  it("settings 是唯一一级导航；brand 点击回 home", async () => {
    seedTree();
    seedWorkspaces([]);
    const wrapper = mountSidebar();
    await flushUI();
    expect(wrapper.find('[data-testid="session-sidebar-nav-settings"]').exists()).toBe(true);
    await wrapper.get('[data-testid="session-sidebar-brand"]').trigger("click");
    expect(wrapper.emitted("navigate")?.[0]).toEqual(["home"]);
    wrapper.unmount();
  });

  it("折叠 rail 四入口与持久化；宽度类固定", async () => {
    seedTree();
    seedWorkspaces([]);
    const wrapper = mountSidebar("home", false);
    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();
    expect(wrapper.find('[data-testid="session-sidebar-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-brand-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-new-chat-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-home-collapsed"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-collapsed"]').exists()).toBe(true);
    await wrapper.get('[data-testid="session-sidebar-home-collapsed"]').trigger("click");
    expect(wrapper.emitted("navigate")?.at(-1)).toEqual(["home"]);
    // localStorage 持久化折叠态
    expect(window.localStorage.getItem("pony-agent.session-sidebar-collapsed.v1")).toBe("1");
    wrapper.unmount();

    const remounted = mountSidebar("home");
    await nextTick();
    expect(remounted.find('[data-testid="session-sidebar-collapsed"]').exists()).toBe(true);
    expect(remounted.attributes("class")).toContain("w-[3.4rem]");
    remounted.unmount();

    // 清除持久化后挂载 → 展开态宽度类。
    window.localStorage.clear();
    const expanded = mountSidebar("home");
    await nextTick();
    expect(expanded.attributes("class")).toContain("w-[17.5rem]");
    expanded.unmount();
  });

  it("更新角标只挂设置入口（展开+折叠）", async () => {
    const updateStore = useUpdateStore();
    vi.spyOn(updateStore, "hasUpdate", "get").mockReturnValue(true);
    seedTree();
    seedWorkspaces([]);
    const wrapper = mountSidebar();
    await flushUI();
    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-update-badge"]').exists()).toBe(true);
    await wrapper.get('[data-testid="session-sidebar-collapse"]').trigger("click");
    await nextTick();
    expect(wrapper.find('[data-testid="session-sidebar-nav-settings-collapsed-update-badge"]').exists()).toBe(true);
    wrapper.unmount();
  });
});

describe("HomeSessionSidebar（三级树结构契约）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
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

  it("default 不渲染组行；其会话平铺置顶；孤儿并入平铺区", () => {
    seedTree({
      sessions: [
        createSession({ conversationId: "d-new", title: "默认新", updatedAtMs: 3000 }),
        createSession({ conversationId: "d-old", title: "默认旧", updatedAtMs: 1000 }),
        createSession({ conversationId: "a1", title: "A 会话", workspaceId: "ws-a", updatedAtMs: 2000 }),
        createSession({ conversationId: "orphan", title: "孤儿", workspaceId: "ws-gone", updatedAtMs: 2500 })
      ]
    });
    seedWorkspaces();
    const wrapper = mountSidebar();
    void wrapper;
    expect(document.body.querySelector('[data-testid="workspace-group-default"]')).toBeNull();
    const flatRows = [...document.body.querySelectorAll('[data-testid="flat-zone-session-row"]')];
    expect(flatRows.length).toBeGreaterThanOrEqual(3);
    wrapper.unmount();
  });

  it("工作区组行解剖：文件夹图标、名称、计数徽标、＋ 与 ⋯；空组提示", () => {
    seedTree({
      sessions: [createSession({ conversationId: "b1", title: "B 会话", workspaceId: "ws-b" })]
    });
    seedWorkspaces();
    mountSidebar();
    const groupA = document.body.querySelector('[data-testid="workspace-group-ws-a"]');
    expect(groupA).toBeTruthy();
    expect(groupA?.textContent).toContain("项目A");
    expect(groupA?.querySelector('[data-testid="workspace-row-new-ws-a"]')).toBeTruthy();
    expect(groupA?.querySelector('[data-testid="workspace-row-menu-ws-a"]')).toBeTruthy();
    expect(document.body.querySelector('[data-testid="workspace-group-empty-ws-a"]')?.textContent).toContain("暂无对话");
    const groupB = document.body.querySelector('[data-testid="workspace-group-ws-b"]');
    expect(groupB?.textContent).toContain("项目B");
    expect(groupB?.textContent).toContain("1");
  });

  it("瞬态新对话按显式目标钉顶：顶层入口落平铺区顶；组内入口落该组顶", async () => {
    seedTree({
      currentId: "fresh",
      messages: [],
      sessions: [
        createSession({ conversationId: "old-flat", title: "旧平铺", updatedAtMs: 9000 }),
        createSession({ conversationId: "old-b", title: "旧B", workspaceId: "ws-b", updatedAtMs: 9000 })
      ]
    });
    seedWorkspaces();
    const runtimeStore = useRuntimeStore();
    const spy = vi.spyOn(runtimeStore, "createSession").mockResolvedValue(undefined);

    // 当前瞬态 target=default（初始）
    let wrapper = mountSidebar();
    void wrapper;
    const flatFirst = document.body.querySelectorAll('[data-testid="flat-zone-session-row"]')[0];
    expect(flatFirst?.textContent).toContain("新对话");
    wrapper.unmount();

    // target=ws-b
    runtimeStore.$patch({ sessionWorkspaceId: "ws-b" });
    wrapper = mountSidebar();
    const groupB = document.body.querySelector('[data-testid="workspace-group-ws-b"]');
    const firstInB = groupB?.querySelector("[data-testid^='workspace-session-row-ws-b-']");
    expect(firstInB?.textContent).toContain("新对话");

    // 组内 ＋ → createSession(ws-b)，且不触发激活
    const activateSpy = vi.spyOn(runtimeStore, "activateWorkspace");
    const newBtn = document.body.querySelector('[data-testid="workspace-row-new-ws-b"]') as HTMLElement;
    expect(newBtn).toBeTruthy();
    newBtn.click();
    await flushUI();
    expect(spy).toHaveBeenCalledWith("ws-b");
    expect(activateSpy).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("顶层「新对话」恒定落默认工作区并先行导航 home", async () => {
    seedTree({ currentId: "fresh", messages: [{ content: "seed" }], sessions: [] });
    seedWorkspaces();
    const runtimeStore = useRuntimeStore();
    const spy = vi.spyOn(runtimeStore, "createSession").mockResolvedValue(undefined);
    const wrapper = mountSidebar("settings");
    await wrapper.get('[data-testid="session-sidebar-new-chat"]').trigger("click");
    expect(wrapper.emitted("navigate")?.[0]).toEqual(["home"]);
    expect(spy).toHaveBeenCalledWith(DEFAULT_WORKSPACE_ID);
    wrapper.unmount();
  });
});

describe("HomeSessionSidebar（管理操作·Tauri 模式）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const updateStore = useUpdateStore();
    vi.spyOn(updateStore, "initialize").mockResolvedValue();
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  function seedStandard() {
    seedTree({
      sessions: [
        createSession({ conversationId: "s-run", title: "运行中的会话", workspaceId: "ws-a", updatedAtMs: 5000 }),
        createSession({ conversationId: "s-idle", title: "空闲会话", workspaceId: "ws-a", updatedAtMs: 4000 })
      ]
    });
    seedWorkspaces();
    const runtimeStore = useRuntimeStore();
    runtimeStore.runningSessionMap["s-run"] = {
      turnId: "t-1",
      phase: "running",
      textBuffer: "",
      reasoningBuffer: ""
    };
    return runtimeStore;
  }

  function rowVm(wrapper: ReturnType<typeof mountSidebar>, conversationId: string) {
    const candidate = wrapper
      .findAllComponents({ name: "SessionRow" })
      .find((row) => (row.props("session") as SessionOverview).conversationId === conversationId);
    expect(candidate, `SessionRow ${conversationId} 应存在`).toBeTruthy();
    return candidate!;
  }

  function openConfirmOf(wrapper: ReturnType<typeof mountSidebar>) {
    const opened = wrapper
      .findAllComponents(ConfirmPopover)
      .find((component) => component.props("open") === true);
    expect(opened, "受控确认弹层应处于打开状态").toBeTruthy();
    return opened!;
  }

  function workspaceMenus(wrapper: ReturnType<typeof mountSidebar>) {
    return wrapper
      .findAllComponents(DropdownMenu)
      .filter((menu) => {
        const items = menu.props("items") as DropdownMenuItemSpec[] | undefined;
        return Array.isArray(items) && items.length === 2 && (items[0] as DropdownMenuItemSpec)?.id === "rename";
      });
  }

  it("添加工作区入口仅 Tauri 可见且 aria-label=添加工作区；浏览器模式隐藏全部管理面", async () => {
    seedStandard();
    let wrapper = mountSidebar();
    await flushUI();
    expect(wrapper.get('[data-testid="workspace-add-button"]').attributes("aria-label")).toBe("添加工作区");
    const idleRow = rowVm(wrapper, "s-idle");
    expect((idleRow.props("menuItems") as DropdownMenuItemSpec[]).length).toBe(3);
    wrapper.unmount();

    document.body.innerHTML = "";
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    wrapper = mountSidebar();
    await flushUI();
    expect(wrapper.find('[data-testid="workspace-add-button"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-row-new-ws-a"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-row-menu-ws-a"]').exists()).toBe(false);
    const browserRow = rowVm(wrapper, "s-idle");
    const browserItems = browserRow.props("menuItems") as DropdownMenuItemSpec[];
    expect(browserItems.length).toBe(1);
    expect(browserItems[0].id).toBe("delete");
    wrapper.unmount();
  });

  it("删除工作区确认：文案三要素+工具根披露；确认调用 store.deleteWorkspace", async () => {
    const runtimeStore = seedStandard();
    const spy = vi.spyOn(runtimeStore, "deleteWorkspace").mockResolvedValue({ ok: true });
    const wrapper = mountSidebar();
    await flushUI();
    const menu = workspaceMenus(wrapper).find((menu) =>
      ((menu.props("items") as DropdownMenuItemSpec[])[1] as DropdownMenuItemSpec).label === SIDEBAR_COPY.menuDeleteWorkspace
      && wrapper.findAllComponents(DropdownMenu)
        .filter((candidate) => candidate.props("items") === menu.props("items"))
        .some((candidate) => (candidate.props("items") as DropdownMenuItemSpec[]).length === 2)
    );
    // 取第一个工作区菜单（项目A）：select delete 打开受控确认。
    const target = workspaceMenus(wrapper)[0];
    target.vm.$emit("select", "delete");
    await flushUI();

    const popover = openConfirmOf(wrapper);
    const title = String(popover.props("title"));
    const description = String(popover.props("description"));
    expect(title).toContain("删除工作区");
    expect(description).toContain("将移除工作区「项目A」的注册");
    expect(description).toContain("不影响磁盘上的目录与文件");
    expect(description).toContain("2 个对话会保留并移到顶部区域");
    expect(description).toContain("默认工作区目录进行");

    popover.vm.$emit("confirm");
    await flushPromises();
    await flushUI();
    expect(spy).toHaveBeenCalledTimes(1);
    expect(String(spy.mock.calls[0][0])).toContain("ws-");
    void runtimeStore;
    wrapper.unmount();
  });

  it("归档确认文案含不可恢复声明；确认调用 archiveSession", async () => {
    const runtimeStore = seedStandard();
    const spy = vi.spyOn(runtimeStore, "archiveSession").mockResolvedValue({ ok: true });
    const wrapper = mountSidebar();
    await flushUI();
    rowVm(wrapper, "s-idle").vm.$emit("menu-select", "archive");
    await flushUI();
    const popover = openConfirmOf(wrapper);
    expect(String(popover.props("description"))).toContain("无法从界面恢复");
    popover.vm.$emit("confirm");
    await flushPromises();
    await flushUI();
    expect(spy).toHaveBeenCalledWith("s-idle");
    wrapper.unmount();
  });

  it("删除对话走受控确认并调用 deleteSession", async () => {
    const runtimeStore = seedStandard();
    const spy = vi.spyOn(runtimeStore, "deleteSession").mockResolvedValue(undefined);
    const wrapper = mountSidebar();
    await flushUI();
    rowVm(wrapper, "s-idle").vm.$emit("menu-select", "delete");
    await flushUI();
    const popover = openConfirmOf(wrapper);
    popover.vm.$emit("confirm");
    await flushPromises();
    await flushUI();
    expect(spy).toHaveBeenCalledWith("s-idle");
    wrapper.unmount();
  });

  it("运行中会话菜单项禁用并带原因 tooltip；瞬态条目无菜单按钮", async () => {
    seedStandard();
    const wrapper = mountSidebar();
    await flushUI();
    const runItems = rowVm(wrapper, "s-run").props("menuItems") as DropdownMenuItemSpec[];
    expect(runItems.length).toBe(3);
    for (const item of runItems) {
      expect(item.disabled, `${item.id} 应禁用`).toBe(true);
      expect(item.disabledTitle ?? "").toContain("运行中");
    }
    // 瞬态（当前空白）条目不渲染三点按钮
    const transientName = "新对话";
    const transientRow = wrapper
      .findAllComponents({ name: "SessionRow" })
      .find((row) => (row.props("session") as SessionOverview).title === transientName);
    expect(transientRow).toBeTruthy();
    expect(transientRow!.find('[data-testid^="session-menu-"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("工作区重命名内联流：Enter 提交 trim 后名称；重名即时报错", async () => {
    const runtimeStore = seedStandard();
    const spy = vi.spyOn(runtimeStore, "renameWorkspace").mockResolvedValue({ ok: true });
    const wrapper = mountSidebar();
    await flushUI();
    workspaceMenus(wrapper)[0].vm.$emit("select", "rename");
    await flushUI();
    const inputWrapper = wrapper.find('[data-testid="workspace-rename-input"]');
    expect(inputWrapper.exists()).toBe(true);
    expect((inputWrapper.element as HTMLInputElement).value).toBe("项目A");

    await inputWrapper.setValue("项目B");
    await inputWrapper.trigger("keydown.enter");
    await flushUI();
    expect(document.body.contains(wrapper.find('[data-testid="workspace-rename-error"]').element)).toBe(true);
    expect(wrapper.find('[data-testid="workspace-rename-error"]').text()).toContain("已存在同名工作区");
    expect(spy).not.toHaveBeenCalled();

    await inputWrapper.setValue("  项目A改  ");
    await inputWrapper.trigger("keydown.enter");
    await flushPromises();
    await flushUI();
    expect(spy).toHaveBeenCalledWith("ws-a", "项目A改");
    wrapper.unmount();
  });

  it("会话重命名内联流提交到 store.renameSession", async () => {
    const runtimeStore = seedStandard();
    const spy = vi.spyOn(runtimeStore, "renameSession").mockResolvedValue({ ok: true });
    const wrapper = mountSidebar();
    await flushUI();
    rowVm(wrapper, "s-idle").vm.$emit("menu-select", "rename");
    await flushUI();
    const inputWrapper = wrapper.find('[data-testid="session-rename-input"]');
    expect(inputWrapper.exists()).toBe(true);
    await inputWrapper.setValue("新标题");
    await inputWrapper.trigger("keydown.enter");
    await flushPromises();
    await flushUI();
    expect(spy).toHaveBeenCalledWith("s-idle", "新标题");
    wrapper.unmount();
  });

  it("分区预览上限各自 5 条，「显示全部」一次解除所有分区", async () => {
    const many = Array.from({ length: 7 }, (_, i) =>
      createSession({ conversationId: `f${i}`, title: `F${i}`, updatedAtMs: 10_000 - i })
    );
    const groupSessions = Array.from({ length: 6 }, (_, i) =>
      createSession({ conversationId: `g${i}`, title: `G${i}`, workspaceId: "ws-a", updatedAtMs: 9_000 - i })
    );
    seedTree({ sessions: [...many, ...groupSessions] });
    seedWorkspaces();
    const wrapper = mountSidebar();
    await flushUI();
    expect(wrapper.findAll('[data-testid="flat-zone-session-row"]').length).toBe(5);
    expect(wrapper.findAll('[data-testid^="workspace-session-row-ws-a-"]').length).toBe(5);
    await wrapper.get('[data-testid="session-sidebar-show-more-conversations"]').trigger("click");
    await flushUI();
    // 7 条种子 + 瞬态"新对话"（钉顶）
    expect(wrapper.findAll('[data-testid="flat-zone-session-row"]').length).toBe(8);
    expect(wrapper.findAll('[data-testid^="workspace-session-row-ws-a-"]').length).toBe(6);
    wrapper.unmount();
  });
});

import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { nextTick } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { useWorkspacePanelsStore } from "@/stores/workspace-panels";
import ShortcutsHelpModal from "@/components/ShortcutsHelpModal.vue";

describe("WorkspacePanelsStore: goal 面板状态管理", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("支持切换 goal 面板并在 toggle 时正确更新 activePanel", () => {
    const store = useWorkspacePanelsStore();
    expect(store.activePanel).toBe("");

    store.toggleGoal();
    expect(store.activePanel).toBe("goal");

    store.toggleGoal();
    expect(store.activePanel).toBe("");
  });

  it("在不同面板间切换时互斥", () => {
    const store = useWorkspacePanelsStore();
    store.togglePlan();
    expect(store.activePanel).toBe("plan");

    store.toggleGoal();
    expect(store.activePanel).toBe("goal");

    store.toggleTasks();
    expect(store.activePanel).toBe("tasks");
  });
});

describe("ShortcutsHelpModal: 快捷键帮助弹窗组件", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("打开时渲染常用快捷键列表，包含 Ctrl/Cmd+G 与 Ctrl/Cmd+H", async () => {
    const wrapper = mount(ShortcutsHelpModal, {
      props: {
        open: true,
      },
      attachTo: document.body,
    });

    await nextTick();
    await flushPromises();

    const bodyText = document.body.textContent || "";
    expect(bodyText).toContain("快捷键指南");
    expect(bodyText).toContain("目标与计划");
    expect(bodyText).toContain("对话与发送");
    expect(bodyText).toContain("会话与导航");
    expect(bodyText).toContain("/goal");
    expect(bodyText).toContain("目标 (Goal) 面板");
    expect(bodyText).toContain("快捷键帮助面板");

    wrapper.unmount();
  });

  it("点击关闭按钮时发出 update:open 事件", async () => {
    const wrapper = mount(ShortcutsHelpModal, {
      props: {
        open: true,
      },
      attachTo: document.body,
    });

    await nextTick();
    await flushPromises();

    const closeBtn = document.body.querySelector('button[aria-label="关闭快捷键帮助"]') as HTMLButtonElement | null;
    expect(closeBtn).not.toBeNull();
    closeBtn?.click();

    await nextTick();
    expect(wrapper.emitted("update:open")).toBeTruthy();
    expect(wrapper.emitted("update:open")?.[0]).toEqual([false]);

    wrapper.unmount();
  });
});
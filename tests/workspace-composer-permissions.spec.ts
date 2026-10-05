import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { TooltipProvider } from "reka-ui";

function mountComposer() {
  return mount(
    {
      components: { WorkspaceComposer, TooltipProvider },
      template: `
        <TooltipProvider>
          <WorkspaceComposer
            :show-reasoning-content="false"
            :can-undo-last-turn="false"
            undo-shortcut-label="Ctrl+Z"
            :handle-composer-keydown="() => {}"
            :handle-primary-action="() => {}"
            :handle-undo-last-turn="() => {}"
            :set-composer-shell-ref="() => {}"
          />
        </TooltipProvider>
      `
    },
    {
      attachTo: document.body
    }
  );
}

describe("WorkspaceComposer permission modes", () => {
  it("defaults to workspace_write mode and displays 工作区修改", () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const runtimeStore = useRuntimeStore();

    expect(runtimeStore.toolAuthorizationMode).toBe("workspace_write");

    const wrapper = mountComposer();
    const trigger = wrapper.find('[data-testid="workspace-auth-mode-trigger"]');
    expect(trigger.exists()).toBe(true);
    expect(trigger.text()).toContain("工作区修改");
    wrapper.unmount();
  });

  it("can switch to read_only mode directly", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const runtimeStore = useRuntimeStore();

    const wrapper = mountComposer();
    const trigger = wrapper.find('[data-testid="workspace-auth-mode-trigger"]');
    await trigger.trigger("click");

    const readOnlyItem = document.querySelector('[data-testid="workspace-auth-mode-item-read_only"]');
    expect(readOnlyItem).not.toBeNull();
    expect(readOnlyItem?.textContent).toContain("工作区查看");

    readOnlyItem?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(runtimeStore.toolAuthorizationMode).toBe("read_only");
    wrapper.unmount();
  });

  it("shows confirmation dialog when switching to full_access and updates store upon confirm", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const runtimeStore = useRuntimeStore();

    const wrapper = mountComposer();
    const trigger = wrapper.find('[data-testid="workspace-auth-mode-trigger"]');
    await trigger.trigger("click");

    const fullAccessItem = document.querySelector('[data-testid="workspace-auth-mode-item-full_access"]');
    expect(fullAccessItem).not.toBeNull();
    expect(fullAccessItem?.textContent).toContain("完全权限");

    fullAccessItem?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await wrapper.vm.$nextTick();
    // 尚未确认，状态不应立即变为 full_access
    expect(runtimeStore.toolAuthorizationMode).toBe("workspace_write");

    const confirmBtn = document.querySelector('[data-testid="confirm-full-access-button"]');
    expect(confirmBtn).not.toBeNull();
    expect(document.body.textContent).toContain("确认启用完全权限？");

    confirmBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(runtimeStore.toolAuthorizationMode).toBe("full_access");
    wrapper.unmount();
  });
});

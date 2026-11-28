import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import { TooltipProvider } from "reka-ui";
import { useRuntimeStore } from "@/stores/runtime";

describe("Workspace Width Alignment", () => {
  it("ensures WorkspaceComposer shell has max-w-[46.4rem] matching the message flow column", () => {
    const pinia = createPinia();
    setActivePinia(pinia);

    const wrapper = mount(
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
      { attachTo: document.body }
    );

    const composerShell = wrapper.find('[data-testid="workspace-composer-shell"]');
    expect(composerShell.exists()).toBe(true);
    expect(composerShell.classes()).toContain("max-w-[46.4rem]");
    expect(composerShell.classes()).not.toContain("max-w-[38.4rem]");
    wrapper.unmount();
  });

});

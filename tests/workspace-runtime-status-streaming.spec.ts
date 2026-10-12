import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { TooltipProvider } from "reka-ui";

describe("Workspace Runtime Status Streaming Behavior", () => {
  it("does not show runtime status bar when assistant is streaming content", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const runtimeStore = useRuntimeStore();

    runtimeStore.sessionId = "sess-1";
    runtimeStore.activeTurnId = "turn-1";
    runtimeStore.isSubmitting = true;
    runtimeStore.currentPhase = "calling_model";

    // 活跃的回合，且 assistant 正在流式输出内容
    runtimeStore.messages = [
      {
        id: "msg-user-1",
        turnId: "turn-1",
        role: "user",
        content: "你好"
      },
      {
        id: "msg-assistant-1",
        turnId: "turn-1",
        role: "assistant",
        content: "正在为你生成回答的第一部分...",
        status: "pending"
      }
    ];

    const wrapper = mount(
      {
        components: { HomeWorkspace, TooltipProvider },
        template: `
          <TooltipProvider>
            <HomeWorkspace />
          </TooltipProvider>
        `
      },
      {
        global: { plugins: [pinia] },
        attachTo: document.body
      }
    );

    // 验证状态槽不显示
    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(false);

    wrapper.unmount();
  }, 15000);

  it("shows runtime status bar when executing tool before streaming content", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const runtimeStore = useRuntimeStore();

    runtimeStore.sessionId = "sess-1";
    runtimeStore.activeTurnId = "turn-1";
    runtimeStore.isSubmitting = true;
    runtimeStore.currentPhase = "calling_tool";

    runtimeStore.messages = [
      {
        id: "msg-user-1",
        turnId: "turn-1",
        role: "user",
        content: "查看文件"
      }
    ];
    runtimeStore.toolActivities = [
      {
        id: "act-1",
        turnId: "turn-1",
        name: "read_file",
        status: "running"
      }
    ];

    const wrapper = mount(
      {
        components: { HomeWorkspace, TooltipProvider },
        template: `
          <TooltipProvider>
            <HomeWorkspace />
          </TooltipProvider>
        `
      },
      {
        global: { plugins: [pinia] },
        attachTo: document.body
      }
    );

    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(true);
    expect(statusSlot.text()).toContain("执行工具");

    wrapper.unmount();
  }, 15000);

  it("shows runtime status bar when tool is drafting even if previous assistant message has pending status", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const runtimeStore = useRuntimeStore();

    runtimeStore.sessionId = "sess-1";
    runtimeStore.activeTurnId = "turn-1";
    runtimeStore.isSubmitting = true;
    runtimeStore.currentPhase = "calling_model";

    runtimeStore.messages = [
      {
        id: "msg-user-1",
        turnId: "turn-1",
        role: "user",
        content: "读一下文件"
      },
      {
        id: "msg-assistant-1",
        turnId: "turn-1",
        role: "assistant",
        content: "",
        status: "pending"
      }
    ];
    runtimeStore.toolActivities = [
      {
        id: "act-1",
        turnId: "turn-1",
        name: "read_file",
        canonicalToolName: "Read",
        displayNameZh: "读取文件",
        status: "planned"
      }
    ];

    const wrapper = mount(
      {
        components: { HomeWorkspace, TooltipProvider },
        template: `
          <TooltipProvider>
            <HomeWorkspace />
          </TooltipProvider>
        `
      },
      {
        global: { plugins: [pinia] },
        attachTo: document.body
      }
    );

    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(true);
    expect(statusSlot.text()).toContain("准备调用工具");

    wrapper.unmount();
  }, 15000);
});

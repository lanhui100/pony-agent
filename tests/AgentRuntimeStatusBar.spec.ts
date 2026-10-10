import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import AgentRuntimeStatusBar from "@/components/chat/AgentRuntimeStatusBar.vue";
import type { AgentRuntimeStatus } from "@/lib/runtime/status";

describe("AgentRuntimeStatusBar", () => {
  it("renders status label and icon correctly", () => {
    const status: AgentRuntimeStatus = {
      mode: "thinking",
      label: "思考中...",
      startTime: Date.now() - 3500,
    };

    const wrapper = mount(AgentRuntimeStatusBar, {
      props: { status },
    });

    expect(wrapper.find('[data-testid="agent-runtime-status-bar"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="agent-runtime-status-label"]').text()).toBe("思考中...");
    expect(wrapper.find('[data-testid="agent-runtime-status-elapsed"]').text()).toMatch(/\d+s/);
  });

  it("handles executing tool mode", () => {
    const status: AgentRuntimeStatus = {
      mode: "executing_tool",
      label: "执行工具: bash...",
      toolName: "bash",
      startTime: null,
    };

    const wrapper = mount(AgentRuntimeStatusBar, {
      props: { status },
    });

    expect(wrapper.find('[data-testid="agent-runtime-status-label"]').text()).toBe("执行工具: bash...");
    expect(wrapper.find('[data-testid="agent-runtime-status-elapsed"]').exists()).toBe(false);
  });
});

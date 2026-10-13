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
    expect(wrapper.find('[data-testid="agent-runtime-status-label"]').text()).toBe("思考中");
    expect(wrapper.find('[data-testid="agent-runtime-status-label"]').classes()).toContain("runtime-status-shimmer");
    expect(wrapper.find('[data-testid="agent-runtime-status-elapsed"]').text()).toMatch(/\d+秒/);
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

  it("removes background, border, and animate-pulse for a gentle, unobtrusive appearance", () => {
    const status: AgentRuntimeStatus = {
      mode: "connecting",
      label: "准备就绪中...",
      startTime: null,
    };

    const wrapper = mount(AgentRuntimeStatusBar, {
      props: { status },
    });

    const bar = wrapper.find('[data-testid="agent-runtime-status-bar"]');
    const classList = bar.classes();
    // 验证去除背景与边框
    const hasBackground = classList.some((c) => c.startsWith("bg-"));
    const hasBorder = classList.some((c) => c === "border" || c.startsWith("border-"));
    expect(hasBackground).toBe(false);
    expect(hasBorder).toBe(false);
    // 验证去除 animate-pulse
    expect(classList).not.toContain("animate-pulse");
  });
});

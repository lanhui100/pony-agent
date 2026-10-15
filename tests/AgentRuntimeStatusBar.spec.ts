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

    const bar = wrapper.find('[data-testid="agent-runtime-status-bar"]');
    expect(bar.exists()).toBe(true);
    expect(bar.classes()).toContain("italic");
    expect(wrapper.find('[data-testid="agent-runtime-status-label"]').text()).toBe("思考中");
    expect(wrapper.find('[data-testid="agent-runtime-status-label"]').classes()).toContain("runtime-status-shimmer");
    expect(wrapper.find('[data-testid="agent-runtime-status-elapsed"]').text()).toMatch(/\d+秒/);
    const icon = wrapper.find('[data-testid="agent-runtime-status-icon"]');
    expect(icon.exists()).toBe(true);
    // Wave-11 契约（.dev-team/contracts/wave-11-status-flow.md §1 M5 / §2.9）：
    // 旧 snake-spinner 由纯 CSS 动效图标 runtime-status-orbit（ring + dot 子节点）替换
    expect(icon.classes()).toContain("runtime-status-orbit");
    expect(icon.classes()).not.toContain("snake-spinner");
    expect(icon.classes()).not.toContain("animate-spin");
    expect(icon.find('[data-testid*="ring"]').exists()).toBe(true);
    expect(icon.find('[data-testid*="dot"]').exists()).toBe(true);
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

  it("handles error mode with alert icon and distinct text styling", () => {
    const status: AgentRuntimeStatus = {
      mode: "error",
      label: "响应超时，已停止等待",
      startTime: null,
    };

    const wrapper = mount(AgentRuntimeStatusBar, {
      props: { status },
    });

    const label = wrapper.find('[data-testid="agent-runtime-status-label"]');
    expect(label.text()).toBe("响应超时，已停止等待");
    expect(label.classes()).toContain("text-rose-600");

    const icon = wrapper.find('[data-testid="agent-runtime-status-icon"]');
    expect(icon.exists()).toBe(true);
    // 错误态保留 AlertCircle，不带 orbit 动效类，也不残留旧 snake-spinner
    expect(icon.classes()).not.toContain("snake-spinner");
    expect(icon.classes()).not.toContain("runtime-status-orbit");
  });
});

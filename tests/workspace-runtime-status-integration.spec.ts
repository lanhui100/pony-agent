import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { defineComponent, h } from "vue";
import AgentRuntimeStatusBar from "@/components/chat/AgentRuntimeStatusBar.vue";
import type { AgentRuntimeStatus } from "@/lib/runtime/status";

const MessageFlowMock = defineComponent({
  props: {
    messages: {
      type: Array as () => Array<{ id: string; content: string }>,
      required: true,
    },
    runtimeStatus: {
      type: Object as () => AgentRuntimeStatus | null,
      default: null,
    },
  },
  setup(props) {
    return () =>
      h("div", { class: "message-flow" }, [
        ...props.messages.map((m) =>
          h("div", { key: m.id, class: "message-item", "data-message-id": m.id }, m.content)
        ),
        props.runtimeStatus
          ? h(
              "div",
              { "data-testid": "workspace-runtime-status-slot" },
              h(AgentRuntimeStatusBar, { status: props.runtimeStatus })
            )
          : null,
      ]);
  },
});

describe("Message Flow Bottom Status Integration", () => {
  it("renders status indicator below the lowest message when active", () => {
    const messages = [
      { id: "msg-1", content: "用户提问" },
      { id: "msg-2", content: "模型部分回答" },
    ];
    const runtimeStatus: AgentRuntimeStatus = {
      mode: "executing_tool",
      label: "执行工具: bash...",
      toolName: "bash",
      startTime: Date.now() - 2000,
    };

    const wrapper = mount(MessageFlowMock, {
      props: {
        messages,
        runtimeStatus,
      },
    });

    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(true);
    expect(statusSlot.find('[data-testid="agent-runtime-status-label"]').text()).toBe(
      "执行工具: bash"
    );

    const messageItems = wrapper.findAll(".message-item");
    expect(messageItems.length).toBe(2);
  });

  it("renders status indicator for tool drafting mode with fixed spinner icon", () => {
    const runtimeStatus: AgentRuntimeStatus = {
      mode: "tool_drafting",
      label: "准备调用工具: 执行命令...",
      toolName: "bash",
      canonicalToolName: "Run",
      startTime: Date.now() - 1000,
    };

    const wrapper = mount(MessageFlowMock, {
      props: {
        messages: [{ id: "msg-1", content: "请帮我运行测试" }],
        runtimeStatus,
      },
    });

    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(true);
    expect(statusSlot.find('[data-testid="agent-runtime-status-label"]').text()).toBe(
      "准备调用工具: 执行命令"
    );
    const icon = statusSlot.find('[data-testid="agent-runtime-status-icon"]');
    expect(icon.exists()).toBe(true);
    // Wave-11 契约（§1 M5）：图标改为 runtime-status-orbit（ring + dot），旧 snake-spinner 已移除
    expect(icon.classes()).toContain("runtime-status-orbit");
    expect(icon.classes()).not.toContain("snake-spinner");
    expect(icon.classes()).not.toContain("animate-spin");
    expect(icon.find('[data-testid*="ring"]').exists()).toBe(true);
    expect(icon.find('[data-testid*="dot"]').exists()).toBe(true);
  });

  it("does not render status indicator when idle", () => {
    const messages = [{ id: "msg-1", content: "对话结束" }];
    const wrapper = mount(MessageFlowMock, {
      props: {
        messages,
        runtimeStatus: null,
      },
    });

    expect(wrapper.find('[data-testid="workspace-runtime-status-slot"]').exists()).toBe(false);
  });

  it("renders status indicator for session operations (e.g. switching session)", () => {
    const runtimeStatus: AgentRuntimeStatus = {
      mode: "connecting",
      label: "正在切换对话…",
      startTime: null,
    };

    const wrapper = mount(MessageFlowMock, {
      props: {
        messages: [{ id: "msg-1", content: "前次对话" }],
        runtimeStatus,
      },
    });

    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(true);
    expect(statusSlot.find('[data-testid="agent-runtime-status-label"]').text()).toBe("正在切换对话…");
  });

  it("renders status indicator for error status at the bottom", () => {
    const runtimeStatus: AgentRuntimeStatus = {
      mode: "error",
      label: "响应超时，已停止等待",
      startTime: null,
    };

    const wrapper = mount(MessageFlowMock, {
      props: {
        messages: [{ id: "msg-1", content: "长时任务" }],
        runtimeStatus,
      },
    });

    const statusSlot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(statusSlot.exists()).toBe(true);
    expect(statusSlot.find('[data-testid="agent-runtime-status-label"]').text()).toBe("响应超时，已停止等待");
  });
});

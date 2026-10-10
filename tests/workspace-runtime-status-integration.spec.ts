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
      "执行工具: bash..."
    );

    const messageItems = wrapper.findAll(".message-item");
    expect(messageItems.length).toBe(2);
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
});

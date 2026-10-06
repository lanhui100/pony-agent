import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import QueuedMessagesBubble from "@/components/chat/QueuedMessagesBubble.vue";
import type { QueuedMessageItem } from "@/types/runtime";

describe("Stage 2 - Queued Messages Stack & Expand Acceptance", () => {
  const singleMessage: QueuedMessageItem[] = [
    {
      id: "msg-1",
      sessionId: "s1",
      content: "第一条排队消息",
      mode: "queue",
      createdAt: 1000
    }
  ];

  const multipleMessages: QueuedMessageItem[] = [
    {
      id: "msg-1",
      sessionId: "s1",
      content: "第一条排队消息",
      mode: "queue",
      createdAt: 1000
    },
    {
      id: "msg-2",
      sessionId: "s1",
      content: "第二条排队消息",
      mode: "queue",
      createdAt: 2000
    },
    {
      id: "msg-3",
      sessionId: "s1",
      content: "第三条插队消息",
      mode: "steer",
      createdAt: 3000
    }
  ];

  it("F2-1 & F2-7: renders nothing when queue is empty", () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: { messages: [] }
    });
    expect(wrapper.find("[data-testid='queued-messages-container']").exists()).toBe(false);
  });

  it("F2-1: renders a single grey user-like bubble with queue badge when length is 1", () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: { messages: singleMessage }
    });

    const container = wrapper.find("[data-testid='queued-messages-container']");
    expect(container.exists()).toBe(true);
    expect(wrapper.text()).toContain("第一条排队消息");
    expect(wrapper.text()).toContain("排队");
    // 不应有堆叠数量徽标
    expect(wrapper.find("[data-testid='queue-stack-count']").exists()).toBe(false);
  });

  it("F2-2: when length > 1 and collapsed (default), only index 0 message content is visible and stack badge shows count", () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: { messages: multipleMessages }
    });

    const container = wrapper.find("[data-testid='queued-messages-container']");
    expect(container.exists()).toBe(true);

    // 堆叠计数徽标存在
    const stackCount = wrapper.find("[data-testid='queue-stack-count']");
    expect(stackCount.exists()).toBe(true);
    expect(stackCount.text()).toContain("+2");

    // 折叠态：应标记处于折叠状态或未展开
    expect(wrapper.find("[data-expanded='true']").exists()).toBe(false);
    expect(wrapper.text()).toContain("第一条排队消息");
  });

  it("F2-3 & F2-4: expands on hover, shows all messages, and emits steer / remove events", async () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: { messages: multipleMessages }
    });

    const container = wrapper.find("[data-testid='queued-messages-container']");
    await container.trigger("mouseenter");

    // 展开态属性验证
    expect(wrapper.find("[data-expanded='true']").exists()).toBe(true);
    expect(wrapper.text()).toContain("第二条排队消息");
    expect(wrapper.text()).toContain("第三条插队消息");

    // 展开态可点击操作
    const steerBtn = wrapper.find("[data-testid='queue-steer-btn-0']");
    expect(steerBtn.exists()).toBe(true);
    await steerBtn.trigger("click");
    expect(wrapper.emitted("steer")?.[0]).toEqual(["msg-1"]);

    const removeBtn = wrapper.find("[data-testid='queue-remove-btn-1']");
    expect(removeBtn.exists()).toBe(true);
    await removeBtn.trigger("click");
    expect(wrapper.emitted("remove")?.[0]).toEqual(["msg-2"]);

    // 移开恢复折叠
    await container.trigger("mouseleave");
    expect(wrapper.find("[data-expanded='true']").exists()).toBe(false);
  });
});

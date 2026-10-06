import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import QueuedMessagesBubble from "@/components/chat/QueuedMessagesBubble.vue";
import type { QueuedMessageItem } from "@/types/runtime";

describe("QueuedMessagesBubble.vue", () => {
  const dummyMessages: QueuedMessageItem[] = [
    {
      id: "msg-1",
      sessionId: "s1",
      content: "First queued message",
      mode: "queue",
      createdAt: 1000,
    },
    {
      id: "msg-2",
      sessionId: "s1",
      content: "Second queued message",
      mode: "queue",
      createdAt: 2000,
    },
    {
      id: "msg-3",
      sessionId: "s1",
      content: "Third queued message",
      mode: "queue",
      createdAt: 3000,
    },
  ];

  it("renders stacked queued messages when length > 2", () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: {
        messages: dummyMessages,
      },
    });

    expect(wrapper.find("[data-testid='queued-messages-container']").exists()).toBe(true);
    expect(wrapper.findAll(".queue-bubble").length).toBe(3);

    // 检查第 1 条排队文本
    expect(wrapper.text()).toContain("First queued message");
  });

  it("emits steer event when ArrowUp button clicked", async () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: {
        messages: dummyMessages,
      },
    });

    const steerBtn = wrapper.find("[data-testid='queue-steer-btn-0']");
    expect(steerBtn.exists()).toBe(true);

    await steerBtn.trigger("click");
    expect(wrapper.emitted("steer")).toBeTruthy();
    expect(wrapper.emitted("steer")![0]).toEqual(["msg-1"]);
  });

  it("emits remove event when trash button clicked on expanded message", async () => {
    const wrapper = mount(QueuedMessagesBubble, {
      props: {
        messages: dummyMessages,
      },
    });

    const container = wrapper.find("[data-testid='queued-messages-container']");
    await container.trigger("mouseenter");

    const removeBtn = wrapper.find("[data-testid='queue-remove-btn-1']");
    expect(removeBtn.exists()).toBe(true);

    await removeBtn.trigger("click");
    expect(wrapper.emitted("remove")).toBeTruthy();
    expect(wrapper.emitted("remove")![0]).toEqual(["msg-2"]);
  });
});

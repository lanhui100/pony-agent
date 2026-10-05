import { describe, expect, it, beforeEach } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";

describe("runtime store - queue and steer message flow", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("enqueues regular and steer messages into pendingQueuedMessages", () => {
    const store = useRuntimeStore();

    // 默认 queue 模式
    store.enqueueMessage("Message 1");
    store.enqueueMessage("Message 2");

    expect(store.pendingQueuedMessages.length).toBe(2);
    expect(store.pendingQueuedMessages[0].content).toBe("Message 1");
    expect(store.pendingQueuedMessages[0].mode).toBe("queue");
    expect(store.pendingQueuedMessages[1].content).toBe("Message 2");

    // steer 模式直接插到队列首位
    store.enqueueMessage("Urgent steer", "steer");
    expect(store.pendingQueuedMessages.length).toBe(3);
    expect(store.pendingQueuedMessages[0].content).toBe("Urgent steer");
    expect(store.pendingQueuedMessages[0].mode).toBe("steer");
  });

  it("promotes an existing queued message to steer and moves it to head", () => {
    const store = useRuntimeStore();
    store.enqueueMessage("First");
    store.enqueueMessage("Second");
    store.enqueueMessage("Third");

    const secondId = store.pendingQueuedMessages[1].id;
    store.promoteToSteer(secondId);

    expect(store.pendingQueuedMessages[0].id).toBe(secondId);
    expect(store.pendingQueuedMessages[0].mode).toBe("steer");
    expect(store.pendingQueuedMessages[0].content).toBe("Second");
  });

  it("removes queued message by id", () => {
    const store = useRuntimeStore();
    store.enqueueMessage("A");
    store.enqueueMessage("B");

    const aId = store.pendingQueuedMessages[0].id;
    store.removeQueuedMessage(aId);

    expect(store.pendingQueuedMessages.length).toBe(1);
    expect(store.pendingQueuedMessages[0].content).toBe("B");
  });

  it("toggles delivery mode between queue and steer", () => {
    const store = useRuntimeStore();
    expect(store.queueDeliveryMode).toBe("queue");

    store.toggleQueueDeliveryMode();
    expect(store.queueDeliveryMode).toBe("steer");

    store.toggleQueueDeliveryMode();
    expect(store.queueDeliveryMode).toBe("queue");
  });
});

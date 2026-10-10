import { describe, it, expect, vi } from "vitest";
import { EventBus } from "@/lib/event-bus";

describe("EventBus (TypeScript) Contract & Behavior", () => {
  it("should successfully register listener and receive emitted event", () => {
    const bus = new EventBus();
    const handler = vi.fn();

    const unsubscribe = bus.on("test:event", handler);
    bus.emit("test:event", { value: 42 });

    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler).toHaveBeenCalledWith({ value: 42 });

    unsubscribe();
    bus.emit("test:event", { value: 100 });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("should handle once listener and auto-unsubscribe after first emission", () => {
    const bus = new EventBus();
    const handler = vi.fn();

    bus.once("test:once", handler);
    bus.emit("test:once", { msg: "first" });
    bus.emit("test:once", { msg: "second" });

    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler).toHaveBeenCalledWith({ msg: "first" });
  });

  it("should safely handle off and clear", () => {
    const bus = new EventBus();
    const handler1 = vi.fn();
    const handler2 = vi.fn();

    bus.on("evt1", handler1);
    bus.on("evt2", handler2);

    bus.off("evt1", handler1);
    bus.emit("evt1", "payload");
    expect(handler1).not.toHaveBeenCalled();

    bus.clear();
    bus.emit("evt2", "payload");
    expect(handler2).not.toHaveBeenCalled();
  });
});

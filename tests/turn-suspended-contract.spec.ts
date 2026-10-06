import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable,
}));

describe("three-faults turn:suspended contract (red-phase)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    tauriMocks.mockSafeInvoke.mockReset();
    tauriMocks.mockSafeListen.mockReset();
    tauriMocks.mockIsTauriAvailable.mockReset();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async () => () => {});
    window.localStorage.clear();
  });

  it("initializeTurnEvents subscribes to the terminal turn:suspended event", async () => {
    const subscribed: string[] = [];
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string) => {
      subscribed.push(eventName);
      return () => {};
    });

    const store = useRuntimeStore();
    await store.initializeTurnEvents();

    // 后端 RecordingTurnEventSink 已将 turn:suspended 视为终态；
    // 前端缺该监听会导致 Ask 挂起后永不解锁、120s 看门狗误判。
    expect(subscribed).toContain("turn:suspended");
  });
});

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useSettingsStore } from "@/stores/settings";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

describe("settings store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
  });

  it("loads persisted workspace mode from tauri settings", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce({
      workspaceMode: "work"
    });

    const store = useSettingsStore();
    await store.loadSettings();

    expect(store.workspaceMode).toBe("work");
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("load_app_settings");
  });

  it("saves workspace mode updates through tauri settings", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce({
      workspaceMode: "work"
    });

    const store = useSettingsStore();
    await store.setWorkspaceMode("work");

    expect(store.workspaceMode).toBe("work");
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("save_app_settings", {
      settings: {
        workspaceMode: "work"
      }
    });
    expect(store.notice).toBe("应用设置已保存。");
  });
});

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
    // Wave43 D7：落盘载荷必须同时携带全局 trace 上报开关（缺省 false）。
    // 键逐一列出（禁止用 objectContaining 放宽到失去约束）。
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("save_app_settings", {
      settings: {
        workspaceMode: "work",
        agentTraceReporting: false
      }
    });
    expect(store.notice).toBe("应用设置已保存。");
  });

  it("load → normalize → save 往返保留前端未建模字段 toolRules（wave43-B7 回归）", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);

    // Arrange：Rust 侧 AppSettings 含前端未建模的 `toolRules`（camelCase 落盘键）。
    const persisted = {
      workspaceMode: "work",
      agentTraceReporting: true,
      toolRules: [{ matcher: "bash", action: "deny", reason: "guard" }]
    };
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce(persisted); // load_app_settings

    const store = useSettingsStore();
    await store.loadSettings();

    // Act：normalize 后必须保留未知键（若重建已知字段即静默丢数据）。
    expect((store.settings as unknown as Record<string, unknown>).toolRules).toEqual(
      persisted.toolRules
    );

    tauriMocks.mockSafeInvoke.mockResolvedValueOnce(persisted); // save_app_settings
    await store.saveSettings();

    // Assert：回写载荷必须仍带 toolRules（深比较），且两个已知键同时在场。
    expect(tauriMocks.mockSafeInvoke).toHaveBeenLastCalledWith("save_app_settings", {
      settings: expect.objectContaining({
        workspaceMode: "work",
        agentTraceReporting: true,
        toolRules: persisted.toolRules
      })
    });
  });
});

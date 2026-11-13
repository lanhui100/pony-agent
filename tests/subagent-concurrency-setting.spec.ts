import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import ConfigGeneralSection from "@/components/config/ConfigGeneralSection.vue";
import { useSettingsStore } from "@/stores/settings";

/**
 * B125 验收：子代理并发上限在设置页可读可改，且落盘走既有 `save_app_settings` 通道。
 *
 * 契约：
 * - 未配置时输入框显示默认值 4（与 Rust `DEFAULT_SUBAGENT_MAX_CONCURRENT` 对齐）；
 * - 失焦/回车触发保存，入参 `subagentMaxConcurrent` 为数字；
 * - 越界值以**后端回显**为真值（mock 模拟 clamp 到 1..=8），输入框随后显示生效值。
 */

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

/** 模拟后端保存：clamp 到 1..=8 并回显（与 Rust `save_view` 同口径）。 */
function mockBackendSaveAppSettings() {
  tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === "save_app_settings") {
      const payload = args as { settings?: Record<string, unknown> } | undefined;
      const settings = { ...(payload?.settings ?? {}) };
      const raw = settings.subagentMaxConcurrent;
      if (typeof raw === "number" && Number.isFinite(raw)) {
        settings.subagentMaxConcurrent = Math.min(8, Math.max(1, Math.round(raw)));
      }
      return settings;
    }
    return "";
  });
}

describe("B125 子代理并发上限设置", () => {
  enableAutoUnmount(afterEach);

  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockSafeInvoke.mockResolvedValue("");
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("未配置时显示默认值 4，并标注可调范围", () => {
    const wrapper = mount(ConfigGeneralSection);

    const section = wrapper.get('[data-testid="config-subagent-concurrency-section"]');
    expect(section.text()).toContain("子代理并发上限");
    expect(section.text()).toContain("1–8");

    const input = wrapper.get('[data-testid="config-subagent-concurrency-input"]');
    expect((input.element as HTMLInputElement).value).toBe("4");
    expect(useSettingsStore().effectiveSubagentMaxConcurrent).toBe(4);
  });

  it("失焦提交后调用 save_app_settings，入参为数字并发上限", async () => {
    mockBackendSaveAppSettings();
    const wrapper = mount(ConfigGeneralSection);
    const input = wrapper.get('[data-testid="config-subagent-concurrency-input"]');

    await input.setValue("6");
    await input.trigger("blur");
    await flushPromises();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "save_app_settings",
      expect.objectContaining({
        settings: expect.objectContaining({ subagentMaxConcurrent: 6 })
      })
    );
    expect(useSettingsStore().settings.subagentMaxConcurrent).toBe(6);
    expect(wrapper.get('[data-testid="config-subagent-concurrency-notice"]').text()).toContain(
      "已保存"
    );
  });

  it("越界输入以后端 clamp 回显为真值（99 → 8）", async () => {
    mockBackendSaveAppSettings();
    const wrapper = mount(ConfigGeneralSection);
    const input = wrapper.get('[data-testid="config-subagent-concurrency-input"]');

    await input.setValue("99");
    await input.trigger("blur");
    await flushPromises();

    expect(useSettingsStore().settings.subagentMaxConcurrent).toBe(8);
    expect((wrapper.get('[data-testid="config-subagent-concurrency-input"]').element as HTMLInputElement).value).toBe(
      "8"
    );
  });

  it("回车提交与失焦同语义，且非法空值恢复原值不写盘", async () => {
    mockBackendSaveAppSettings();
    const wrapper = mount(ConfigGeneralSection);
    const input = wrapper.get('[data-testid="config-subagent-concurrency-input"]');

    await input.setValue("5");
    await input.trigger("keydown", { key: "Enter" });
    await flushPromises();
    expect(useSettingsStore().settings.subagentMaxConcurrent).toBe(5);

    tauriMocks.mockSafeInvoke.mockClear();
    await input.setValue("");
    await input.trigger("blur");
    await flushPromises();
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalledWith(
      "save_app_settings",
      expect.anything()
    );
    expect((input.element as HTMLInputElement).value).toBe("5");
  });
});

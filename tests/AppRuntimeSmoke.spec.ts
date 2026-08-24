import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import App from "@/App.vue";
import { useUpdateStore } from "@/stores/update";

// 诊断用：真实组件链（无业务组件 stub）挂载 App，抓白屏类运行时异常。
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

describe("App runtime smoke (real components)", () => {
  enableAutoUnmount(afterEach);

  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const updateStore = useUpdateStore();
    vi.spyOn(updateStore, "initialize").mockResolvedValue();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("mounts the real home page without runtime errors", async () => {
    const errors: string[] = [];
    const warnSpy = vi.spyOn(console, "error").mockImplementation(((...args: unknown[]) => {
      errors.push(args.map(String).join(" "));
    }) as typeof console.error);

    const wrapper = mount(App);
    await new Promise((resolve) => setTimeout(resolve, 50));
    await Promise.resolve();

    expect(wrapper.find('[data-testid="session-sidebar-brand"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-right-sidebar-toggle"]').exists()).toBe(true);
    const fatal = errors.filter((e) => /Unhandled|TypeError|ReferenceError|RangeError|Cannot read/.test(e));
    expect(fatal, fatal.join("\n---\n")).toEqual([]);
    warnSpy.mockRestore();
  });

  it("mounts the real config page models tab (ProviderConfigPage) without runtime errors", async () => {
    const errors: string[] = [];
    const warnSpy = vi.spyOn(console, "error").mockImplementation(((...args: unknown[]) => {
      errors.push(args.map(String).join(" "));
    }) as typeof console.error);

    const wrapper = mount(App);
    await new Promise((resolve) => setTimeout(resolve, 50));

    // 观测入口 → 真实观测页（观测按钮仅存在于 home 布局，需在进入配置页前验证）
    await wrapper.get('[data-testid="workspace-observation-toggle"]').trigger("click");
    await Promise.resolve();
    expect(wrapper.find('[data-testid="telemetry-page"]').exists()).toBe(true);
    await wrapper.get('[data-testid="telemetry-back"]').trigger("click");
    await Promise.resolve();

    // 左栏 设置 → 配置页；再切到 模型 tab 挂载真实 ProviderConfigPage
    await wrapper.get('[data-testid="session-sidebar-nav-settings"]').trigger("click");
    await Promise.resolve();
    expect(wrapper.find('[data-testid="config-page"]').exists()).toBe(true);

    await wrapper.get('[data-testid="config-tab-models"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    expect(wrapper.find('[data-testid="provider-detail-section"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="model-list-section"]').exists()).toBe(true);

    const fatal = errors.filter((e) => /Unhandled|TypeError|ReferenceError|RangeError|Cannot read|Failed to resolve/.test(e));
    expect(fatal, fatal.join("\n---\n")).toEqual([]);
    warnSpy.mockRestore();
  });
});

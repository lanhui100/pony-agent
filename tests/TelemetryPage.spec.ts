import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent } from "vue";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import TelemetryPage from "@/components/telemetry/TelemetryPage.vue";
import { useSettingsStore } from "@/stores/settings";

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

const TraceInspectorStub = defineComponent({
  template: '<div data-testid="trace-inspector-stub">trace-inspector</div>'
});

const ModelMonitorPageStub = defineComponent({
  template: '<div data-testid="model-monitor-stub">model-monitor</div>'
});

async function mountTelemetry() {
  // attachTo：键盘漫游测试断言 document.activeElement，需真实挂载到文档。
  const wrapper = mount(TelemetryPage, {
    attachTo: document.body,
    global: {
      stubs: {
        TraceInspector: TraceInspectorStub,
        ModelMonitorPage: ModelMonitorPageStub
      }
    }
  });
  await Promise.resolve();
  return wrapper;
}

describe("TelemetryPage", () => {
  enableAutoUnmount(afterEach);

  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("coding 模式默认 Trace tab，双 tab 可切换到指标", async () => {
    const wrapper = await mountTelemetry();

    expect(wrapper.get('[data-testid="telemetry-panel-trace"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-inspector-stub"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="telemetry-tab-trace"]').attributes("aria-selected")).toBe("true");

    await wrapper.get('[data-testid="telemetry-tab-metrics"]').trigger("click");

    expect(wrapper.get('[data-testid="telemetry-panel-metrics"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="model-monitor-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-inspector-stub"]').exists()).toBe(false);
  });

  it("work 模式仅渲染指标单 tab，无 Trace 入口", async () => {
    const settingsStore = useSettingsStore();
    settingsStore.$patch({ settings: { workspaceMode: "work" } });

    const wrapper = await mountTelemetry();

    expect(wrapper.find('[data-testid="telemetry-tab-trace"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="telemetry-tab-metrics"]').attributes("aria-selected")).toBe("true");
    expect(wrapper.get('[data-testid="model-monitor-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-inspector-stub"]').exists()).toBe(false);
  });

  it("tab 可用性收敛：coding 停留 Trace 时切到 work 自动落指标（启动竞态回归）", async () => {
    const settingsStore = useSettingsStore();
    const wrapper = await mountTelemetry();

    expect(wrapper.get('[data-testid="trace-inspector-stub"]').exists()).toBe(true);

    settingsStore.$patch({ settings: { workspaceMode: "work" } });
    await Promise.resolve();
    await Promise.resolve();

    expect(wrapper.find('[data-testid="telemetry-tab-trace"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="model-monitor-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-inspector-stub"]').exists()).toBe(false);
  });

  it("work→coding 往返后停留指标 tab（PA-096 review 显式收敛语义）", async () => {
    const settingsStore = useSettingsStore();
    const wrapper = await mountTelemetry();

    settingsStore.$patch({ settings: { workspaceMode: "work" } });
    await Promise.resolve();
    await Promise.resolve();

    settingsStore.$patch({ settings: { workspaceMode: "coding" } });
    await Promise.resolve();
    await Promise.resolve();

    // Trace tab 恢复可见，但激活 tab 保持指标（不回跳 Trace）
    expect(wrapper.get('[data-testid="telemetry-tab-trace"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="telemetry-tab-metrics"]').attributes("aria-selected")).toBe("true");
    expect(wrapper.get('[data-testid="model-monitor-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="trace-inspector-stub"]').exists()).toBe(false);
  });

  it("work 模式页头为指标文案且不出现 Trace 字样（PA-096 review B-P2-1）", async () => {
    const settingsStore = useSettingsStore();
    settingsStore.$patch({ settings: { workspaceMode: "work" } });

    const wrapper = await mountTelemetry();

    expect(wrapper.get('[data-testid="telemetry-heading"]').text()).toBe("指标");
    expect(wrapper.text()).not.toContain("Trace 与指标的二级读面");
  });

  it("tablist 支持方向键/Home/End 漫游并迁移焦点（APG）", async () => {
    const wrapper = await mountTelemetry();

    const traceTab = wrapper.get('[data-testid="telemetry-tab-trace"]');
    const metricsTab = wrapper.get('[data-testid="telemetry-tab-metrics"]');

    await traceTab.trigger("keydown", { key: "ArrowRight" });
    expect(wrapper.get('[data-testid="telemetry-tab-metrics"]').attributes("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(metricsTab.element);

    await metricsTab.trigger("keydown", { key: "ArrowRight" });
    // 单指标+trace 双 tab：右移越界回绕到 trace
    expect(wrapper.get('[data-testid="telemetry-tab-trace"]').attributes("aria-selected")).toBe("true");

    await traceTab.trigger("keydown", { key: "End" });
    expect(wrapper.get('[data-testid="telemetry-tab-metrics"]').attributes("aria-selected")).toBe("true");

    await metricsTab.trigger("keydown", { key: "Home" });
    expect(wrapper.get('[data-testid="telemetry-tab-trace"]').attributes("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(traceTab.element);
  });

  it("返回按钮 emit navigate('home')", async () => {
    const wrapper = await mountTelemetry();

    await wrapper.get('[data-testid="telemetry-back"]').trigger("click");

    expect(wrapper.emitted("navigate")).toEqual([["home"]]);
  });
});

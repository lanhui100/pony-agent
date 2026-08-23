import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent } from "vue";
import { mount } from "@vue/test-utils";
import ConfigPage from "@/components/config/ConfigPage.vue";
import type { ConfigTab } from "@/types/config";

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

const ConfigGeneralSectionStub = defineComponent({
  template: '<div data-testid="config-general-stub">general</div>'
});

const ProviderConfigPageStub = defineComponent({
  template: '<div data-testid="provider-config-stub">provider-config</div>'
});

const ConfigToolsSectionStub = defineComponent({
  template: '<div data-testid="config-tools-stub">tools</div>'
});

async function mountConfig(tab: ConfigTab = "general") {
  const wrapper = mount(ConfigPage, {
    props: { tab },
    global: {
      stubs: {
        ConfigGeneralSection: ConfigGeneralSectionStub,
        ProviderConfigPage: ProviderConfigPageStub,
        ConfigToolsSection: ConfigToolsSectionStub
      }
    }
  });
  await Promise.resolve();
  return wrapper;
}

describe("ConfigPage", () => {
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

  it("默认渲染通用 tab，三个 tab 受控切换且面板互斥懒挂载", async () => {
    const wrapper = await mountConfig("general");

    expect(wrapper.get('[data-testid="config-general-stub"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="config-tab-general"]').attributes("aria-selected")).toBe("true");

    await wrapper.get('[data-testid="config-tab-models"]').trigger("click");
    expect(wrapper.emitted("update:tab")).toEqual([["models"]]);
    await wrapper.setProps({ tab: "models" });

    expect(wrapper.get('[data-testid="provider-config-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="config-general-stub"]').exists()).toBe(false);

    await wrapper.get('[data-testid="config-tab-tools"]').trigger("click");
    expect(wrapper.emitted("update:tab")).toEqual([["models"], ["tools"]]);
    await wrapper.setProps({ tab: "tools" });

    expect(wrapper.get('[data-testid="config-tools-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="provider-config-stub"]').exists()).toBe(false);
  });

  it("非法 tab prop 防御性回退到通用 tab", async () => {
    const wrapper = await mountConfig("junk" as unknown as ConfigTab);

    expect(wrapper.get('[data-testid="config-general-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="provider-config-stub"]').exists()).toBe(false);
  });

  it("tab 键具备 APG 语义（role/aria-selected/aria-controls）", async () => {
    const wrapper = await mountConfig();

    const modelsTab = wrapper.get('[data-testid="config-tab-models"]');
    expect(modelsTab.attributes("role")).toBe("tab");
    expect(modelsTab.attributes("aria-controls")).toBe("config-panel-models");
    expect(wrapper.get('[data-testid="config-tablist"]').attributes("role")).toBe("tablist");
  });

  it("tablist 支持方向键/Home/End 漫游（APG，受控 emit 驱动）", async () => {
    const wrapper = await mountConfig("general");

    await wrapper.get('[data-testid="config-tab-general"]').trigger("keydown", { key: "ArrowRight" });
    expect(wrapper.emitted("update:tab")).toEqual([["models"]]);

    await wrapper.setProps({ tab: "models" });
    await wrapper.get('[data-testid="config-tab-models"]').trigger("keydown", { key: "End" });
    expect(wrapper.emitted("update:tab")).toEqual([["models"], ["tools"]]);

    await wrapper.setProps({ tab: "tools" });
    await wrapper.get('[data-testid="config-tab-tools"]').trigger("keydown", { key: "Home" });
    expect(wrapper.emitted("update:tab")).toEqual([["models"], ["tools"], ["general"]]);

    await wrapper.setProps({ tab: "general" });
    await wrapper.get('[data-testid="config-tab-general"]').trigger("keydown", { key: "ArrowLeft" });
    // 左移越界回绕到最后一项
    expect(wrapper.emitted("update:tab")).toEqual([["models"], ["tools"], ["general"], ["tools"]]);
  });
});

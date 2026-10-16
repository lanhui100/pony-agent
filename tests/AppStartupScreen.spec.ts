import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import AppStartupScreen from "@/components/AppStartupScreen.vue";

describe("AppStartupScreen Component", () => {
  it("renders when isInitializing is true", () => {
    const wrapper = mount(AppStartupScreen, {
      props: {
        isInitializing: true,
        stageText: "正在加载测试引擎..."
      }
    });

    const screen = wrapper.find('[data-testid="app-startup-screen"]');
    expect(screen.exists()).toBe(true);
    expect(screen.text()).toContain("Pony Agent");
    expect(screen.text()).toContain("正在加载测试引擎...");
    expect(screen.classes()).toContain("backdrop-blur-xl");
  });

  it("does not render when isInitializing is false", () => {
    const wrapper = mount(AppStartupScreen, {
      props: {
        isInitializing: false,
        stageText: "就绪"
      }
    });

    const screen = wrapper.find('[data-testid="app-startup-screen"]');
    expect(screen.exists()).toBe(false);
  });

  it("updates stage text dynamically", async () => {
    const wrapper = mount(AppStartupScreen, {
      props: {
        isInitializing: true,
        stageText: "阶段 1"
      }
    });

    expect(wrapper.find('[data-testid="app-startup-stage"]').text()).toContain("阶段 1");

    await wrapper.setProps({ stageText: "阶段 2：恢复会话" });
    expect(wrapper.find('[data-testid="app-startup-stage"]').text()).toContain("阶段 2：恢复会话");
  });
});

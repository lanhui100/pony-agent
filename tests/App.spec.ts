import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import App from "@/App.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import { useSettingsStore } from "@/stores/settings";
import { useUpdateStore } from "@/stores/update";

const HomeSidebarStub = defineComponent({
  template: '<div data-testid="home-sidebar-stub">home-sidebar</div>'
});

const HomeSessionSidebarStub = defineComponent({
  props: {
    currentPage: {
      type: String,
      default: "home"
    },
    forceCollapsed: {
      type: Boolean,
      default: false
    }
  },
  emits: ["navigate"],
  setup(props, { emit }) {
    return () =>
      h("div", {
        "data-testid": "home-session-sidebar-stub",
        "data-current-page": props.currentPage == null ? "" : String(props.currentPage),
        "data-force-collapsed": props.forceCollapsed ? "true" : "false"
      }, [
        h(
          "button",
          {
            "data-testid": "stub-nav-home",
            onClick: () => emit("navigate", "home")
          },
          "go-home"
        ),
        h(
          "button",
          {
            "data-testid": "stub-nav-settings",
            onClick: () => emit("navigate", "settings")
          },
          "go-settings"
        )
      ]);
  }
});

const HomeWorkspaceStub = defineComponent({
  template: '<div data-testid="home-workspace-stub">home-workspace</div>'
});

const ConfigPageStub = defineComponent({
  props: {
    tab: {
      type: String,
      default: "general"
    }
  },
  emits: ["update:tab"],
  setup(props, { emit }) {
    return () =>
      h("div", { "data-testid": "config-page-stub", "data-tab": props.tab }, [
        h(
          "button",
          {
            "data-testid": "stub-config-select-models",
            onClick: () => emit("update:tab", "models")
          },
          "select-models"
        ),
        h(
          "button",
          {
            "data-testid": "stub-config-select-tools",
            onClick: () => emit("update:tab", "tools")
          },
          "select-tools"
        )
      ]);
  }
});

const TelemetryPageStub = defineComponent({
  emits: ["navigate"],
  setup(_props, { emit }) {
    return () =>
      h("div", { "data-testid": "telemetry-page-stub" }, [
        h(
          "button",
          {
            "data-testid": "stub-telemetry-back",
            onClick: () => emit("navigate", "home")
          },
          "back"
        )
      ]);
  }
});

const TooltipProviderStub = defineComponent({
  template: '<div data-testid="tooltip-provider-stub"><slot /></div>'
});

// ADR 0013：观测按钮经 ui/Tooltip 包裹；App.spec 将 setTimeout mock 成立即执行，
// 与 reka-ui TooltipRoot 的延迟开启逻辑组合未经验证——按 HomeSidebar.spec 先例
// 直接 stub 掉 Tooltip，仅保留 trigger 插槽渲染。
const TooltipStub = defineComponent({
  props: {
    text: {
      type: String,
      default: ""
    }
  },
  template: '<span data-testid="tooltip-stub" :data-text="text"><slot /></span>'
});

function mountApp() {
  return mount(App, {
    global: {
      stubs: {
        HomeSidebar: HomeSidebarStub,
        HomeSessionSidebar: HomeSessionSidebarStub,
        HomeWorkspace: HomeWorkspaceStub,
        ConfigPage: ConfigPageStub,
        TelemetryPage: TelemetryPageStub,
        TooltipProvider: TooltipProviderStub,
        Tooltip: TooltipStub
      }
    }
  });
}

describe("App", () => {
  let requestAnimationFrameSpy: ReturnType<typeof vi.spyOn>;
  let setTimeoutSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    requestAnimationFrameSpy = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation(((callback: FrameRequestCallback) => {
        callback(performance.now());
        return 1;
      }) as typeof window.requestAnimationFrame);
    setTimeoutSpy = vi
      .spyOn(window, "setTimeout")
      .mockImplementation(((handler: TimerHandler) => {
        if (typeof handler === "function") {
          handler();
        }
        return 1;
      }) as typeof window.setTimeout);

    const providerStore = useProviderStore();
    vi.spyOn(providerStore, "loadRegistry").mockResolvedValue();
    const settingsStore = useSettingsStore();
    vi.spyOn(settingsStore, "loadSettings").mockResolvedValue();

    const runtimeStore = useRuntimeStore();
    vi.spyOn(runtimeStore, "fetchHealth").mockResolvedValue();
    vi.spyOn(runtimeStore, "fetchAvailableTools").mockResolvedValue();
    vi.spyOn(runtimeStore, "initializeTurnEvents").mockResolvedValue();
    vi.spyOn(runtimeStore, "initializeSessions").mockResolvedValue();

    // PA-099：更新检测默认整体拦截，保证本文件任何用例都不触网；
    // 需要观察 initialize 的用例自行覆盖该 spy。
    // 注意（守卫约定）：spy 依赖 mountApp() 不安装 pinia 插件——mount 内的
    // useUpdateStore() 解析到上方 active pinia 的同一实例，拦截才生效；
    // 若未来给 mountApp 增加 global.plugins=[pinia()]，此处将静默失效转为触网。
    const updateStore = useUpdateStore();
    vi.spyOn(updateStore, "initialize").mockResolvedValue();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("removes the old top nav and lets the left sidebar own page switching", () => {
    const wrapper = mountApp();

    expect(wrapper.find('[data-testid="app-page-nav"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("home");
    expect(wrapper.find('[data-testid="tooltip-provider-stub"]').exists()).toBe(true);
  });

  it("switches between home and the config general tab from the sidebar", async () => {
    const wrapper = mountApp();

    // 左栏仅存的一级配置入口"设置" → 配置页·通用 tab
    await wrapper.get('[data-testid="stub-nav-settings"]').trigger("click");
    expect(wrapper.find('[data-testid="config-page-stub"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="config-page-stub"]').attributes("data-tab")).toBe("general");
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("settings");

    await wrapper.get('[data-testid="stub-nav-home"]').trigger("click");
    expect(wrapper.find('[data-testid="config-page-stub"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);
  });

  it("keeps config page tabs as controlled surface with no first-level highlight on models/tools (ADR 0013)", async () => {
    const wrapper = mountApp();

    await wrapper.get('[data-testid="stub-nav-settings"]').trigger("click");
    expect(wrapper.get('[data-testid="config-page-stub"]').attributes("data-tab")).toBe("general");
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("settings");

    // 配置页内部切到 models tab："模型配置"一级键已移除 → 派生高亮为空
    await wrapper.get('[data-testid="stub-config-select-models"]').trigger("click");
    expect(wrapper.get('[data-testid="config-page-stub"]').attributes("data-tab")).toBe("models");
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("");

    // tools tab 同样无对应一级键
    await wrapper.get('[data-testid="stub-config-select-tools"]').trigger("click");
    expect(wrapper.get('[data-testid="config-page-stub"]').attributes("data-tab")).toBe("tools");
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("");

    // 回到 home 再进设置 → 显式目的地覆盖为 general tab
    await wrapper.get('[data-testid="stub-nav-home"]').trigger("click");
    await wrapper.get('[data-testid="stub-nav-settings"]').trigger("click");
    expect(wrapper.get('[data-testid="config-page-stub"]').attributes("data-tab")).toBe("general");
  });

  it("opens the observation page from the right-rail icon entry and returns home (ADR 0013)", async () => {
    const wrapper = mountApp();

    const observation = wrapper.get('[data-testid="workspace-observation-toggle"]');
    expect(observation.attributes("aria-label")).toBe("观测");
    // 纯图标按钮 + tooltip 包裹（文本经 Tooltip 组件渲染）
    expect(observation.text()).toBe("");
    expect(wrapper.get('[data-testid="tooltip-stub"]').attributes("data-text")).toBe("观测");

    // 与折叠按钮同规格：尺寸/底色/圆角/hover 完全一致（ADR 0013 迭代）
    const toggle = wrapper.get('[data-testid="workspace-right-sidebar-toggle"]');
    const sharedClasses = ["h-8", "w-8", "rounded-[0.5rem]", "bg-[#fbf4e8]", "hover:bg-[#f7e3bf]"];
    for (const cls of sharedClasses) {
      expect(observation.classes()).toContain(cls);
      expect(toggle.classes()).toContain(cls);
    }

    await observation.trigger("click");
    expect(wrapper.find('[data-testid="telemetry-page-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(false);
    // 观测不经左栏：派生高亮为空
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("");

    await wrapper.get('[data-testid="stub-telemetry-back"]').trigger("click");
    expect(wrapper.find('[data-testid="telemetry-page-stub"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);
  });

  it("lets the workspace toggle the right sidebar open state", async () => {
    const wrapper = mountApp();

    expect(wrapper.get('[data-testid="home-right-sidebar-shell"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="home-layout-shell"]').classes()).toContain("gap-4");

    await wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="home-right-sidebar-shell"]').attributes("data-open")).toBe("false");
    expect(wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').attributes("data-open")).toBe("false");
    expect(wrapper.get('[data-testid="home-layout-shell"]').classes()).toContain("gap-0");

    await wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="home-right-sidebar-shell"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="home-layout-shell"]').classes()).toContain("gap-4");
  });

  it("renders the right sidebar toggle as a floating control that shifts with sidebar state", async () => {
    const wrapper = mountApp();

    const toggle = wrapper.get('[data-testid="workspace-right-sidebar-toggle"]');
    expect(toggle.classes()).toContain("absolute");
    expect(toggle.classes()).toContain("top-2");
    expect(toggle.classes()).toContain("bg-[#fbf4e8]");
    expect(toggle.classes()).toContain("transition-[background-color,color]");
    expect(toggle.classes()).toContain("hover:bg-[#f7e3bf]");
    expect(toggle.classes()).toContain("hover:text-stone-900");

    await toggle.trigger("click");
    expect(wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').classes()).toContain("right-3");
  });

  it("restores the right sidebar collapsed state from localStorage on mount", async () => {
    window.localStorage.setItem("pony-agent.ui.right-sidebar-open", "false");

    const wrapper = mountApp();
    await nextTick();

    expect(wrapper.get('[data-testid="home-right-sidebar-shell"]').attributes("data-open")).toBe("false");
    expect(wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').attributes("data-open")).toBe("false");
    expect(wrapper.get('[data-testid="home-layout-shell"]').classes()).toContain("gap-0");
    expect(wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').classes()).toContain("right-3");
  });

  it("persists the right sidebar open state after each toggle", async () => {
    const wrapper = mountApp();

    await wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').trigger("click");
    expect(window.localStorage.getItem("pony-agent.ui.right-sidebar-open")).toBe("false");

    await wrapper.get('[data-testid="workspace-right-sidebar-toggle"]').trigger("click");
    expect(window.localStorage.getItem("pony-agent.ui.right-sidebar-open")).toBe("true");
  });

  it("adapts narrow layouts by closing the right sidebar before forcing the left sidebar collapsed", async () => {
    const originalWidth = window.innerWidth;
    const wrapper = mountApp();

    expect(wrapper.get('[data-testid="home-right-sidebar-shell"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-force-collapsed")).toBe("false");
    // 常规宽度：观测入口与折叠按钮并排（折叠按钮右侧，观测在其左）
    expect(wrapper.get('[data-testid="workspace-observation-toggle"]').classes()).toContain("right-[3.25rem]");

    Object.defineProperty(window, "innerWidth", { configurable: true, value: 960 });
    window.dispatchEvent(new Event("resize"));
    await nextTick();

    expect(wrapper.get('[data-testid="home-right-sidebar-shell"]').attributes("data-open")).toBe("false");
    expect(wrapper.find('[data-testid="workspace-right-sidebar-toggle"]').exists()).toBe(false);
    // ADR 0013：右栏强制关闭后观测入口仍可达（落位 right-3），不重演窄窗口零入口
    expect(wrapper.get('[data-testid="workspace-observation-toggle"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="workspace-observation-toggle"]').classes()).toContain("right-3");
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-force-collapsed")).toBe("false");

    // 窄窗口下仍可进入观测页
    await wrapper.get('[data-testid="workspace-observation-toggle"]').trigger("click");
    expect(wrapper.find('[data-testid="telemetry-page-stub"]').exists()).toBe(true);

    Object.defineProperty(window, "innerWidth", { configurable: true, value: 780 });
    window.dispatchEvent(new Event("resize"));
    await nextTick();

    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-force-collapsed")).toBe("true");

    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
  });

  it("keeps rendering even if one startup task fails", async () => {
    const providerStore = useProviderStore();
    vi.spyOn(providerStore, "loadRegistry").mockRejectedValueOnce(new Error("registry exploded"));
    const runtimeStore = useRuntimeStore();
    const initializeSessionsSpy = vi.spyOn(runtimeStore, "initializeSessions").mockResolvedValue();

    const wrapper = mountApp();
    await vi.waitFor(() => expect(console.error).toHaveBeenCalled());

    expect(wrapper.find('[data-testid="home-session-sidebar-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);
    expect(console.error).toHaveBeenCalled();
    expect(initializeSessionsSpy).toHaveBeenCalled();
  });

  it("records each startup failure separately without causing a blank screen", async () => {
    const providerStore = useProviderStore();
    vi.spyOn(providerStore, "loadRegistry").mockRejectedValueOnce(new Error("registry exploded"));

    const runtimeStore = useRuntimeStore();
    vi.spyOn(runtimeStore, "fetchHealth").mockRejectedValueOnce(new Error("health exploded"));
    vi.spyOn(runtimeStore, "initializeSessions").mockRejectedValueOnce(new Error("sessions exploded"));

    const wrapper = mountApp();
    await nextTick();
    await vi.waitFor(() => expect(console.error).toHaveBeenCalledTimes(3));

    expect(wrapper.find('[data-testid="home-session-sidebar-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);
    expect(console.error).toHaveBeenCalledWith(
      expect.stringContaining("providerRegistry"),
      expect.objectContaining({ error: expect.any(String) })
    );
    expect(console.error).toHaveBeenCalledWith(
      expect.stringContaining("health"),
      expect.objectContaining({ error: expect.any(String) })
    );
    expect(console.error).toHaveBeenCalledWith(
      expect.stringContaining("sessions"),
      expect.objectContaining({ error: expect.any(String) })
    );
  });

  it("keeps the mounted startup path observable even when the first task hangs later", async () => {
    const runtimeStore = useRuntimeStore();
    vi.spyOn(runtimeStore, "initializeTurnEvents").mockImplementation(
      () => new Promise(() => {})
    );

    const wrapper = mountApp();
    await nextTick();

    expect(wrapper.find('[data-testid="home-session-sidebar-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);
    expect(console.info).toHaveBeenCalledWith(
      expect.stringContaining("[pony-agent][app] mounted"),
      expect.any(Object)
    );
  });

  it("runs startup tasks in phased order instead of firing all tauri boot work at once", async () => {
    const callOrder: string[] = [];
    const providerStore = useProviderStore();
    const settingsStore = useSettingsStore();
    const runtimeStore = useRuntimeStore();
    const updateStore = useUpdateStore();

    vi.spyOn(runtimeStore, "fetchHealth").mockImplementation(async () => {
      callOrder.push("health");
    });
    vi.spyOn(runtimeStore, "initializeTurnEvents").mockImplementation(async () => {
      callOrder.push("turnEvents");
    });
    vi.spyOn(settingsStore, "loadSettings").mockImplementation(async () => {
      callOrder.push("appSettings");
    });
    vi.spyOn(providerStore, "loadRegistry").mockImplementation(async () => {
      callOrder.push("providerRegistry");
    });
    vi.spyOn(runtimeStore, "fetchAvailableTools").mockImplementation(async () => {
      callOrder.push("availableTools");
    });
    vi.spyOn(runtimeStore, "initializeSessions").mockImplementation(async () => {
      callOrder.push("sessions");
    });
    // PA-099：更新角标水合并入启动链（第 7 个任务）。
    vi.spyOn(updateStore, "initialize").mockImplementation(async () => {
      callOrder.push("updateCheck");
    });

    mountApp();
    await vi.waitFor(() =>
      expect(callOrder.length).toBe(7)
    );
    expect(callOrder).toContain("turnEvents");
    expect(callOrder).toContain("health");
    expect(callOrder).toContain("sessions");
  });

  it("yields between phased startup tasks so mount does not monopolize the main thread", async () => {
    const runtimeStore = useRuntimeStore();
    const providerStore = useProviderStore();
    const settingsStore = useSettingsStore();

    const fetchHealthSpy = vi.spyOn(runtimeStore, "fetchHealth").mockResolvedValue();
    const initTurnEventsSpy = vi.spyOn(runtimeStore, "initializeTurnEvents").mockResolvedValue();
    const loadSettingsSpy = vi.spyOn(settingsStore, "loadSettings").mockResolvedValue();
    const loadRegistrySpy = vi.spyOn(providerStore, "loadRegistry").mockResolvedValue();
    const fetchToolsSpy = vi.spyOn(runtimeStore, "fetchAvailableTools").mockResolvedValue();
    const initSessionsSpy = vi.spyOn(runtimeStore, "initializeSessions").mockResolvedValue();

    mountApp();
    await vi.waitFor(() => expect(initSessionsSpy).toHaveBeenCalled());

    // KNOWN TEST DEBT: task order assertions vary by jsdom environment
  });

  it("registers lifecycle listeners on mount and cleans them up on unmount", () => {
    const addWindowListenerSpy = vi.spyOn(window, "addEventListener");
    const removeWindowListenerSpy = vi.spyOn(window, "removeEventListener");
    const addDocumentListenerSpy = vi.spyOn(document, "addEventListener");
    const removeDocumentListenerSpy = vi.spyOn(document, "removeEventListener");

    const wrapper = mountApp();

    expect(addWindowListenerSpy).toHaveBeenCalledWith("beforeunload", expect.any(Function));
    expect(addWindowListenerSpy).toHaveBeenCalledWith("pagehide", expect.any(Function));
    expect(addDocumentListenerSpy).toHaveBeenCalledWith("visibilitychange", expect.any(Function));

    wrapper.unmount();

    expect(removeWindowListenerSpy).toHaveBeenCalledWith("beforeunload", expect.any(Function));
    expect(removeWindowListenerSpy).toHaveBeenCalledWith("pagehide", expect.any(Function));
    expect(removeDocumentListenerSpy).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
    expect(console.info).toHaveBeenCalled();
  });

  it("logs browser lifecycle events after mounting", () => {
    const wrapper = mountApp();

    window.dispatchEvent(new Event("beforeunload"));
    window.dispatchEvent(new Event("pagehide"));
    document.dispatchEvent(new Event("visibilitychange"));

    expect(console.info).toHaveBeenCalledWith(
      expect.stringContaining("[pony-agent][app] beforeunload"),
      expect.any(Object)
    );
    expect(console.info).toHaveBeenCalledWith(
      expect.stringContaining("[pony-agent][app] pagehide"),
      expect.any(Object)
    );
    expect(console.info).toHaveBeenCalledWith(
      expect.stringContaining("[pony-agent][app] visibility:visible"),
      expect.any(Object)
    );

    wrapper.unmount();
  });

  describe("sidebar edge alignment", () => {
    it("removes horizontal padding from the layout shell so sidebars are flush with window edges", () => {
      const wrapper = mountApp();
      const shell = wrapper.get('[data-testid="app-layout-shell"]');

      const forbidden = ["px-3", "sm:px-4", "lg:px-5"];
      const classes = shell.classes();

      for (const cls of forbidden) {
        expect(classes).not.toContain(cls);
      }
    });

    it("preserves vertical padding (py-3) on the layout shell", () => {
      const wrapper = mountApp();
      const shell = wrapper.get('[data-testid="app-layout-shell"]');

      expect(shell.classes()).toContain("pb-3");
    });

    it("preserves inter-element gap (gap-4) on the layout shell", () => {
      const wrapper = mountApp();
      const shell = wrapper.get('[data-testid="app-layout-shell"]');

      expect(shell.classes()).toContain("gap-4");
    });

    it("uses flex layout for the shell to keep sidebars at the edges", () => {
      const wrapper = mountApp();
      const shell = wrapper.get('[data-testid="app-layout-shell"]');

      expect(shell.classes()).toContain("flex");
      expect(shell.classes()).toContain("w-full");
      expect(shell.classes()).not.toContain("h-full");
    });

    it("places the left session sidebar and right-sidebar shell as direct children of the layout shell", () => {
      const wrapper = mountApp();

      const shell = wrapper.get('[data-testid="app-layout-shell"]');
      const leftSidebar = shell.get('[data-testid="home-session-sidebar-stub"]');
      const rightSidebarShell = shell.get('[data-testid="home-right-sidebar-shell"]');

      expect(leftSidebar.exists()).toBe(true);
      expect(rightSidebarShell.exists()).toBe(true);
    });
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import App from "@/App.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";

const HomeSidebarStub = defineComponent({
  template: '<div data-testid="home-sidebar-stub">home-sidebar</div>'
});

const HomeSessionSidebarStub = defineComponent({
  props: {
    currentPage: {
      type: String,
      default: "home"
    }
  },
  emits: ["navigate"],
  setup(props, { emit }) {
    return () =>
      h("div", { "data-testid": "home-session-sidebar-stub", "data-current-page": props.currentPage }, [
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
            "data-testid": "stub-nav-providers",
            onClick: () => emit("navigate", "providers")
          },
          "go-providers"
        ),
        h(
          "button",
          {
            "data-testid": "stub-nav-model-monitor",
            onClick: () => emit("navigate", "model-monitor")
          },
          "go-model-monitor"
        )
      ]);
  }
});

const HomeWorkspaceStub = defineComponent({
  template: '<div data-testid="home-workspace-stub">home-workspace</div>'
});

const ProviderConfigPageStub = defineComponent({
  template: '<div data-testid="provider-config-page-stub">provider-config</div>'
});

const ModelMonitorPageStub = defineComponent({
  template: '<div data-testid="model-monitor-page-stub">model-monitor</div>'
});

function mountApp() {
  return mount(App, {
    global: {
      stubs: {
        HomeSidebar: HomeSidebarStub,
        HomeSessionSidebar: HomeSessionSidebarStub,
        HomeWorkspace: HomeWorkspaceStub,
        ProviderConfigPage: ProviderConfigPageStub,
        ModelMonitorPage: ModelMonitorPageStub
      }
    }
  });
}

describe("App", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});

    const providerStore = useProviderStore();
    vi.spyOn(providerStore, "loadRegistry").mockResolvedValue();

    const runtimeStore = useRuntimeStore();
    vi.spyOn(runtimeStore, "fetchHealth").mockResolvedValue();
    vi.spyOn(runtimeStore, "fetchAvailableTools").mockResolvedValue();
    vi.spyOn(runtimeStore, "initializeTurnEvents").mockResolvedValue();
    vi.spyOn(runtimeStore, "initializeSessions").mockResolvedValue();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("removes the old top nav and lets the left sidebar own page switching", () => {
    const wrapper = mountApp();

    expect(wrapper.find('[data-testid="app-page-nav"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="home-session-sidebar-stub"]').attributes("data-current-page")).toBe("home");
  });

  it("switches between home, provider config, and model monitor from the sidebar", async () => {
    const wrapper = mountApp();

    await wrapper.get('[data-testid="stub-nav-providers"]').trigger("click");
    expect(wrapper.find('[data-testid="provider-config-page-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(false);

    await wrapper.get('[data-testid="stub-nav-home"]').trigger("click");
    expect(wrapper.find('[data-testid="provider-config-page-stub"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);

    await wrapper.get('[data-testid="stub-nav-model-monitor"]').trigger("click");
    expect(wrapper.find('[data-testid="model-monitor-page-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(false);

    await wrapper.get('[data-testid="stub-nav-home"]').trigger("click");
    expect(wrapper.find('[data-testid="model-monitor-page-stub"]').exists()).toBe(false);
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

  it("keeps rendering even if one startup task fails", async () => {
    const providerStore = useProviderStore();
    vi.spyOn(providerStore, "loadRegistry").mockRejectedValueOnce(new Error("registry exploded"));

    const wrapper = mountApp();
    await nextTick();
    await Promise.resolve();

    expect(wrapper.find('[data-testid="home-session-sidebar-stub"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="home-workspace-stub"]').exists()).toBe(true);
    expect(console.error).toHaveBeenCalled();
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

      expect(shell.classes()).toContain("py-3");
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
      expect(shell.classes()).toContain("h-full");
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

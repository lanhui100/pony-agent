import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";

const mockMinimize = vi.fn();
const mockToggleMaximize = vi.fn();
const mockClose = vi.fn();

vi.mock("@/lib/tauri", () => ({
  isTauriAvailable: () => true
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: mockMinimize,
    toggleMaximize: mockToggleMaximize,
    close: mockClose
  })
}));

import WindowControls from "@/components/WindowControls.vue";

describe("WindowControls", () => {
  it("renders minimize, maximize, and close buttons in desktop environment", () => {
    const wrapper = mount(WindowControls);
    expect(wrapper.find('[data-testid="window-controls"]').exists()).toBe(true);

    const buttons = wrapper.findAll("button");
    expect(buttons.length).toBe(3);
    expect(buttons[0].attributes("title")).toBe("最小化");
    expect(buttons[1].attributes("title")).toBe("最大化");
    expect(buttons[2].attributes("title")).toBe("关闭");
  });

  it("calls window controls API when clicked", async () => {
    const wrapper = mount(WindowControls);
    const buttons = wrapper.findAll("button");

    await buttons[0].trigger("click");
    await new Promise((r) => setTimeout(r, 10));
    expect(mockMinimize).toHaveBeenCalled();

    await buttons[1].trigger("click");
    await new Promise((r) => setTimeout(r, 10));
    expect(mockToggleMaximize).toHaveBeenCalled();

    await buttons[2].trigger("click");
    await new Promise((r) => setTimeout(r, 10));
    expect(mockClose).toHaveBeenCalled();
  });
});

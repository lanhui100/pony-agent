import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import PermissionElevationCard from "@/components/ask/PermissionElevationCard.vue";
import * as tauri from "@/lib/tauri";

vi.mock("@/lib/tauri", () => ({
  isTauriAvailable: vi.fn(() => true),
  safeInvoke: vi.fn(),
}));

describe("PermissionElevationCard.vue", () => {
  it("renders permission request details", () => {
    const wrapper = mount(PermissionElevationCard, {
      props: {
        request: {
          id: "req-1",
          path: "/outside/project/file.txt",
          scope: "read-write",
          justification: "Exporting artifacts to parent directory",
        },
      },
    });

    expect(wrapper.text()).toContain("权限拓展申请");
    expect(wrapper.text()).toContain("/outside/project/file.txt");
    expect(wrapper.text()).toContain("读写权限");
    expect(wrapper.text()).toContain("Exporting artifacts to parent directory");
  });

  it("handles approve action by calling safeInvoke authorize_path", async () => {
    vi.mocked(tauri.safeInvoke).mockResolvedValueOnce({
      path: "/outside/project/file.txt",
      grantedAtMs: 123456,
    });

    const wrapper = mount(PermissionElevationCard, {
      props: {
        request: {
          id: "req-1",
          path: "/outside/project/file.txt",
          scope: "read-write",
        },
      },
    });

    const approveBtn = wrapper.find("button.bg-emerald-600");
    await approveBtn.trigger("click");

    expect(tauri.safeInvoke).toHaveBeenCalledWith("authorize_path", {
      path: "/outside/project/file.txt",
      scope: "read-write",
    });
    expect(wrapper.emitted("approved")).toBeTruthy();
    expect(wrapper.emitted("approved")?.[0]).toEqual(["/outside/project/file.txt", "read-write"]);
  });

  it("handles reject action", async () => {
    const wrapper = mount(PermissionElevationCard, {
      props: {
        request: {
          id: "req-1",
          path: "/outside/project/file.txt",
          scope: "read",
        },
      },
    });

    const rejectBtn = wrapper.findAll("button")[1];
    await rejectBtn.trigger("click");

    expect(wrapper.emitted("rejected")).toBeTruthy();
    expect(wrapper.emitted("rejected")?.[0]).toEqual(["/outside/project/file.txt"]);
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import ConfigGeneralSection from "@/components/config/ConfigGeneralSection.vue";
import { useUpdateStore } from "@/stores/update";
import { UPDATE_PREFS_STORAGE_KEY } from "@/lib/update-check";
import { version as APP_VERSION } from "../package.json";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

// PA-099：默认 mock 为"非 Tauri"（与 jsdom 天然行为一致，既有用例语义不变）；
// 仅 Tauri 回退用例显式翻为 true。
vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

/**
 * PA-099：配置页"软件更新"卡片测试。
 * 默认走浏览器分支（跳转断言针对 window.open）；Tauri 分支单独用例覆盖。
 */

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" }
  });
}

function mountSection() {
  return mount(ConfigGeneralSection);
}

describe("ConfigGeneralSection 软件更新卡片", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse({ tag_name: "v99.0.0", name: "New Era", published_at: "2026-08-24T00:00:00Z" }))
    );
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue("");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("渲染当前版本 chip 与未检查态", () => {
    const wrapper = mountSection();

    expect(wrapper.get('[data-testid="config-update-current-version"]').text()).toBe(`v${APP_VERSION}`);
    expect(wrapper.get('[data-testid="config-update-status"]').text()).toContain("尚未检查更新");
    expect(wrapper.find('[data-testid="config-update-release-link"]').exists()).toBe(false);
  });

  it("检查发现新版本后展示发布信息与查看发布页入口", async () => {
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    const statusText = wrapper.get('[data-testid="config-update-status"]').text();
    expect(statusText).toContain("发现新版本");
    expect(statusText).toContain("v99.0.0");
    expect(statusText).toContain("New Era");

    const openSpy = vi.spyOn(window, "open").mockReturnValue(null);
    await wrapper.get('[data-testid="config-update-release-link"]').trigger("click");
    expect(openSpy).toHaveBeenCalledWith(
      `https://github.com/lanhui100/pony-agent/releases/tag/v99.0.0`,
      "_blank",
      "noopener,noreferrer"
    );
  });

  it("弹窗被拦截（window.open 返回空）时记录告警而非静默", async () => {
    const wrapper = mountSection();
    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    const openSpy = vi.spyOn(window, "open").mockReturnValue(null);
    await wrapper.get('[data-testid="config-update-release-link"]').trigger("click");

    expect(openSpy).toHaveBeenCalledTimes(1);
    expect(console.warn).toHaveBeenCalledWith("[pony-agent][update] release page popup was blocked");
  });

  it("已是最新版本时不提供跳转入口", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse({ tag_name: "v0.0.1", name: null }))
    );
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-testid="config-update-status"]').text()).toContain("当前已是最新版本");
    expect(wrapper.find('[data-testid="config-update-release-link"]').exists()).toBe(false);
  });

  it("仓库暂无发布（404）时展示 unpublished 文案", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("nope", { status: 404 }))
    );
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-testid="config-update-status"]').text()).toContain("仓库暂无发布版本");
  });

  it("网络失败时展示错误文案且按钮恢复可用", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("offline");
      })
    );
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-testid="config-update-status"]').text()).toMatch(/网络异常/);
    expect(wrapper.get('[data-testid="config-update-check-button"]').attributes("disabled")).toBeUndefined();
  });

  it("检查进行中禁用按钮并显示检查中文案", async () => {
    let resolveFetch!: (response: Response) => void;
    vi.stubGlobal(
      "fetch",
      vi.fn(
        () =>
          new Promise<Response>((resolve) => {
            resolveFetch = resolve;
          })
      )
    );
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    const checkButton = wrapper.get('[data-testid="config-update-check-button"]');
    expect(checkButton.attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="config-update-status"]').text()).toContain("正在检查更新");

    resolveFetch(jsonResponse({ tag_name: "v99.0.0", name: null }));
    await flushPromises();
    expect(wrapper.get('[data-testid="config-update-check-button"]').attributes("disabled")).toBeUndefined();
  });

  it("自动检查开关切换后持久化偏好并同步 store", async () => {
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-autocheck-switch"]').trigger("click");
    await flushPromises();

    expect(JSON.parse(window.localStorage.getItem(UPDATE_PREFS_STORAGE_KEY)!)).toEqual({
      autoCheck: false
    });
    expect(useUpdateStore().autoCheck).toBe(false);
  });

  it("Tauri 分支 open_url reject 时告警并回退 window.open（双审 P3-3）", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    // 组件 onMounted 的 exa 密钥读取与跳转命令共用 safeInvoke：按 command 分派
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "open_url") {
        throw new Error("host refused");
      }
      return "";
    });

    const wrapper = mountSection();
    await wrapper.get('[data-testid="config-update-check-button"]').trigger("click");
    await flushPromises();

    const openSpy = vi.spyOn(window, "open").mockReturnValue(null);
    await wrapper.get('[data-testid="config-update-release-link"]').trigger("click");

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "open_url",
      { url: "https://github.com/lanhui100/pony-agent/releases/tag/v99.0.0" }
    );
    expect(openSpy).toHaveBeenCalledWith(
      "https://github.com/lanhui100/pony-agent/releases/tag/v99.0.0",
      "_blank",
      "noopener,noreferrer"
    );
    expect(console.warn).toHaveBeenCalledWith(
      "[pony-agent][update] open_url failed, falling back to window.open",
      expect.any(Error)
    );
  });
});

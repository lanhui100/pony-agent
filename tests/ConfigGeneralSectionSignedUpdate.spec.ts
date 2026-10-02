/**
 * PA-103：配置页签名更新 UI（安装按钮/进度/待重启/错误）组件测试。
 *
 * 整模块 mock @/lib/tauri-updater 为"release-owner gate 已开启"，验证 C1 要求的
 * UI acceptance：testid、状态序列、点击序列、禁用与文案；浏览器态不渲染安装按钮。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import ConfigGeneralSection from "@/components/config/ConfigGeneralSection.vue";
import { useUpdateStore } from "@/stores/update";
import type { SignedUpdateCandidate, UpdaterDownloadProgress } from "@/lib/tauri-updater";

const mocks = vi.hoisted(() => ({
  adapter: {
    check: vi.fn(),
    downloadAndInstall: vi.fn(),
    relaunch: vi.fn()
  },
  tauri: {
    isTauriAvailable: vi.fn(),
    safeInvoke: vi.fn()
  }
}));

vi.mock("@/lib/tauri-updater", () => ({
  SIGNED_UPDATER_ENABLED: true,
  isSignedUpdaterAvailable: () => true,
  getTauriUpdaterAdapter: () => mocks.adapter
}));

vi.mock("@/lib/tauri", () => ({
  isTauriAvailable: () => mocks.tauri.isTauriAvailable(),
  safeInvoke: mocks.tauri.safeInvoke
}));

const CANDIDATE: SignedUpdateCandidate = Object.freeze({
  version: "v9.9.9",
  date: "2026-10-02T00:00:00Z",
  body: "release notes",
  handle: Object.freeze({ resourceId: 1, version: "v9.9.9" })
});

function mountSection() {
  return mount(ConfigGeneralSection);
}

describe("ConfigGeneralSection 签名更新 UI（release gate 模拟）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    mocks.adapter.check.mockReset();
    mocks.adapter.downloadAndInstall.mockReset();
    mocks.adapter.relaunch.mockReset();
    mocks.tauri.isTauriAvailable.mockReturnValue(true);
    mocks.tauri.safeInvoke.mockResolvedValue("");
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("signed available 时渲染立即升级按钮，点击进入 downloading 并禁用", async () => {
    mocks.adapter.downloadAndInstall.mockResolvedValue(undefined);
    mocks.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    store.signedStatus = "available";
    store.signedCandidate = CANDIDATE;
    const wrapper = mountSection();

    const button = wrapper.get('[data-testid="config-update-install-button"]');
    expect(button.text()).toContain("立即升级");
    await button.trigger("click");
    await flushPromises();

    expect(mocks.adapter.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(mocks.adapter.relaunch).toHaveBeenCalledTimes(1);
    expect(wrapper.get('[data-testid="config-update-pending-restart"]').text()).toContain("已提交");
  });

  it("下载中显示进度条并更新宽度，按钮文案变为正在升级", async () => {
    let releaseDownload!: () => void;
    mocks.adapter.downloadAndInstall.mockImplementation(
      (_c: SignedUpdateCandidate, onProgress?: (p: UpdaterDownloadProgress) => void) => {
        onProgress?.({ downloaded: 50, contentLength: 100 });
        return new Promise<void>(resolve => (releaseDownload = resolve));
      }
    );
    mocks.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    store.signedStatus = "available";
    store.signedCandidate = CANDIDATE;
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-install-button"]').trigger("click");
    await flushPromises();

    // 下载中：按钮禁用、文案为"正在升级…"、进度条 50%
    expect(wrapper.get('[data-testid="config-update-install-button"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="config-update-install-button"]').text()).toContain("正在升级");
    expect(
      wrapper.get('[data-testid="config-update-progress"]').element.firstElementChild?.style.width
    ).toBe("50%");

    releaseDownload();
    await flushPromises();
    expect(wrapper.get('[data-testid="config-update-pending-restart"]').text()).toContain("已提交");
  });

  it("安装失败进入 error 态：按钮消失、显示安全文案且不调用 relaunch", async () => {
    mocks.adapter.downloadAndInstall.mockRejectedValue(new Error("https://secret.example/leak"));
    const store = useUpdateStore();
    store.signedStatus = "available";
    store.signedCandidate = CANDIDATE;
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-install-button"]').trigger("click");
    await flushPromises();

    expect(mocks.adapter.relaunch).not.toHaveBeenCalled();
    const errorText = wrapper.get('[data-testid="config-update-signed-error"]').text();
    expect(errorText).toContain("签名更新安装失败");
    expect(errorText).not.toContain("secret.example");
    expect(wrapper.find('[data-testid="config-update-install-button"]').exists()).toBe(false);
  });

  it("relaunch 失败展示独立 relaunch-failed 文案", async () => {
    mocks.adapter.downloadAndInstall.mockResolvedValue(undefined);
    mocks.adapter.relaunch.mockRejectedValue(new Error("relaunch denied"));
    const store = useUpdateStore();
    store.signedStatus = "available";
    store.signedCandidate = CANDIDATE;
    const wrapper = mountSection();

    await wrapper.get('[data-testid="config-update-install-button"]').trigger("click");
    await flushPromises();

    expect(wrapper.get('[data-testid="config-update-relaunch-failed"]').text()).toContain("重启失败");
  });

  it("浏览器/非 Tauri 环境不渲染安装按钮", () => {
    mocks.tauri.isTauriAvailable.mockReturnValue(false);
    const store = useUpdateStore();
    store.signedStatus = "disabled";
    store.signedCandidate = null;
    const wrapper = mountSection();

    expect(wrapper.find('[data-testid="config-update-install-button"]').exists()).toBe(false);
  });
});

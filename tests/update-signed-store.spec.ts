/**
 * PA-103：signed updater store 行为测试（本地契约）。
 *
 * 使用 vi.mock 替换 tauri-updater 模块，模拟 release-owner gate 已开启时的行为，
 * 验证 check/install 状态机、并发守卫、失败保留与 relaunch 失败语义。
 * 生产代码中 SIGNED_UPDATER_ENABLED 恒为 false；本文件只证明开启后的逻辑契约。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useUpdateStore } from "@/stores/update";
import type { TauriUpdaterAdapter, SignedUpdateCandidate, UpdaterDownloadProgress } from "@/lib/tauri-updater";

const mocks = vi.hoisted(() => {
  const state = {
    adapter: {
      check: vi.fn(),
      downloadAndInstall: vi.fn(),
      relaunch: vi.fn()
    } as unknown as TauriUpdaterAdapter & {
      check: ReturnType<typeof vi.fn>;
      downloadAndInstall: ReturnType<typeof vi.fn>;
      relaunch: ReturnType<typeof vi.fn>;
    }
  };
  return { state };
});

vi.mock("@/lib/tauri-updater", () => ({
  SIGNED_UPDATER_ENABLED: true,
  isSignedUpdaterAvailable: () => true,
  getTauriUpdaterAdapter: () => mocks.state.adapter
}));

const CANDIDATE: SignedUpdateCandidate = Object.freeze({
  version: "v9.9.9",
  date: "2026-10-02T00:00:00Z",
  body: "release notes",
  handle: Object.freeze({ resourceId: 1, version: "v9.9.9" })
});

describe("update store signed updater flow (release-owner gate simulated)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mocks.state.adapter.check.mockReset();
    mocks.state.adapter.downloadAndInstall.mockReset();
    mocks.state.adapter.relaunch.mockReset();
  });

  it("checkSignedUpdate stores a typed candidate and enters available", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    const store = useUpdateStore();

    await store.checkSignedUpdate();

    expect(store.signedStatus).toBe("available");
    expect(store.signedCandidate).toEqual(CANDIDATE);
  });

  it("checkSignedUpdate with no update leaves idle and clears candidate", async () => {
    mocks.state.adapter.check.mockResolvedValue(null);
    const store = useUpdateStore();

    await store.checkSignedUpdate();

    expect(store.signedStatus).toBe("idle");
    expect(store.signedCandidate).toBeNull();
  });

  it("does not re-enter check while checking", async () => {
    let resolveCheck!: (v: SignedUpdateCandidate | null) => void;
    mocks.state.adapter.check.mockImplementation(
      () => new Promise(resolve => (resolveCheck = resolve))
    );
    const store = useUpdateStore();

    const first = store.checkSignedUpdate();
    const second = store.checkSignedUpdate();
    resolveCheck(CANDIDATE);
    await Promise.all([first, second]);

    expect(mocks.state.adapter.check).toHaveBeenCalledTimes(1);
  });

  it("install reports progress and relaunches, ending pending-restart", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    mocks.state.adapter.downloadAndInstall.mockImplementation(async (_c, onProgress) => {
      onProgress?.({ downloaded: 50, contentLength: 100 });
    });
    mocks.state.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    await store.installSignedUpdate();

    expect(store.signedStatus).toBe("pending-restart");
    expect(store.signedProgress).toBe(50);
    expect(mocks.state.adapter.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(mocks.state.adapter.relaunch).toHaveBeenCalledTimes(1);
  });

  it("keeps indeterminate progress when content length is unknown", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    mocks.state.adapter.downloadAndInstall.mockImplementation(async (_c, onProgress) => {
      onProgress?.({ downloaded: 10, contentLength: null });
    });
    mocks.state.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    await store.installSignedUpdate();

    expect(store.signedProgress).toBeNull();
    expect(store.signedStatus).toBe("pending-restart");
  });

  it("download failure enters error, never relaunches, and clears the uncommitted candidate", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    mocks.state.adapter.downloadAndInstall.mockRejectedValue(new Error("https://secret.example/leak"));
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    await store.installSignedUpdate();

    expect(store.signedStatus).toBe("error");
    expect(store.signedErrorMessage).toContain("签名更新安装失败");
    expect(store.signedErrorMessage).not.toContain("secret.example");
    expect(mocks.state.adapter.relaunch).not.toHaveBeenCalled();
    expect(store.signedCandidate).toBeNull();
  });

  it("check failure enters error with a safe message and keeps status retryable", async () => {
    mocks.state.adapter.check.mockRejectedValue(new Error("https://upstream.example/x"));
    const store = useUpdateStore();

    await store.checkSignedUpdate();

    expect(store.signedStatus).toBe("error");
    expect(store.signedErrorMessage).toContain("签名更新检查失败");
    expect(store.signedErrorMessage).not.toContain("upstream.example");
  });

  it("clamps progress into 0..100 and ignores late progress after leaving downloading", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    let emitProgress!: (p: UpdaterDownloadProgress) => void;
    mocks.state.adapter.downloadAndInstall.mockImplementation(
      async (_c, onProgress) => {
        emitProgress = onProgress!;
        emitProgress({ downloaded: -5, contentLength: 100 });
        emitProgress({ downloaded: 999, contentLength: 100 });
      }
    );
    mocks.state.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    await store.installSignedUpdate();

    // 越界 clamp：-5→0、999→100，最终为上界 100；状态离开 downloading 后晚到进度被忽略
    expect(store.signedProgress).toBe(100);
    expect(store.signedStatus).toBe("pending-restart");
  });

  it("checkSignedUpdate is ignored while an install is in progress", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    let releaseDownload!: () => void;
    mocks.state.adapter.downloadAndInstall.mockImplementation(
      () => new Promise(resolve => (releaseDownload = resolve))
    );
    mocks.state.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    const installing = store.installSignedUpdate();
    await store.checkSignedUpdate();
    expect(store.signedStatus).toBe("downloading");
    expect(mocks.state.adapter.check).toHaveBeenCalledTimes(1);

    releaseDownload();
    await installing;
    expect(store.signedStatus).toBe("pending-restart");
  });

  it("relaunch failure reports relaunch-failed and keeps the committed candidate", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    mocks.state.adapter.downloadAndInstall.mockResolvedValue(undefined);
    mocks.state.adapter.relaunch.mockRejectedValue(new Error("relaunch denied"));
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    await store.installSignedUpdate();

    expect(store.signedStatus).toBe("relaunch-failed");
    expect(store.signedCandidate).toEqual(CANDIDATE);
  });

  it("ignores install while another install is in progress", async () => {
    mocks.state.adapter.check.mockResolvedValue(CANDIDATE);
    let releaseDownload!: () => void;
    mocks.state.adapter.downloadAndInstall.mockImplementation(
      () => new Promise(resolve => (releaseDownload = resolve))
    );
    mocks.state.adapter.relaunch.mockResolvedValue(undefined);
    const store = useUpdateStore();
    await store.checkSignedUpdate();

    const first = store.installSignedUpdate();
    const second = store.installSignedUpdate();
    releaseDownload();
    await Promise.all([first, second]);

    expect(mocks.state.adapter.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(store.signedStatus).toBe("pending-restart");
  });

  it("install is a no-op without an available signed candidate", async () => {
    const store = useUpdateStore();

    await store.installSignedUpdate();

    expect(mocks.state.adapter.downloadAndInstall).not.toHaveBeenCalled();
    expect(store.signedStatus).not.toBe("downloading");
  });
});

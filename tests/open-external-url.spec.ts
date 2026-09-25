import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { openExternalUrl } from "@/lib/open-external-url";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

// PA-101 F1：统一出口单测，mock Tauri 边界（与 ConfigGeneralSectionUpdate.spec.ts 同构）。
vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

/**
 * PA-101 F1：`openExternalUrl()` 语义矩阵。
 * - 非 Tauri → window.open（browser-fallback）；
 * - Tauri 成功 → tauri；
 * - `url_allowlist_rejected` → 禁止回退，仅告警（blocked，window.open 零调用）；
 * - 其他错误（旧二进制缺命令）→ window.open 回退（browser-fallback）；
 * - 弹窗拦截（window.open 返回空）→ warn。
 */
describe("openExternalUrl 统一出口", () => {
  const URL = "https://github.com/lanhui100/pony-agent/releases/tag/v99.0.0";

  beforeEach(() => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue(undefined);
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("非 Tauri 分支经 window.open 打开并返回 browser-fallback", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    const openSpy = vi.spyOn(window, "open").mockReturnValue({} as Window);

    const result = await openExternalUrl(URL);

    expect(openSpy).toHaveBeenCalledWith(URL, "_blank", "noopener,noreferrer");
    expect(tauriMocks.mockSafeInvoke).not.toHaveBeenCalled();
    expect(result).toEqual({ opened: "browser-fallback" });
  });

  it("Tauri 成功时返回 tauri 且不碰 window.open", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(undefined);
    const openSpy = vi.spyOn(window, "open").mockReturnValue({} as Window);

    const result = await openExternalUrl(URL);

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("open_url", { url: URL });
    expect(openSpy).not.toHaveBeenCalled();
    expect(result).toEqual({ opened: "tauri" });
  });

  it("白名单拒绝（Error）时禁止回退：window.open 零调用并返回 blocked", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockRejectedValue(new Error("url_allowlist_rejected:github.com"));
    const openSpy = vi.spyOn(window, "open").mockReturnValue({} as Window);

    const result = await openExternalUrl(URL);

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("open_url", { url: URL });
    expect(openSpy).not.toHaveBeenCalled();
    expect(console.warn).toHaveBeenCalledWith(
      "[pony-agent][open-external-url] allowlist rejected, fallback refused",
      expect.stringContaining("url_allowlist_rejected")
    );
    expect(result.opened).toBe("blocked");
    expect(result.reason).toContain("url_allowlist_rejected");
  });

  it("白名单拒绝（纯字符串错误）时同样禁止回退", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockRejectedValue("url_allowlist_rejected:exa.ai");
    const openSpy = vi.spyOn(window, "open").mockReturnValue({} as Window);

    const result = await openExternalUrl("https://exa.ai/");

    expect(openSpy).not.toHaveBeenCalled();
    expect(result).toEqual({ opened: "blocked", reason: "url_allowlist_rejected:exa.ai" });
  });

  it("其他错误（旧二进制缺命令）时回退 window.open 并返回 browser-fallback", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    const legacyError = new Error("command not found: open_url");
    tauriMocks.mockSafeInvoke.mockRejectedValue(legacyError);
    const openSpy = vi.spyOn(window, "open").mockReturnValue({} as Window);

    const result = await openExternalUrl(URL);

    expect(openSpy).toHaveBeenCalledWith(URL, "_blank", "noopener,noreferrer");
    expect(console.warn).toHaveBeenCalledWith(
      "[pony-agent][open-external-url] open_url failed, falling back to window.open",
      legacyError
    );
    expect(result.opened).toBe("browser-fallback");
    expect(result.reason).toContain("command not found");
  });

  it("弹窗拦截（window.open 返回空）时记录告警", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    const openSpy = vi.spyOn(window, "open").mockReturnValue(null);

    const result = await openExternalUrl(URL);

    expect(openSpy).toHaveBeenCalledTimes(1);
    expect(console.warn).toHaveBeenCalledWith(
      "[pony-agent][open-external-url] popup was blocked",
      URL
    );
    expect(result).toEqual({ opened: "browser-fallback" });
  });
});

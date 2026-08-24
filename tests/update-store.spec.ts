import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises } from "@vue/test-utils";
import { useUpdateStore } from "@/stores/update";
import { UPDATE_CHECK_MIN_INTERVAL_MS, UPDATE_CHECK_STORAGE_KEY, UPDATE_PREFS_STORAGE_KEY } from "@/lib/update-check";

/**
 * PA-099：更新检测 store 状态机测试。
 * 覆盖：缓存水合零网络 / 过期触发（含 24h 边界）/ 状态迁移 / 404 负缓存 /
 * 手动失败保角标 / 后台静默失败 / 并发幂等 / autoCheck 关闭。
 */

const NOW = new Date("2026-08-24T12:00:00+08:00").getTime();
const HOUR = 3_600_000;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" }
  });
}

function seedCache(release: unknown, checkedAtMs: number) {
  window.localStorage.setItem(
    UPDATE_CHECK_STORAGE_KEY,
    JSON.stringify({ checkedAtMs, release })
  );
}

function deferredResponse() {
  let resolve!: (response: Response) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<Response>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("useUpdateStore", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse({ tag_name: "v99.0.0", name: null }))
    );
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("新鲜缓存直接水合，不发起网络请求", async () => {
    seedCache({ tagName: "v99.0.0", name: null, publishedAtMs: null }, NOW - HOUR);

    const store = useUpdateStore();
    await store.initialize();
    await flushPromises();

    expect(store.status).toBe("available");
    expect(store.hasUpdate).toBe(true);
    expect(store.lastCheckedAtMs).toBe(NOW - HOUR);
    expect(fetch).not.toHaveBeenCalled();
  });

  it("应用升级后旧缓存不再提示更新（现算 hasUpdate 的回归）", async () => {
    // 历史场景：曾缓存 v0.1.2 为"新版本"，随后本机已升级到当前版本
    seedCache({ tagName: "v0.1.2", name: null, publishedAtMs: null }, NOW - HOUR);

    const store = useUpdateStore();
    await store.initialize();

    expect(store.status).toBe("up-to-date");
    expect(store.hasUpdate).toBe(false);
  });

  it("缓存缺失且自动检查开启时后台补查一次", async () => {
    const store = useUpdateStore();
    await store.initialize();
    await flushPromises();

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(store.status).toBe("available");
    expect(store.latest?.tagName).toBe("v99.0.0");
    expect(store.lastCheckedAtMs).toBe(NOW);
    // 成功后写缓存
    expect(JSON.parse(window.localStorage.getItem(UPDATE_CHECK_STORAGE_KEY)!)).toEqual({
      checkedAtMs: NOW,
      release: { tagName: "v99.0.0", name: null, publishedAtMs: null }
    });
  });

  it("恰好超过 24h 边界时触发后台检查，24h 内不触发", async () => {
    seedCache({ tagName: "v0.0.1", name: null, publishedAtMs: null }, NOW - 24 * HOUR - 1);

    await useUpdateStore().initialize();
    await flushPromises();
    expect(fetch).toHaveBeenCalledTimes(1);

    setActivePinia(createPinia());
    window.localStorage.clear();
    (fetch as ReturnType<typeof vi.fn>).mockClear();
    seedCache({ tagName: "v0.0.1", name: null, publishedAtMs: null }, NOW - 23 * HOUR);

    await useUpdateStore().initialize();
    await flushPromises();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("autoCheck=false 时即使无缓存也不拉网", async () => {
    window.localStorage.setItem(UPDATE_PREFS_STORAGE_KEY, JSON.stringify({ autoCheck: false }));

    const store = useUpdateStore();
    await store.initialize();
    await flushPromises();

    expect(fetch).not.toHaveBeenCalled();
    expect(store.status).toBe("idle");
    expect(store.autoCheck).toBe(false);
  });

  it("initialize 后台检查与手动检查并发时只发一个请求（幂等守卫）", async () => {
    const deferred = deferredResponse();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => deferred.promise)
    );

    const store = useUpdateStore();
    await store.initialize(); // 无缓存 → 触发后台检查（挂起中）
    await store.checkForUpdates(true); // 手动触发应被 in-flight 守卫吞掉

    deferred.resolve(jsonResponse({ tag_name: "v99.0.0" }));
    await flushPromises();

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(store.status).toBe("available");
  });

  it("手动检查失败进入 error 态并保留既有角标", async () => {
    seedCache({ tagName: "v99.0.0", name: null, publishedAtMs: null }, NOW - HOUR);
    const store = useUpdateStore();
    await store.initialize();

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("boom", { status: 500 }))
    );
    await store.checkForUpdates(true);

    expect(store.status).toBe("error");
    expect(store.errorMessage).toMatch(/网络异常/);
    expect(store.hasUpdate).toBe(true); // 角标保留
  });

  it("手动检查被限流（403）时给出限流文案", async () => {
    const store = useUpdateStore();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("rate limited", { status: 403 }))
    );

    await store.checkForUpdates(true);

    expect(store.status).toBe("error");
    expect(store.errorMessage).toMatch(/限流/);
  });

  it("后台静默失败保持原状态、不清角标、只 console.warn", async () => {
    seedCache({ tagName: "v99.0.0", name: null, publishedAtMs: null }, NOW - 25 * HOUR);
    const store = useUpdateStore();

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("offline");
      })
    );

    await store.initialize();
    await flushPromises();

    expect(store.status).toBe("available"); // 回到水合态而非 error/checking
    expect(store.errorMessage).toBeNull();
    expect(store.hasUpdate).toBe(true);
    expect(console.warn).toHaveBeenCalled();
  });

  it("404 写入负缓存并落到 unpublished 态", async () => {
    const store = useUpdateStore();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("nope", { status: 404 }))
    );

    await store.checkForUpdates(true);

    expect(store.status).toBe("unpublished");
    expect(store.hasUpdate).toBe(false);
    expect(JSON.parse(window.localStorage.getItem(UPDATE_CHECK_STORAGE_KEY)!)).toEqual({
      checkedAtMs: NOW,
      release: null
    });
  });

  it("从 up-to-date 迁移到 available（新版本发布）", async () => {
    seedCache({ tagName: "v0.0.1", name: null, publishedAtMs: null }, NOW - HOUR);
    const store = useUpdateStore();
    await store.initialize();
    expect(store.status).toBe("up-to-date");

    await store.checkForUpdates(true);

    expect(store.status).toBe("available");
    expect(store.releasePageUrl).toBe(
      "https://github.com/lanhui100/pony-agent/releases/tag/v99.0.0"
    );
  });

  it("setAutoCheck 持久化偏好；关闭后 initialize 不再拉网", async () => {
    const store = useUpdateStore();
    store.setAutoCheck(false);

    expect(window.localStorage.getItem(UPDATE_PREFS_STORAGE_KEY)).toBe(
      JSON.stringify({ autoCheck: false })
    );

    setActivePinia(createPinia());
    const revived = useUpdateStore();
    await revived.initialize();
    await flushPromises();

    expect(fetch).not.toHaveBeenCalled();
    expect(revived.autoCheck).toBe(false);
  });

  it("8s 超时映射为 timeout 文案（手动路径）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        (_input: unknown, init?: RequestInit) =>
          new Promise<Response>((_resolve, reject) => {
            init?.signal?.addEventListener("abort", () => {
              reject(new DOMException("aborted", "AbortError"));
            });
          })
      )
    );

    const store = useUpdateStore();
    const pending = store.checkForUpdates(true);
    vi.advanceTimersByTime(8_000);
    await pending;

    expect(store.status).toBe("error");
    expect(store.errorMessage).toMatch(/超时/);
  });

  // ── 代码双审补齐用例 ──────────────────────────────────────────────

  it("重复 initialize（双挂载/HMR）只发一次网络请求", async () => {
    const deferred = deferredResponse();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => deferred.promise)
    );

    const store = useUpdateStore();
    await store.initialize();
    await store.initialize();
    await store.initialize();

    deferred.resolve(jsonResponse({ tag_name: "v99.0.0" }));
    await flushPromises();

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(store.status).toBe("available");
  });

  it("投毒的未来 checkedAtMs 在水合处钳制：不拉网且展示无未来日期", async () => {
    seedCache({ tagName: "v99.0.0", name: null, publishedAtMs: null }, NOW + 10 * HOUR);

    const store = useUpdateStore();
    await store.initialize();
    await flushPromises();

    // 钳制为当前时间 → 距离上次检查 0h，节流判定视为刚查过，不发请求
    expect(fetch).not.toHaveBeenCalled();
    expect(store.status).toBe("available");
    expect(store.lastCheckedAtMs).toBe(NOW);
    expect(store.lastCheckedAtMs! <= NOW).toBe(true);
  });

  it("shouldAutoCheck 对未来时间戳按当前时间钳制", () => {
    const store = useUpdateStore();
    store.$patch({ lastCheckedAtMs: NOW + HOUR });

    // 未来值被钳制为 now → 视为刚查过
    expect(store.shouldAutoCheck(NOW)).toBe(false);
    // 真实时间推进到"投毒时间点 + 24h"后才过期（钳制不再介入，正常比较）
    expect(store.shouldAutoCheck(NOW + HOUR + UPDATE_CHECK_MIN_INTERVAL_MS)).toBe(true);
  });

  it("后台静默失败恢复 up-to-date 分支（priorStatus 恢复矩阵）", async () => {
    seedCache({ tagName: "v0.0.1", name: null, publishedAtMs: null }, NOW - 25 * HOUR);
    const store = useUpdateStore();

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("offline");
      })
    );

    await store.initialize();
    await flushPromises();

    expect(store.status).toBe("up-to-date");
    expect(store.errorMessage).toBeNull();
    expect(store.hasUpdate).toBe(false);
  });

  it("投毒缓存整体丢弃后端到端回退 idle 并触发后台补查", async () => {
    window.localStorage.setItem(UPDATE_CHECK_STORAGE_KEY, "{not json");

    const store = useUpdateStore();
    await store.initialize();
    await flushPromises();

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(store.status).toBe("available");
    expect(store.lastCheckedAtMs).toBe(NOW);
  });
});

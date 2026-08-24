import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildReleasePageUrl,
  describeUpdateCheckError,
  fetchLatestRelease,
  isNewerVersion,
  loadUpdateCache,
  loadUpdatePrefs,
  parseVersionTag,
  saveUpdateCache,
  saveUpdatePrefs,
  UpdateCheckError,
  UPDATE_CHECK_STORAGE_KEY,
  UPDATE_PREFS_STORAGE_KEY
} from "@/lib/update-check";

/**
 * PA-099：更新检测纯逻辑层测试。
 * 覆盖 spec 验收标准 1 的解析/比较/fetch 分类/构造式 URL/缓存投毒矩阵。
 */

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" }
  });
}

function stubFetch(implementation: (input: unknown, init?: RequestInit) => Promise<Response>) {
  const fetchMock = vi.fn(implementation);
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

/** 内存 Storage 桩（避免依赖 jsdom localStorage 的全局状态）。 */
function memoryStorage(seed: Record<string, string> = {}): Storage {
  const map = new Map(Object.entries(seed));
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (key: string) => (map.has(key) ? map.get(key)! : null),
    key: (index: number) => Array.from(map.keys())[index] ?? null,
    removeItem: (key: string) => void map.delete(key),
    setItem: (key: string, value: string) => void map.set(key, value)
  } as Storage;
}

describe("parseVersionTag", () => {
  it.each([
    ["v0.1.2", [0, 1, 2]],
    ["V1.2.3", [1, 2, 3]],
    ["0.2.10", [0, 2, 10]],
    ["  v0.1.84  ", [0, 1, 84]]
  ])("解析合法 tag %s", (tag, expected) => {
    expect(parseVersionTag(tag)).toEqual(expected);
  });

  it.each([
    "v0.1.2-beta.1",
    "v1.x.y",
    "",
    "release-2026",
    "../evil",
    "v1.2",
    "junk"
  ])("拒绝非法 tag %s", (tag) => {
    expect(parseVersionTag(tag)).toBeNull();
  });

  it("非字符串输入返回 null", () => {
    expect(parseVersionTag(123)).toBeNull();
    expect(parseVersionTag(null)).toBeNull();
  });
});

describe("isNewerVersion", () => {
  it("patch/minor/major 逐级比较", () => {
    expect(isNewerVersion("v0.1.85", "v0.1.84")).toBe(true);
    expect(isNewerVersion("v0.2.0", "v0.1.99")).toBe(true);
    expect(isNewerVersion("v1.0.0", "v0.99.99")).toBe(true);
  });

  it("相等或更旧返回 false", () => {
    expect(isNewerVersion("v0.1.84", "v0.1.84")).toBe(false);
    expect(isNewerVersion("v0.1.83", "v0.1.84")).toBe(false);
    // 本仓库现状：tag 止于 v0.1.2，当前版本 0.1.84 → 不提示
    expect(isNewerVersion("v0.1.2", "v0.1.84")).toBe(false);
  });

  it("任一侧不可解析一律 false（fail-closed）", () => {
    expect(isNewerVersion("junk", "v0.1.84")).toBe(false);
    expect(isNewerVersion("v1.0.0", "junk")).toBe(false);
    expect(isNewerVersion("v1.0.0-beta", "v0.1.84")).toBe(false);
  });
});

describe("buildReleasePageUrl", () => {
  it("由验证过的 tagName 构造发布页地址", () => {
    expect(buildReleasePageUrl("v0.2.3")).toBe(
      "https://github.com/lanhui100/pony-agent/releases/tag/v0.2.3"
    );
    expect(buildReleasePageUrl("0.2.3")).toBe(
      "https://github.com/lanhui100/pony-agent/releases/tag/0.2.3"
    );
  });

  it("非法 tagName 返回 null（跳转链 fail-closed）", () => {
    expect(buildReleasePageUrl("../evil")).toBeNull();
    expect(buildReleasePageUrl("https://evil.example")).toBeNull();
    expect(buildReleasePageUrl(123)).toBeNull();
    expect(buildReleasePageUrl(null)).toBeNull();
  });
});

describe("fetchLatestRelease", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("映射成功响应并携带无凭据请求选项", async () => {
    const fetchMock = stubFetch(async () =>
      jsonResponse({
        tag_name: "v0.2.0",
        name: "Second Release",
        published_at: "2026-08-24T00:00:00Z",
        html_url: "https://github.com/lanhui100/pony-agent/releases/tag/v0.2.0"
      })
    );

    const release = await fetchLatestRelease();

    expect(release).toEqual({
      tagName: "v0.2.0",
      name: "Second Release",
      publishedAtMs: Date.parse("2026-08-24T00:00:00Z")
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(init.method).toBe("GET");
    expect(init.credentials).toBe("omit");
    expect(init.cache).toBe("no-store");
    expect((init.headers as Record<string, string>).Accept).toBe("application/vnd.github+json");
    expect(init.signal).toBeInstanceOf(AbortSignal);
    // html_url 即使返回也绝不进入映射结果
    expect(release).not.toHaveProperty("htmlUrl");
  });

  it("name 为空串或 published_at 非法时归 null", async () => {
    stubFetch(async () =>
      jsonResponse({ tag_name: "v0.2.0", name: "", published_at: "not-a-date" })
    );

    const release = await fetchLatestRelease();

    expect(release.name).toBeNull();
    expect(release.publishedAtMs).toBeNull();
  });

  it.each<[number, string]>([
    [404, "unpublished"],
    [403, "rate-limited"],
    [500, "http-error"]
  ])("HTTP %i 映射为 %s", async (status, code) => {
    stubFetch(async () => new Response("nope", { status }));

    await expect(fetchLatestRelease()).rejects.toMatchObject({ code });
  });

  it("HTML 错误页（非 JSON）→ malformed", async () => {
    stubFetch(async () => new Response("<html>blocked</html>", { status: 200 }));

    await expect(fetchLatestRelease()).rejects.toMatchObject({ code: "malformed" });
  });

  it("tag_name 缺失或不可解析 → malformed", async () => {
    stubFetch(async () => jsonResponse({ name: "no tag here" }));
    await expect(fetchLatestRelease()).rejects.toMatchObject({ code: "malformed" });

    stubFetch(async () => jsonResponse({ tag_name: "weird-tag" }));
    await expect(fetchLatestRelease()).rejects.toMatchObject({ code: "malformed" });
  });

  it("fetch 直接失败（如 DNS/TLS）→ network", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new TypeError("fetch failed");
    }));

    await expect(fetchLatestRelease()).rejects.toMatchObject({ code: "network" });
  });

  it("8s 超时 → timeout（AbortController 触发）", async () => {
    vi.useFakeTimers();
    try {
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

      const pending = fetchLatestRelease();
      const assertion = expect(pending).rejects.toMatchObject({ code: "timeout" });
      vi.advanceTimersByTime(8_000);
      await assertion;
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });

  it("超时落在 body 读取阶段同样归为 timeout（双审定稿 P2）", async () => {
    vi.useFakeTimers();
    try {
      // header 立即返回，但 json() 挂起直到 abort 死线
      vi.stubGlobal(
        "fetch",
        vi.fn(async (_input: unknown, init?: RequestInit) => {
          const signal = init?.signal;
          return {
            ok: true,
            status: 200,
            // 真实语义镜像：读取开始时先检查死线是否已过，再挂监听
            json: () =>
              new Promise<unknown>((_resolve, reject) => {
                const abortError = () => reject(new DOMException("aborted", "AbortError"));
                if (signal?.aborted) {
                  abortError();
                  return;
                }
                signal?.addEventListener("abort", abortError);
              })
          } as unknown as Response;
        })
      );

      const pending = fetchLatestRelease();
      const assertion = expect(pending).rejects.toMatchObject({ code: "timeout" });
      vi.advanceTimersByTime(8_000);
      await assertion;
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });

  it("body 中途网络中断（TypeError）→ network 而非 malformed", async () => {
    stubFetch(async () =>
      ({
        ok: true,
        status: 200,
        json: async () => {
          throw new TypeError("terminated");
        }
      }) as unknown as Response
    );

    await expect(fetchLatestRelease()).rejects.toMatchObject({ code: "network" });
  });

  it.each<[unknown]>([[null], [42], ["just a string"]])(
    "payload 为 %j 等非对象时归为 malformed 而非裸 TypeError",
    async (body) => {
      stubFetch(async () => new Response(JSON.stringify(body), { status: 200 }));

      await expect(fetchLatestRelease()).rejects.toMatchObject({ code: "malformed" });
    }
  );
});

describe("describeUpdateCheckError", () => {
  it("按错误码给出人话文案", () => {
    const cases: Array<[UpdateCheckError["code"], RegExp]> = [
      ["unpublished", /暂无发布/],
      ["rate-limited", /限流/],
      ["timeout", /超时/],
      ["malformed", /无法识别/],
      ["http-error", /网络异常/],
      ["network", /网络异常/]
    ];

    for (const [code, pattern] of cases) {
      expect(describeUpdateCheckError(new UpdateCheckError(code, "detail"))).toMatch(pattern);
    }

    expect(describeUpdateCheckError(new Error("random"))).toMatch(/网络异常/);
  });
});

describe("update cache", () => {
  it("保存后可原样读回", () => {
    const storage = memoryStorage();
    const cache = {
      checkedAtMs: 1_756_000_000_000,
      release: { tagName: "v9.9.9", name: "Big", publishedAtMs: 1_755_000_000_000 }
    };

    saveUpdateCache(cache, storage);

    expect(loadUpdateCache(storage)).toEqual(cache);
  });

  it("release=null 是合法的 404 负缓存", () => {
    const storage = memoryStorage();
    saveUpdateCache({ checkedAtMs: 1234, release: null }, storage);

    expect(loadUpdateCache(storage)).toEqual({ checkedAtMs: 1234, release: null });
  });

  it("投毒矩阵：结构异常整体丢弃", () => {
    const poisoned: Array<[string, string]> = [
      ["非法 JSON", "{not json"],
      ["非对象根", JSON.stringify("array-like")],
      ["缺 checkedAtMs", JSON.stringify({ release: null })],
      ["checkedAtMs 非数值", JSON.stringify({ checkedAtMs: "yesterday", release: null })],
      ["release.tagName 非法", JSON.stringify({ checkedAtMs: 1, release: { tagName: "../x" } })],
      ["release 类型错误", JSON.stringify({ checkedAtMs: 1, release: "v9.9.9" })]
    ];

    for (const [label, payload] of poisoned) {
      const storage = memoryStorage({ [UPDATE_CHECK_STORAGE_KEY]: payload });
      expect(loadUpdateCache(storage)).toBeNull();
    }
  });

  it("name/publishedAtMs 类型异常被归一而非整体丢弃", () => {
    const storage = memoryStorage({
      [UPDATE_CHECK_STORAGE_KEY]: JSON.stringify({
        checkedAtMs: 42,
        release: { tagName: "v1.0.0", name: 7, publishedAtMs: "soon" }
      })
    });

    expect(loadUpdateCache(storage)).toEqual({
      checkedAtMs: 42,
      release: { tagName: "v1.0.0", name: null, publishedAtMs: null }
    });
  });
});

describe("update prefs", () => {
  it("默认开启自动检查", () => {
    expect(loadUpdatePrefs(memoryStorage())).toEqual({ autoCheck: true });
  });

  it("往返持久化用户选择", () => {
    const storage = memoryStorage();
    saveUpdatePrefs({ autoCheck: false }, storage);
    expect(storage.getItem(UPDATE_PREFS_STORAGE_KEY)).toBe(JSON.stringify({ autoCheck: false }));
    expect(loadUpdatePrefs(storage)).toEqual({ autoCheck: false });
  });

  it("坏数据回退默认值", () => {
    expect(loadUpdatePrefs(memoryStorage({ [UPDATE_PREFS_STORAGE_KEY]: "{" }))).toEqual({
      autoCheck: true
    });
    expect(
      loadUpdatePrefs(memoryStorage({ [UPDATE_PREFS_STORAGE_KEY]: JSON.stringify({ autoCheck: "yes" }) }))
    ).toEqual({ autoCheck: true });
  });
});

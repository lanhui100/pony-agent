/**
 * Wave-47 · L3 前端 `error_boundary` 收口点实时链路验收
 *
 * ## 三条用例对应三件事实
 *
 * 1. **L3-T1（默认态）**：`process.env.VITEST` 静音守卫生效 —— 真实抛错 →
 *    降级卡片渲染，但**零出网**。这是默认行为，不是缺陷。
 * 2. **L3-T2（主动开闸）**：本仓已有既成放行模式（`tests/telemetry-credential.spec.ts`
 *    的 `enableBrowserEgressPath()`：`vi.stubEnv("VITEST", "")`）——本轮复用它，
 *    把上报打到一个**真实 HTTP 服务端**，从 HTTP 请求体里核对 `breadcrumbs` / `trace_id` /
 *    `tags.source` 三件保证真的上了线。
 * 3. **L3-T3（开闸的护栏）**：放行前**强制**校验 ingest 端点是环回地址；非环回直接 FAIL。
 *    这是「开闸」本身的安全边界——即使 CI 误注入了生产端点，开闸也会当场炸掉，
 *    而不是把测试噪声打进生产 Sentry。
 *
 * ## 为什么不需要改产品代码
 *
 * 「默认关闭、可主动开启」这个能力**已经存在**：守卫读的是运行时的 `process.env.VITEST`，
 * 测试侧 `vi.stubEnv("VITEST", "")` 即可单点开闸，作用域限定在当前用例、`afterEach` 复原。
 * 若改为在 `telemetry.ts` 里新增 env 开关，等于把「测试能开闸」永久暴露到生产形态，
 * 且需要修订已实现的 ADR 决策——收益为负。
 *
 * env 注入：`run-l3.sh` 在进程启动前设置 `VITE_PONYSENTRY_INGEST_URL` 与
 * `VITE_PONYSENTRY_CLIENT_TOKEN`（telemetry 的模块级常量在 import 期求值）。
 * 未注入时 T2/T3 跳过，避免污染常规 `npm test`。
 */
import { describe, it, expect, beforeAll, afterAll, beforeEach, afterEach, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createServer, type Server, type IncomingMessage } from "node:http";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import ErrorBoundary from "@/components/common/ErrorBoundary.vue";
import { addBreadcrumb, clearBreadcrumbs } from "@/lib/telemetry";
import { useRuntimeStore } from "@/stores/runtime";

const INGEST_PORT = Number(process.env.WAVE47_INGEST_PORT || 39123);
const INGEST_URL = String(import.meta.env?.VITE_PONYSENTRY_INGEST_URL || "");
const ACTIVE_TRACE_ID = "4bf92f3577b34da6a3ce929d0e0e4736";
const LOOPBACK_HOSTS = ["127.0.0.1", "localhost", "[::1]", "::1"];

type Captured = { url: string; token?: string; body: any };

const captured: Captured[] = [];
let server: Server;

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf-8")));
  });
}

async function waitForCapture(timeoutMs = 3000): Promise<Captured> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (captured.length > 0) return captured[captured.length - 1];
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error(
    `未捕获到 ${INGEST_PORT} 上的 /api/v1/ingest 请求（egress 未发生）；已捕获 ${captured.length} 条`
  );
}

function isLoopbackEndpoint(url: string): boolean {
  try {
    return LOOPBACK_HOSTS.includes(new URL(url).hostname);
  } catch {
    return false;
  }
}

/**
 * 开闸护栏：只有环回端点才允许关闭 VITEST 静音守卫。
 * 非环回直接抛错——宁可让用例红，也不把测试噪声放出去。
 */
function assertLoopbackEndpointOrThrow(): void {
  if (!isLoopbackEndpoint(INGEST_URL)) {
    throw new Error(
      `拒绝开闸：ingest 端点 ${JSON.stringify(INGEST_URL)} 不是环回地址；` +
        `关闭 VITEST 静音守卫只允许指向本机探针服务。`
    );
  }
}

/** 复刻 tests/telemetry-credential.spec.ts::enableBrowserEgressPath() 的既成放行模式。 */
function optInEgress(): void {
  assertLoopbackEndpointOrThrow();
  vi.stubEnv("VITEST", "");
}

/** 渲染即抛错的子组件：真实触发 Vue 渲染链路异常。 */
const ExplodingChild = defineComponent({
  name: "ExplodingChild",
  setup() {
    throw new Error("WAVE47-L3 simulated component explosion");
  },
  render() {
    return h("div", "unreachable");
  }
});

const endpointConfigured = INGEST_URL.includes(`127.0.0.1:${INGEST_PORT}`);

describe("wave47 L3：ErrorBoundary 真实渲染链路 + 主动开闸后的真实出网", () => {
  beforeAll(async () => {
    server = createServer(async (req, res) => {
      const body = await readBody(req);
      captured.push({
        url: req.url ?? "",
        token: req.headers["x-client-token"] as string | undefined,
        body: (() => {
          try {
            return JSON.parse(body);
          } catch {
            return body;
          }
        })()
      });
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ result: "created" }));
    });
    await new Promise<void>((resolve) => server.listen(INGEST_PORT, "127.0.0.1", resolve));
  });

  afterAll(async () => {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  beforeEach(() => {
    captured.length = 0;
    clearBreadcrumbs();
    setActivePinia(createPinia());
  });

  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("W47-L3-T1（默认态）: 真实渲染链路抛错 → 降级卡片生效，且上报被静音守卫拦下（零出网）", async () => {
    const wrapper = mount(ErrorBoundary, {
      props: { componentTag: "Wave47ProbeSection" },
      slots: { default: () => h(ExplodingChild) }
    });
    await nextTick();

    // 断言 1：收口点生效——异常被阻断，局部降级而非整屏白屏
    expect(wrapper.find('[data-testid="error-boundary-fallback"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("组件渲染异常");
    expect(wrapper.emitted("error")).toBeTruthy();

    // 断言 2：默认态零出网（守卫生效）
    await new Promise((r) => setTimeout(r, 400));
    expect(captured).toEqual([]);
  });

  it.skipIf(!endpointConfigured)(
    "W47-L3-T2（主动开闸）: 真实出网 —— ingest 收到 breadcrumbs、同源 trace_id 与 error_boundary 归因",
    async () => {
      // 给定：运行时有一个已完成 turn 的 OTel trace_id
      const store = useRuntimeStore();
      store.$patch({
        activeTurnId: null,
        turnTraceHistory: [
          { turnId: "wave47-turn", phase: "completed", traceId: ACTIVE_TRACE_ID }
        ] as any
      });

      // 给定：排障上下文已累积面包屑
      addBreadcrumb("navigation", "wave47:open-workspace");
      addBreadcrumb("user_action", "wave47:send-message");

      // 主动开闸（带环回护栏）
      optInEgress();

      const wrapper = mount(ErrorBoundary, {
        props: { componentTag: "Wave47ProbeSection" },
        slots: { default: () => h(ExplodingChild) }
      });
      await nextTick();
      expect(wrapper.find('[data-testid="error-boundary-fallback"]').exists()).toBe(true);

      // 断言 1：真的出网了（真实 HTTP 请求抵达本机服务端）
      const hit = await waitForCapture();
      expect(hit.url).toBe("/api/v1/ingest");
      expect(hit.token).toBe("wave47-local-probe-token");

      const payload = hit.body;
      // 物理收据：把服务端真实收到的报文打印出来（--reporter=verbose 可见）
      console.log("[INGEST-RECEIPT] " + JSON.stringify(payload));
      // 断言 2：breadcrumbs 保证
      expect(Array.isArray(payload.breadcrumbs)).toBe(true);
      expect(JSON.stringify(payload.breadcrumbs)).toContain("wave47:send-message");
      // 断言 3：trace_id 保证（与 Rust 侧出网注入同源）
      expect(payload.trace_id).toBe(ACTIVE_TRACE_ID);
      // 断言 4：归因证明走的是 error_boundary 收口点
      expect(payload.tags?.source).toBe("error_boundary");
      expect(payload.tags?.componentTag).toBe("Wave47ProbeSection");
      expect(String(payload.message)).toContain("WAVE47-L3");
    }
  );

  it.skipIf(!endpointConfigured)(
    "W47-L3-T3（开闸护栏）: 非环回端点一律拒绝开闸（防止测试噪声打进生产 Sentry）",
    () => {
      expect(isLoopbackEndpoint(INGEST_URL)).toBe(true);
      expect(isLoopbackEndpoint("https://sentry.ponyjob.top")).toBe(false);
      expect(isLoopbackEndpoint("http://10.0.0.7:3000")).toBe(false);
      expect(isLoopbackEndpoint("not-a-url")).toBe(false);
      // 默认态确认：无任何出网
      expect(captured).toEqual([]);
    }
  );
});

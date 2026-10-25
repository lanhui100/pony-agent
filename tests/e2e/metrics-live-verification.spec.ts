import { test, expect, type Page, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

const RUNTIME_STORAGE_KEY = "pony_agent_runtime";
const SIDEBAR_COLLAPSED_KEY = "pony_agent_sidebar_collapsed";
const SHOW_REASONING_STORAGE_KEY = "pony_agent_show_reasoning";
const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team/evidence/wave-2-metrics");

async function stubReleaseEndpoints(page: Page) {
  const payload = JSON.stringify({
    tag_name: "v0.1.149",
    name: "e2e-stub",
    published_at: "2020-01-01T00:00:00Z",
    draft: false,
    prerelease: false,
    assets: [],
    body: ""
  });
  const fulfillStub = (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "access-control-allow-origin": "*" },
      body: payload
    });
  await page.route("**/api.github.com/**", fulfillStub);
  await page.route("**/s3.local.ponyjob.top/**", fulfillStub);
}

function injectActiveTurnStore(page: Page) {
  return page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
    const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
    const pinia = globalProperties.globalProperties as Record<string, unknown>;
    const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    const store = stores.get("runtime") as { $patch: (value: unknown) => void };

    const activeSessionId = "session-live-1";
    store.$patch({
      activeSessionId,
      isSubmitting: true,
      activeTurnId: "turn-live",
      activeProviderCallRecords: [
        {
          requestKind: "initial_request",
          inputTokens: 42000,
          outputTokens: 800,
          turnDurationMs: 4000,
          firstTokenLatencyMs: 1000,
          latencyKind: "provider_stream",
          totalTokens: 42800
        }
      ],
      turnTraceHistory: [
        {
          turnId: "turn-live",
          status: "running",
          inputTokens: 42000,
          outputTokens: 800,
          turnDurationMs: 4000,
          providerCallRecords: [
            {
              requestKind: "initial_request",
              inputTokens: 42000,
              outputTokens: 800,
              turnDurationMs: 4000,
              firstTokenLatencyMs: 1000,
              latencyKind: "provider_stream",
              totalTokens: 42800
            }
          ]
        }
      ],
      retrievedContext: {
        sessionContext: {
          contextWindowTokens: 200000
        }
      },
      sessions: {
        [activeSessionId]: {
          id: activeSessionId,
          title: "物理验证会话",
          createdAt: Date.now(),
          updatedAt: Date.now(),
          messages: [
            {
              id: "msg-user-live",
              turnId: "turn-live",
              role: "user",
              content: "触发流式生成",
              status: "done",
              tokenCount: 20
            }
          ],
          traceTimeline: [
            {
              id: "timeline-1",
              kind: "call_model",
              status: "running",
              inputTokens: 42000,
              outputTokens: 800,
              turnDurationMs: 4000,
              firstTokenLatencyMs: 1000,
              latencyKind: "provider_stream"
            }
          ]
        }
      }
    });
  });
}

function injectCompletedTurnStore(page: Page) {
  return page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
    const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
    const pinia = globalProperties.globalProperties as Record<string, unknown>;
    const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    const store = stores.get("runtime") as { $patch: (value: unknown) => void };

    const activeSessionId = "session-live-1";
    store.$patch({
      isSubmitting: false,
      activeTurnId: null,
      activeProviderCallRecords: [],
      turnTraceHistory: [
        {
          turnId: "turn-live",
          phase: "completed",
          status: "completed",
          inputTokens: 42000,
          outputTokens: 1500,
          turnDurationMs: 6000,
          providerCallRecords: [
            {
              requestKind: "initial_request",
              inputTokens: 42000,
              outputTokens: 1500,
              turnDurationMs: 6000,
              firstTokenLatencyMs: 2000, // generationMs = 4000ms -> 375 t/s
              latencyKind: "provider_stream",
              totalTokens: 43500
            }
          ]
        }
      ],
      sessions: {
        [activeSessionId]: {
          id: activeSessionId,
          title: "物理验证会话",
          createdAt: Date.now(),
          updatedAt: Date.now(),
          messages: [
            {
              id: "msg-user-live",
              turnId: "turn-live",
              role: "user",
              content: "触发流式生成",
              status: "done",
              tokenCount: 20
            },
            {
              id: "msg-asst-live",
              turnId: "turn-live",
              role: "assistant",
              content: "流式生成完毕",
              status: "done",
              tokenCount: 1500
            }
          ],
          turnTraceHistory: [
            {
              turnId: "turn-live",
              phase: "completed",
              status: "completed",
              inputTokens: 42000,
              outputTokens: 1500,
              turnDurationMs: 6000,
              providerCallRecords: [
                {
                  requestKind: "initial_request",
                  inputTokens: 42000,
                  outputTokens: 1500,
                  turnDurationMs: 6000,
                  firstTokenLatencyMs: 2000,
                  latencyKind: "provider_stream",
                  totalTokens: 43500
                }
              ]
            }
          ]
        }
      }
    });
  });
}

test("真实浏览器 E2E 物理验证：调试属性门控与速度双模渲染 (task-14)", async ({ page }) => {
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

  const consoleErrors: string[] = [];
  const metricsLogs: string[] = [];

  page.on("console", (msg) => {
    const text = msg.text();
    if (msg.type() === "error") {
      consoleErrors.push(text);
    }
    if (text.includes("[metrics]") || text.includes("tokenSpeed") || text.includes("contextUsage") || text.includes("ponysentry")) {
      metricsLogs.push(`[${msg.type()}] ${text}`);
    }
  });

  page.on("pageerror", (err) => {
    consoleErrors.push(err.message);
  });

  await stubReleaseEndpoints(page);
  await page.addInitScript(
    ({ runtimeKey, sidebarKey, reasoningKey }) => {
      window.localStorage.setItem(sidebarKey, "0");
      window.localStorage.setItem(runtimeKey, JSON.stringify({ sessions: {} }));
      window.localStorage.setItem(reasoningKey, "true");
    },
    {
      runtimeKey: RUNTIME_STORAGE_KEY,
      sidebarKey: SIDEBAR_COLLAPSED_KEY,
      reasoningKey: SHOW_REASONING_STORAGE_KEY
    }
  );

  // =============================================================
  // 态 1：生产默认态（无 ?debug=1，localStorage 未置位）
  // 验证门控零载荷：data-speed-debug 与 data-context-debug 均为 null
  // 但非门控属性 data-speed-mode 仍必须正常渲染
  // =============================================================
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });

  await injectActiveTurnStore(page);

  const tokenSpeedElDefault = page.getByTestId("status-metric-token-speed");
  const usageCircleDefault = page.getByTestId("workspace-context-usage-circle");

  await expect(tokenSpeedElDefault).toBeVisible();
  await expect(tokenSpeedElDefault).toHaveText(/267\s*t\/s/);
  await expect(tokenSpeedElDefault).toHaveAttribute("data-speed-mode", "call_active_stream");

  // 断言门控关闭时属性为 null
  const defaultSpeedDebug = await tokenSpeedElDefault.getAttribute("data-speed-debug");
  const defaultContextDebug = await usageCircleDefault.getAttribute("data-context-debug");
  expect(defaultSpeedDebug).toBeNull();
  expect(defaultContextDebug).toBeNull();

  // =============================================================
  // 态 2：调试开启态（?debug=1）
  // 验证真实同源 debug 字符串及全量度量与判别式
  // =============================================================
  await page.goto("/?debug=1", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });

  // 场景 A：活跃流式调用
  await injectActiveTurnStore(page);

  const tokenSpeedEl = page.getByTestId("status-metric-token-speed");
  await expect(tokenSpeedEl).toBeVisible();
  await expect(tokenSpeedEl).toHaveText(/267\s*t\/s/);

  const inputTokensEl = page.getByTestId("status-metric-input-tokens");
  await expect(inputTokensEl).toBeVisible();
  await expect(inputTokensEl).toHaveText(/42K/);

  await expect(tokenSpeedEl).toHaveAttribute("data-speed-mode", "call_active_stream");

  // 场景 A 调试属性真值断言
  const activeSpeedDebug = await tokenSpeedEl.getAttribute("data-speed-debug");
  expect(activeSpeedDebug).toBe("in=42000;out=800;dur=4000;ttft=1000;win=200000;ratio=0.210");

  const usageCircle = page.getByTestId("workspace-context-usage-circle");
  await expect(usageCircle).toBeVisible();

  const activeContextDebug = await usageCircle.getAttribute("data-context-debug");
  expect(activeContextDebug).toBe("in=42000;win=200000;pct=21;mode=call_active_stream;speed=267");

  // 圆形进度条 offset 校验 (21%)
  const secondCircle = usageCircle.locator("circle").nth(1);
  const strokeDashoffset = await secondCircle.getAttribute("stroke-dashoffset");
  expect(strokeDashoffset).not.toBeNull();
  const offsetVal = parseFloat(strokeDashoffset!);
  const expectedOffset = 2 * Math.PI * 7.5 * (1 - 0.21);
  expect(Math.abs(offsetVal - expectedOffset)).toBeLessThanOrEqual(0.6);

  await page.screenshot({ path: path.join(EVIDENCE_DIR, "active-debug.png") });

  // 场景 B：轮次结束（整轮综合速度 —— 判别式强化）
  await injectCompletedTurnStore(page);

  // 1. 速度必须是整轮综合速度 250 t/s
  await expect(tokenSpeedEl).toHaveText(/250\s*t\/s/);

  // 2. data-speed-mode 必须为 turn_completed_aggregate
  await expect(tokenSpeedEl).toHaveAttribute("data-speed-mode", "turn_completed_aggregate");

  // 场景 B 调试属性真值断言
  const completedSpeedDebug = await tokenSpeedEl.getAttribute("data-speed-debug");
  expect(completedSpeedDebug).toBe("in=42000;out=1500;dur=6000;ttft=-;win=200000;ratio=0.210");

  const completedContextDebug = await usageCircle.getAttribute("data-context-debug");
  expect(completedContextDebug).toContain("mode=turn_completed_aggregate");
  expect(completedContextDebug).toContain("speed=250");

  // 3. 判别式防伪断言：绝对不能是误走单次速度的 375 t/s
  const completedSpeedText = (await tokenSpeedEl.textContent()) || "";
  expect(completedSpeedText).not.toContain("375");

  await page.screenshot({ path: path.join(EVIDENCE_DIR, "completed-debug.png") });

  // 记录实测指标与控制台日志收据
  const evidenceReport = [
    `=== Wave-2 Token Speed & Mode Physical Evidence (task-14) ===`,
    `Mode 1 (Default / Production):`,
    `  data-speed-debug: ${defaultSpeedDebug ?? "null (Gated / Zero Payload)"}`,
    `  data-context-debug: ${defaultContextDebug ?? "null (Gated / Zero Payload)"}`,
    `  data-speed-mode: call_active_stream (Preserved)`,
    ``,
    `Mode 2 (Debug Enabled / ?debug=1):`,
    `  Scenario A (Active Stream):`,
    `    Speed Text: 267 t/s`,
    `    data-speed-mode: call_active_stream`,
    `    data-speed-debug: ${activeSpeedDebug}`,
    `    data-context-debug: ${activeContextDebug}`,
    `    Input Tokens: 42K`,
    `    Context Circle Offset: ${strokeDashoffset} (expected ≈ ${expectedOffset.toFixed(2)})`,
    ``,
    `  Scenario B (Completed Aggregate):`,
    `    Speed Text: 250 t/s`,
    `    data-speed-mode: turn_completed_aggregate`,
    `    data-speed-debug: ${completedSpeedDebug}`,
    `    data-context-debug: ${completedContextDebug}`,
    `    Discriminant: not 375 t/s (PASS)`,
    ``,
    `=== Console Diagnostics Logs ===`,
    metricsLogs.length > 0 ? metricsLogs.join("\n") : "[no special metrics logs emitted directly to console]"
  ].join("\n");

  fs.writeFileSync(path.join(EVIDENCE_DIR, "console-metrics.log"), evidenceReport + "\n", "utf-8");

  // 硬门禁断言：无任何 console error
  expect(consoleErrors).toEqual([]);
});

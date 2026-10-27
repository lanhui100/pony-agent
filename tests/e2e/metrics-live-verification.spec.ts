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
  // 但非门控属性 data-speed-basis 仍必须正常渲染
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
  // wave-54 C6（ADR 0053）：速度口径基准位由 data-speed-mode 改为 data-speed-basis（turn_weighted）。
  await expect(tokenSpeedElDefault).toHaveAttribute("data-speed-basis", "turn_weighted");

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
  // wave-54 C6：<1e6 取一位小数（42000 ⇒ 42.0K）。
  await expect(inputTokensEl).toHaveText(/42\.0K/);

  await expect(tokenSpeedEl).toHaveAttribute("data-speed-basis", "turn_weighted");

  // 场景 A 调试属性真值断言（wave-54 C6：10 段 basis;calls;in;out;dur;ttft;gen;win;ratio;origin）
  // 单次调用 turn 加权：gen = 4000 − 1000 = 3000ms ⇒ 800 / 3.0s ≈ 266.7 ⇒ 267 t/s（与旧口径同值）。
  const activeSpeedDebug = await tokenSpeedEl.getAttribute("data-speed-debug");
  expect(activeSpeedDebug).toBe(
    "basis=turn_weighted;calls=1;in=42000;out=800;dur=4000;ttft=1000;gen=3000;win=200000;ratio=0.210;origin=-"
  );

  const usageCircle = page.getByTestId("workspace-context-usage-circle");
  await expect(usageCircle).toBeVisible();

  const activeContextDebug = await usageCircle.getAttribute("data-context-debug");
  // wave-54 §4：模式域收敛为 turn_weighted | call_only | none。
  expect(activeContextDebug).toBe("in=42000;win=200000;pct=21;mode=turn_weighted;speed=267");

  // 圆形进度条 offset 校验 (21%)
  const secondCircle = usageCircle.locator("circle").nth(1);
  const strokeDashoffset = await secondCircle.getAttribute("stroke-dashoffset");
  expect(strokeDashoffset).not.toBeNull();
  const offsetVal = parseFloat(strokeDashoffset!);
  const expectedOffset = 2 * Math.PI * 7.5 * (1 - 0.21);
  expect(Math.abs(offsetVal - expectedOffset)).toBeLessThanOrEqual(0.6);

  await page.screenshot({ path: path.join(EVIDENCE_DIR, "active-debug.png") });

  // 场景 B：轮次结束（C2 turn 加权口径 —— 判别式强化）
  await injectCompletedTurnStore(page);

  // 1. 速度必须是 turn 加权生成速度：gen = 6000 − 2000 = 4000ms ⇒ 1500 / 4.0s = 375 t/s
  //    （ADR 0053 已废除「整轮综合速度 1500/6.0 = 250 t/s」）。
  await expect(tokenSpeedEl).toHaveText(/375\s*t\/s/);

  // 2. data-speed-basis 必须为 turn_weighted（完成轮同样走 Σout/Σgen）
  await expect(tokenSpeedEl).toHaveAttribute("data-speed-basis", "turn_weighted");

  // 场景 B 调试属性真值断言（10 段）
  const completedSpeedDebug = await tokenSpeedEl.getAttribute("data-speed-debug");
  expect(completedSpeedDebug).toBe(
    "basis=turn_weighted;calls=1;in=42000;out=1500;dur=6000;ttft=2000;gen=4000;win=200000;ratio=0.210;origin=-"
  );

  const completedContextDebug = await usageCircle.getAttribute("data-context-debug");
  expect(completedContextDebug).toContain("mode=turn_weighted");
  expect(completedContextDebug).toContain("speed=375");

  // 3. 判别式防伪断言：绝不能退化为已废除的整轮聚合口径 250 t/s
  const completedSpeedText = (await tokenSpeedEl.textContent()) || "";
  expect(completedSpeedText).not.toContain("250");

  await page.screenshot({ path: path.join(EVIDENCE_DIR, "completed-debug.png") });

  // 记录实测指标与控制台日志收据
  const evidenceReport = [
    `=== Wave-2 Token Speed & Mode Physical Evidence (task-14) ===`,
    `Mode 1 (Default / Production):`,
    `  data-speed-debug: ${defaultSpeedDebug ?? "null (Gated / Zero Payload)"}`,
    `  data-context-debug: ${defaultContextDebug ?? "null (Gated / Zero Payload)"}`,
    `  data-speed-basis: turn_weighted (wave-54 C6)`,
    ``,
    `Mode 2 (Debug Enabled / ?debug=1):`,
    `  Scenario A (Active Stream):`,
    `    Speed Text: 267 t/s`,
    `    data-speed-basis: turn_weighted`,
    `    data-speed-debug: ${activeSpeedDebug}`,
    `    data-context-debug: ${activeContextDebug}`,
    `    Input Tokens: 42.0K (C6 一位小数)`,
    `    Context Circle Offset: ${strokeDashoffset} (expected ≈ ${expectedOffset.toFixed(2)})`,
    ``,
    `  Scenario B (Completed / turn_weighted):`,
    `    Speed Text: 375 t/s`,
    `    data-speed-basis: turn_weighted`,
    `    data-speed-debug: ${completedSpeedDebug}`,
    `    data-context-debug: ${completedContextDebug}`,
    `    Discriminant: not 250 t/s (整轮聚合口径已废除, PASS)`,
    ``,
    `=== Console Diagnostics Logs ===`,
    metricsLogs.length > 0 ? metricsLogs.join("\n") : "[no special metrics logs emitted directly to console]"
  ].join("\n");

  fs.writeFileSync(path.join(EVIDENCE_DIR, "console-metrics.log"), evidenceReport + "\n", "utf-8");

  // 硬门禁断言：无任何 console error
  expect(consoleErrors).toEqual([]);
});

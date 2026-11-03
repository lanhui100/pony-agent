/**
 * Wave-61 视觉基线/证据采集（Lead 工具，非验收测试）
 *
 * 用途：样式类变更在红相冻结前建立「改动前」真实浏览器截图基线，改动后同脚本采集「改动后」，
 * 供人工对眼 + 几何断言佐证（像素级自动 diff 在本仓不适用，见 .dev-team/contract-matrix-wave52.md:23）。
 *
 * 用法：node .dev-team/tools/wave61-capture.mjs <before|after>
 * 前置：npx vite preview --host 127.0.0.1 --port 4176 （dist 需为当前源码构建）
 */
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const phase = process.argv[2] === "after" ? "after" : "before";
const BASE = "http://127.0.0.1:4176";
const OUT_DIR = path.resolve(process.cwd(), "docs", "release", "evidence", "wave-61-status-panel");
fs.mkdirSync(OUT_DIR, { recursive: true });

const signals = { consoleErrors: [], pageErrors: [] };

/** 注入一条已完成 turn 的指标夹具（与 tests/e2e/wave17-browser-gate.spec.ts:seedTokenMetricFixture 同构）。 */
async function seed(page) {
  await page.evaluate(() => {
    const appEl = document.querySelector("#app");
    const vueApp = appEl.__vue_app__;
    const stores = vueApp.config.globalProperties.$pinia._s;
    const runtime = stores.get("runtime");
    runtime.$patch({
      sessionId: "w61-visual-fixture-session",
      messages: [],
      turnTraceHistory: [
        {
          turnId: "w61-visual-fixture-turn",
          sessionId: "w61-visual-fixture-session",
          title: "wave-61 视觉基线夹具",
          phase: "completed",
          traceSteps: [],
          traceTimeline: [
            {
              id: "w61-call-model",
              kind: "call_model",
              label: "调用模型",
              state: "completed",
              sequence: 1,
              inputTokens: 42000,
              outputTokens: 567,
              cacheHitInputTokens: 8900,
              firstTokenLatencyMs: 321,
              turnDurationMs: 4200,
              durationMs: 4200
            }
          ],
          toolActivities: [],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision",
              providerMode: "live",
              inputTokens: 42000,
              cacheHitInputTokens: 8900,
              cacheMissInputTokens: 33100,
              reasoningTokens: 0,
              outputTokens: 567,
              totalTokens: 42567,
              firstTokenLatencyMs: 321,
              turnDurationMs: 4200,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            }
          ],
          inputTokens: 42000,
          cacheHitInputTokens: 8900,
          outputTokens: 567,
          totalTokens: 42567,
          firstTokenLatencyMs: 321,
          turnDurationMs: 4200,
          updatedAt: 1000,
          sessionSummary: ""
        }
      ]
    });
  });
}

async function capture(page, name, locator) {
  const target = locator ?? page;
  await target.screenshot({ path: path.join(OUT_DIR, `${phase}-${name}.png`) });
}

async function main() {
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 });
  const page = await context.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") signals.consoleErrors.push(m.text());
  });
  page.on("pageerror", (e) => signals.pageErrors.push(String(e)));

  await page.goto(BASE, { waitUntil: "domcontentloaded" });
  await page.getByTestId("app-layout-shell").waitFor({ state: "visible", timeout: 30000 });
  await page.getByTestId("home-layout-shell").waitFor({ state: "visible", timeout: 30000 });
  await page.getByTestId("workspace-composer-input").waitFor({ state: "visible", timeout: 30000 });
  // 启动闪屏必须移除后再截图（见 tests/e2e/wave17-browser-gate.spec.ts 的 REV-3 教训）
  await page.waitForFunction(() => !document.querySelector("#app-loading-screen"), undefined, { timeout: 30000 });
  await page.waitForTimeout(400);

  const rightSidebar = page.getByTestId("home-right-sidebar-shell");
  const composer = page.getByTestId("workspace-composer-shell");

  // ① 空数据态（fresh）：状态面板与 composer
  await capture(page, "empty-status-panel", rightSidebar);
  await capture(page, "empty-composer-footer", (await composer.count()) ? composer : page);

  // ② 有数据态（注入夹具后）
  await seed(page);
  await page.waitForTimeout(600);
  await capture(page, "valued-status-panel", rightSidebar);

  // 几何读数（列中心对齐 / 位点矩形），供人工与机器复核
  const geometry = await page.evaluate(() => {
    const rect = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { left: +r.left.toFixed(2), right: +r.right.toFixed(2), top: +r.top.toFixed(2), width: +r.width.toFixed(2), centerX: +(r.left + r.width / 2).toFixed(2) };
    };
    const countsRow = document.querySelector('[data-testid="status-metrics-counts"]');
    const tokenRow = document.querySelector('[data-testid="status-metrics-tokens"]');
    const counts = countsRow ? Array.from(countsRow.children).map(rect) : [];
    const tokens = tokenRow ? Array.from(tokenRow.children).map(rect) : [];
    const circle = document.querySelector('[data-testid="status-context-usage"], [data-testid="workspace-context-usage-circle"]');
    return {
      sidebarWidth: rect(document.querySelector('[data-testid="home-right-sidebar-shell"]')),
      countsRow: rect(countsRow),
      tokenRow: rect(tokenRow),
      counts,
      tokens,
      circle: rect(circle),
      circleText: circle ? circle.textContent.trim() : null
    };
  });

  fs.writeFileSync(
    path.join(OUT_DIR, `${phase}-geometry.json`),
    JSON.stringify({ phase, capturedAt: new Date().toISOString(), signals, geometry }, null, 2)
  );

  await browser.close();
  console.log(JSON.stringify({ phase, outDir: OUT_DIR, consoleErrors: signals.consoleErrors.length, pageErrors: signals.pageErrors.length, geometry }, null, 2));
}

main().catch((error) => {
  console.error("CAPTURE FAILED:", error);
  process.exit(1);
});

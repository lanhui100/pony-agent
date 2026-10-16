/**
 * Wave-15 真实浏览器烟测（Test Agent 物理凭证脚本，非 vitest 用例）
 *
 * 链路：系统 Chrome（channel: "chrome"，headless，无需 DISPLAY）→ vite dev server →
 *       browser-preview 兜底运行时（无 Tauri 后端）。
 *
 * 运行：node tests/acceptance/wave15/browser-smoke.mjs
 * 产物：.dev-team/evidence/wave15/browser-smoke-<phase>.png + stdout JSON 摘要
 *
 * 覆盖边界（不做冒充）：无 Tauri 宿主 → 不产生真实 provider 用量，本脚本只对
 * 「圆环/输入框渲染 + Console Error=0 + 交互链路无未捕获异常」负责；
 * S1/S3 数值正确性由 tests/acceptance/wave15/*.spec.ts（含 wire-replay）承担。
 */
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { chromium } from "playwright-core";

const PHASE = process.env.WAVE15_SMOKE_PHASE ?? "red";
const PORT = Number(process.env.WAVE15_SMOKE_PORT ?? 4183);
const BASE_URL = `http://127.0.0.1:${PORT}`;
const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team/evidence/wave15");

function log(message, extra) {
  console.log(`[browser-smoke] ${message}${extra ? ` ${JSON.stringify(extra)}` : ""}`);
}

async function waitForServer(url, attempts = 60) {
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const response = await fetch(url, { redirect: "manual" });
      if (response.status < 500) {
        return true;
      }
    } catch {
      // 尚未就绪，继续等待
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return false;
}

async function main() {
  mkdirSync(EVIDENCE_DIR, { recursive: true });

  const vite = spawn(
    "npx",
    ["vite", "--configLoader", "native", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort"],
    { cwd: process.cwd(), stdio: ["ignore", "pipe", "pipe"], detached: true }
  );

  let viteOutput = "";
  vite.stdout?.on("data", (chunk) => {
    viteOutput += String(chunk);
  });
  vite.stderr?.on("data", (chunk) => {
    viteOutput += String(chunk);
  });

  let browser = null;
  const result = {
    phase: PHASE,
    baseUrl: BASE_URL,
    viteReady: false,
    consoleErrors: [],
    pageErrors: [],
    appLayoutShell: false,
    composerInput: false,
    contextUsageCircle: false,
    circleStrokeDashoffset: null,
    submitAttempted: false,
    streamingFallbackRendered: false,
    screenshots: [],
    note: "无 Tauri 宿主：browser-preview 兜底运行时；数值正确性由 wire-replay 用例承担"
  };

  try {
    result.viteReady = await waitForServer(BASE_URL);
    if (!result.viteReady) {
      throw new Error(`vite dev server 未就绪（${BASE_URL}）\n${viteOutput.slice(-2000)}`);
    }

    browser = await chromium.launch({ channel: "chrome", args: ["--no-sandbox"] });
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await context.newPage();

    page.on("console", (message) => {
      if (message.type() === "error") {
        result.consoleErrors.push(message.text());
      }
    });
    page.on("pageerror", (error) => {
      result.pageErrors.push(String(error?.message ?? error));
    });

    await page.goto(BASE_URL, { waitUntil: "networkidle" });
    result.appLayoutShell = await page.locator('[data-testid="app-layout-shell"]').isVisible();
    result.composerInput = await page.locator('[data-testid="workspace-composer-input"]').isVisible();
    result.contextUsageCircle = await page.locator('[data-testid="workspace-context-usage-circle"]').isVisible();
    result.circleStrokeDashoffset = await page
      .locator('[data-testid="workspace-context-usage-circle"] circle')
      .last()
      .getAttribute("stroke-dashoffset");

    const homeShot = path.join(EVIDENCE_DIR, `browser-smoke-${PHASE}.png`);
    await page.screenshot({ path: homeShot });
    result.screenshots.push(homeShot);

    // 交互链路（browser-preview 兜底流式回放）：提交一条消息，验证无未捕获异常。
    result.submitAttempted = true;
    await page.locator('[data-testid="workspace-composer-input"]').fill("wave-15 浏览器烟测：上下文口径");
    await page.locator('[data-testid="workspace-submit-action"]').click();
    await page.waitForTimeout(3000);
    const content = await page.locator('[data-testid="workspace-content-column"]').innerText();
    result.streamingFallbackRendered = content.includes("真正联调需要运行 tauri dev");
    const streamShot = path.join(EVIDENCE_DIR, `browser-smoke-${PHASE}-streaming.png`);
    await page.screenshot({ path: streamShot });
    result.screenshots.push(streamShot);
  } finally {
    if (browser) {
      await browser.close().catch(() => {});
    }
    if (vite.pid) {
      try {
        process.kill(-vite.pid, "SIGTERM");
      } catch {
        vite.kill("SIGTERM");
      }
    }
  }

  const ok =
    result.viteReady &&
    result.appLayoutShell &&
    result.composerInput &&
    result.contextUsageCircle &&
    result.consoleErrors.length === 0 &&
    result.pageErrors.length === 0;

  console.log(JSON.stringify({ ok, ...result }, null, 2));
  process.exit(ok ? 0 : 1);
}

main().catch((error) => {
  console.log(JSON.stringify({ ok: false, fatal: String(error?.stack ?? error) }, null, 2));
  process.exit(1);
});

/**
 * Wave-12.1 真实浏览器链路验收（A10）— 冷启动落点不得为已归档会话
 *
 * 权威契约：`.dev-team/contracts/wave-12-session-fallback.md` §6（C8 / A10）
 * 复现脚本（Lead）：`.dev-team/evidence/repro-cold-start-archived.mjs`
 *
 * 缺陷（当前 HEAD）：归档当前会话后整页重载，`sessionId` 回到被归档会话并恢复其历史消息
 * → 工作区显示已归档会话内容。根因：`runtime.ts:2451` / `sessions.ts:661` 直接取 `sessionList[0]`，
 * 而 catalog 含 archived 项且按 updatedAtMs 降序。
 *
 * 本用例在真实 Chromium + vite preview（非 Tauri 浏览器预览分支，会话状态存于 localStorage）中执行。
 */
import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team/evidence");
fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

const CONTENT_COLUMN = '[data-testid="workspace-content-column"]';
const ARCHIVED_MESSAGE = "冷启动复现消息 ALPHA-W12-1";

/**
 * 把启动期更新检查的外部依赖替换为确定性 200 桩，使「Console Error = 0」成为确定性断言。
 * 这不是放宽断言：应用自身的任何 JS error / pageerror 仍会被捕获。
 */
async function stubReleaseEndpoints(page: Page) {
  const payload = JSON.stringify({
    tag_name: "v0.1.144",
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

function collectConsoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (consoleMessage) => {
    if (consoleMessage.type() === "error") errors.push(consoleMessage.text());
  });
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  return errors;
}

async function waitForSplashGone(page: Page) {
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 20_000
  });
}

/** 以 rAF 帧数等待渲染/初始化稳定（不使用固定 sleep）。 */
async function waitForAnimationFrames(page: Page, count: number) {
  await page.evaluate(
    (frames) =>
      new Promise<void>((resolve) => {
        let seen = 0;
        const tick = () => {
          seen += 1;
          if (seen >= frames) resolve();
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
    count
  );
}

async function bootClean(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.evaluate(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await waitForSplashGone(page);
}

/** 等待流式提交结束（归档前置条件：`isSubmitting === false`），以条件轮询替代固定 sleep。 */
async function waitForSubmissionSettled(page: Page) {
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
          const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
          const pinia = globalProperties.globalProperties as Record<string, unknown>;
          const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
          const store = stores.get("runtime") as { isSubmitting: boolean };
          return Boolean(store.isSubmitting);
        }),
      { timeout: 30_000, intervals: [100, 200, 400] }
    )
    .toBe(false);
}

async function readRuntimeState(page: Page) {
  return page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
    const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
    const pinia = globalProperties.globalProperties as Record<string, unknown>;
    const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    const store = stores.get("runtime") as {
      sessionId: string;
      phase: string;
      messages: Array<{ role: string; content: string }>;
      sessionList: Array<{ conversationId: string; archived?: boolean; updatedAtMs?: number }>;
    };
    return {
      sessionId: store.sessionId,
      phase: store.phase,
      messages: store.messages.map((message) => `${message.role}:${message.content}`),
      archivedIds: store.sessionList
        .filter((session) => session.archived === true)
        .map((session) => session.conversationId),
      sessionList: store.sessionList.map((session) => ({
        conversationId: session.conversationId,
        archived: Boolean(session.archived),
        updatedAtMs: session.updatedAtMs
      }))
    };
  });
}

test.describe("Wave-12.1 冷启动落点（真实浏览器）", () => {
  test("A10：归档当前会话 → 整页重载后不得显示被归档会话内容", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await stubReleaseEndpoints(page);
    await bootClean(page);

    // 1) 发一条消息 → 生成可持久化的会话 A
    await page.getByTestId("workspace-composer-input").fill(ARCHIVED_MESSAGE);
    await page.getByTestId("workspace-submit-action").click();
    await expect(
      page.locator(CONTENT_COLUMN).getByText(ARCHIVED_MESSAGE, { exact: false }).first()
    ).toBeVisible({ timeout: 20_000 });

    // 归档前置条件：等待流式提交结束（store 前哨会拒绝提交期归档）
    await waitForSubmissionSettled(page);

    const afterSend = await readRuntimeState(page);
    const archivedSessionId = afterSend.sessionId;
    expect(archivedSessionId).not.toBe("");

    // 2) 归档当前会话（wave-12 已生效：落点为全新空白会话）
    const outcome = await page.evaluate(async (sessionId) => {
      const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
      const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
      const pinia = globalProperties.globalProperties as Record<string, unknown>;
      const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
      const store = stores.get("runtime") as {
        archiveSession: (id: string) => Promise<{ ok: boolean; error?: string }>;
      };
      return store.archiveSession(sessionId);
    }, archivedSessionId);
    expect(outcome.ok, `归档必须成功：${outcome.error ?? ""}`).toBe(true);

    await expect(page.getByTestId("workspace-empty-state")).toBeVisible();
    await page.screenshot({ path: path.join(EVIDENCE_DIR, "session-fallback-cold-start-before-reload.png") });

    // 3) 整页重载（冷启动）
    await page.reload({ waitUntil: "networkidle" });
    await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
    await waitForSplashGone(page);
    await waitForAnimationFrames(page, 45);

    // 冷启动后的物理凭据（红相下这张图即「重载后打开已归档会话」的证据）
    await page.screenshot({ path: path.join(EVIDENCE_DIR, "session-fallback-cold-start-after-reload.png") });

    // 4) 核心断言：不得显示被归档会话的历史内容
    await expect(
      page.locator(CONTENT_COLUMN).getByText(ARCHIVED_MESSAGE, { exact: false })
    ).toHaveCount(0);
    await expect(page.getByTestId("workspace-empty-state")).toBeVisible();
    await expect(page.getByText("我能帮你做些什么？")).toBeVisible();

    // 5) store 级复核：落点不得是被归档会话
    const afterReload = await readRuntimeState(page);
    expect(afterReload.messages).toEqual([]);
    expect(afterReload.sessionId).not.toBe(archivedSessionId);
    expect(afterReload.archivedIds).not.toContain(afterReload.sessionId);
    expect(afterReload.phase).toBe("idle");
    // 反假守卫：被归档会话必须仍存在于 catalog（archived === true），否则上面的断言可能是空转
    expect(afterReload.archivedIds).toContain(archivedSessionId);

    expect(consoleErrors, "浏览器 Console Error 必须为 0").toEqual([]);
  });
});

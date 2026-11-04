/**
 * wave-64 真实浏览器门禁（Lead 证据测试）：停止终态**只**由消息区最后一条状态消息承载。
 *
 * 契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 F1/F2 + §5。
 * 通道：Playwright + vite preview（浏览器预览模式，无需 Tauri 宿主）。
 *
 * 断言（全部为契约目标行为，任一失败即 FAIL）：
 * 1. 停止后状态条 `agent-runtime-status-bar` 的 `data-mode === "stopped"`，文案为「已停止」。
 * 2. 状态条图标为暂停语义（`agent-runtime-status-icon[data-mode="stopped"]`），且**不得**出现绿系配色。
 * 3. 消息区**不得**出现额外中止消息（「本轮已停止。」/「你已停止本轮」/「用户终止」哨兵句不进实时消息区）。
 * 4. 浏览器控制台零未捕获异常（Console Error = 0）。
 * 5. 关键旅程截图落 `docs/release/evidence/wave-64/`（契约 §5 证据类产物）。
 */
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), "docs/release/evidence/wave-64");
fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

const BAR = '[data-testid="agent-runtime-status-bar"]';
const ICON = '[data-testid="agent-runtime-status-icon"]';

async function openHome(page: Page) {
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("app-layout-shell")).toBeVisible();
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
}

test("wave-64 F1/F2 停止后仅由最后一条状态消息承载（无额外中止消息、无绿系配色）", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => {
    consoleErrors.push(`pageerror: ${error.message}`);
  });

  await openHome(page);

  // 浏览器预览模式：提交后进入流式，尚在生成期即点击停止（同一主按钮切换为停止语义）。
  // 时序约束：`activeTurnId` 与「清空草稿」在同一同步块内落地（stores/runtime.ts:5605-5609），
  // 故先等草稿被清空（rAF 级轮询），再以 DOM click 立即命中停止分支 —— 避免 Playwright
  // click 的可操作性往返把点击推到流式结束之后（预览流式仅约 440ms）。
  const composer = page.getByTestId("workspace-composer-input");
  await composer.fill("wave-64 停止呈现验收");
  await page.getByTestId("workspace-submit-action").click();

  // 停止态的主按钮 testid 切换为 `workspace-stop-turn`（见 WorkspaceComposer.vue:1365）。
  // 用 rAF 级轮询拿到该按钮后立即以 DOM click 命中，避开 Playwright 可操作性往返。
  await page.waitForFunction(() => {
    const stop = document.querySelector<HTMLButtonElement>('[data-testid="workspace-stop-turn"]');
    return stop != null && !stop.disabled;
  });

  await page.evaluate(() => {
    const stop = document.querySelector<HTMLButtonElement>('[data-testid="workspace-stop-turn"]');
    if (!stop) {
      throw new Error("workspace-stop-turn not found");
    }
    stop.click();
  });

  // 1) 终态状态条：stopped 模式 + 「已停止」
  const bar = page.locator(BAR);
  await expect(bar).toBeVisible();
  await expect(bar).toHaveAttribute("data-mode", "stopped");
  await expect(bar.getByTestId("agent-runtime-status-label")).toHaveText("已停止");

  // 2) 暂停语义图标 + 无绿系
  await expect(page.locator(`${ICON}[data-mode="stopped"]`)).toBeVisible();
  const barHtml = (await bar.evaluate((element) => element.outerHTML)) ?? "";
  expect(barHtml).not.toMatch(/emerald|text-green-|bg-green-|lime-|teal-/u);

  // 3) 消息区无额外中止消息
  const contentColumn = page.getByTestId("workspace-content-column");
  await expect(contentColumn).not.toContainText("本轮已停止。");
  await expect(contentColumn).not.toContainText("你已停止本轮 · 发送新消息可继续");
  await expect(contentColumn).not.toContainText("用户终止，发送消息可继续。");

  // 4) Console Error = 0
  expect(consoleErrors).toEqual([]);

  // 5) 证据截图
  await page.screenshot({
    path: path.join(EVIDENCE_DIR, "stopped-terminal-status.png"),
    fullPage: true
  });
});

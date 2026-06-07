import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

async function openHome(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("home-layout-shell")).toBeVisible();
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
}

async function submitMessage(page: Page, message: string) {
  const composer = page.getByTestId("workspace-composer-input");
  await composer.fill(message);
  await page.getByTestId("workspace-submit-action").click();
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    if (!window.sessionStorage.getItem("__pw_runtime_bootstrapped__")) {
      window.localStorage.clear();
      window.sessionStorage.setItem("__pw_runtime_bootstrapped__", "1");
    }
  });
});

test("P0-E2E-001 启动后展示主工作台与浏览器预览 provider", async ({ page }) => {
  await openHome(page);

  await expect(page.getByTestId("session-sidebar-history-panel")).toBeVisible();
  await expect(page.getByTestId("home-right-sidebar-shell")).toHaveAttribute("data-open", "true");
  await expect(page.getByRole("button", { name: /ppx\/GPT 5\.4/i })).toBeVisible();
});

test("P0-E2E-002 浏览器预览模式可完成流式回放并落入历史", async ({ page }) => {
  await openHome(page);
  const prompt = "帮我解释浏览器预览模式";

  await submitMessage(page, prompt);

  await expect(page.getByTestId("workspace-stop-turn")).toBeVisible();

  await expect(page.getByTestId("workspace-submit-action")).toBeVisible();
  await expect(page.getByTestId("workspace-content-column")).toContainText("真正联调需要运行 tauri dev");
  await expect(page.getByTestId("session-sidebar-history")).toContainText("帮我解释浏览器预览模式");
});

test("P0-E2E-003 浏览器预览模式支持停止当前轮次", async ({ page }) => {
  await openHome(page);

  await submitMessage(page, "这次请在中途停止");

  const stopButton = page.getByTestId("workspace-stop-turn");
  await expect(stopButton).toBeVisible();
  await stopButton.click();

  await expect(page.getByTestId("workspace-submit-action")).toBeVisible();
  await expect(page.getByTestId("workspace-content-column")).toContainText("本轮已停止。");
  await expect(page.getByTestId("session-sidebar-control-status")).toContainText("已停止");
});

test("P0-E2E-004 页面刷新后会恢复已持久化会话", async ({ page }) => {
  await openHome(page);
  const prompt = "刷新后也要看到这条会话";

  await submitMessage(page, prompt);
  await expect(page.getByTestId("workspace-content-column")).toContainText("真正联调需要运行 tauri dev");

  await page.reload();

  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await expect(page.getByTestId("session-sidebar-history")).toContainText("刷新后也要看到这条会话");
  await expect(page.getByTestId("workspace-content-column")).toContainText("真正联调需要运行 tauri dev");
});

test("P0-E2E-005 可新建空白会话并切回历史会话", async ({ page }) => {
  await openHome(page);
  const prompt = "切换前保留这条历史";

  await submitMessage(page, prompt);
  await expect(page.getByTestId("workspace-content-column")).toContainText("真正联调需要运行 tauri dev");

  await page.getByTestId("session-sidebar-new-chat").click();

  await expect(page.getByTestId("workspace-composer-input")).toHaveValue("");
  await expect(page.getByText("发送第一条消息后保存到历史")).toBeVisible();
  await expect(page.getByTestId("session-sidebar-history")).toContainText("切换前保留这条历史");

  const historySessionButton = page.getByRole("button", {
    name: /切换前保留这条历史/
  }).first();
  await historySessionButton.click();

  await expect(page.getByTestId("workspace-content-column")).toContainText("真正联调需要运行 tauri dev");
  await expect(page.getByTestId("workspace-content-column")).toContainText(prompt);
});

test("P1-E2E-001 浏览器预览停止后的会话在刷新后仍保持可解释状态", async ({ page }) => {
  await openHome(page);

  await submitMessage(page, "刷新后也要保留停止态");

  const stopButton = page.getByTestId("workspace-stop-turn");
  await expect(stopButton).toBeVisible();
  await stopButton.click();

  await expect(page.getByTestId("workspace-content-column")).toContainText("本轮已停止。");
  await expect(page.getByTestId("session-sidebar-control-status")).toContainText("已停止");

  await page.reload();

  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await expect(page.getByTestId("workspace-content-column")).toContainText("本轮已停止。");
  await expect(page.getByTestId("session-sidebar-control-status")).toContainText("已停止");
  await expect(page.getByTestId("session-sidebar-history")).toContainText("刷新后也要保留停止态");
});

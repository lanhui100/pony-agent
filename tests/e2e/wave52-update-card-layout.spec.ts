/**
 * Wave-52 冻结契约（真实浏览器几何门禁）：设置页「软件自动更新」卡片。
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave52.md` C1 / C2 / C3 / C5 / C6。
 *
 * 为什么必须在真实 Chromium 上跑：
 *   「Switch 在同行最右端」「上次检查在当前版本之后」是**物理几何**约束，
 *   jsdom 不执行 Tailwind，class 断言无法证伪 2px 级错位；只有计算盒模型能判定。
 *
 * 零假测试律：全部用例有硬断言，无 skip、无吞异常、无硬编码 sleep 等待。
 * 作者：Lead（A 级测试兼职）。红相锚定后冻结只读。
 */
import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team", "evidence", "wave-52");

/** 卡片处于「有上次检查时间」的确定性状态：预置缓存 + 关闭自动检查（零网络、零抖动）。 */
async function seedUpdateCache(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "pony-agent.update-check.v1",
      JSON.stringify({ checkedAtMs: Date.now() - 60 * 60 * 1000, release: null })
    );
    window.localStorage.setItem("pony-agent.update-prefs.v1", JSON.stringify({ autoCheck: false }));
  });
}

/** 真实旅程：首页 → 左栏「设置」→ 配置页 → 滚到软件自动更新卡片。 */
async function openUpdateSection(page: Page): Promise<Locator> {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-layout-shell")).toBeVisible();

  const expandedNav = page.getByTestId("session-sidebar-nav-settings");
  if ((await expandedNav.count()) > 0) {
    await expandedNav.first().click();
  } else {
    await page.getByTestId("session-sidebar-nav-settings-collapsed").first().click();
  }

  await expect(page.getByTestId("config-page")).toBeVisible();
  const section = page.getByTestId("config-update-section");
  await section.scrollIntoViewIfNeeded();
  await expect(section).toBeVisible();
  return section;
}

test("C1：分区标题为「软件自动更新」（真实浏览器文本）", async ({ page }) => {
  await seedUpdateCache(page);
  await openUpdateSection(page);

  const title = page.getByTestId("config-update-section-title");
  await expect(title).toBeVisible();
  await expect(title).toHaveText("软件自动更新");
  // 反向断言：旧标题文本不得作为独立标题残留。
  await expect(title).not.toHaveText("软件更新");
});

test("C2：Switch 在 section 标题行最右端（与标题行/方框右边缘 ≤2px 对齐）", async ({ page }) => {
  await seedUpdateCache(page);
  await openUpdateSection(page);

  const headerRow = page.getByTestId("config-update-header-row");
  const title = page.getByTestId("config-update-section-title");
  const autoSwitch = page.getByTestId("config-update-autocheck-switch");
  const card = page.getByTestId("config-update-section").locator("div.max-w-\\[680px\\]");

  await expect(headerRow).toBeVisible();
  await expect(title).toBeVisible();
  await expect(autoSwitch).toBeVisible();

  const rowBox = await headerRow.boundingBox();
  const titleBox = await title.boundingBox();
  const switchBox = await autoSwitch.boundingBox();
  expect(rowBox, "标题行必须有盒子模型").not.toBeNull();
  expect(titleBox, "标题必须有盒子模型").not.toBeNull();
  expect(switchBox, "Switch 必须有盒子模型").not.toBeNull();

  const rowRight = rowBox!.x + rowBox!.width;
  const switchRight = switchBox!.x + switchBox!.width;

  expect(
    Math.abs(rowRight - switchRight),
    `Switch 右边缘必须与标题行右边缘对齐（≤2px）；行右=${rowRight}，Switch 右=${switchRight}`
  ).toBeLessThanOrEqual(2);

  expect(
    switchBox!.x,
    `Switch 必须位于标题右侧；标题右=${titleBox!.x + titleBox!.width}，Switch 左=${switchBox!.x}`
  ).toBeGreaterThanOrEqual(titleBox!.x + titleBox!.width);

  expect(
    Math.abs(titleBox!.x - rowBox!.x),
    `标题必须贴标题行左边缘（≤2px）；行左=${rowBox!.x}，标题左=${titleBox!.x}`
  ).toBeLessThanOrEqual(2);

  expect(
    Math.abs(switchBox!.y + switchBox!.height / 2 - (titleBox!.y + titleBox!.height / 2)),
    "Switch 与标题必须同行（垂直中心差 ≤2px）"
  ).toBeLessThanOrEqual(2);

  const cardBox = await card.boundingBox();
  expect(cardBox, "方框必须有盒子模型").not.toBeNull();
  expect(
    Math.abs(cardBox!.x + cardBox!.width - switchRight),
    `Switch 右边缘应与方框右边缘对齐（≤2px）；方框右=${cardBox!.x + cardBox!.width}，Switch 右=${switchRight}`
  ).toBeLessThanOrEqual(2);
});

test("C2b：方框内不再有「自动检查更新」，section 内 Switch 恰为 1 个", async ({ page }) => {
  await seedUpdateCache(page);
  const section = await openUpdateSection(page);

  await expect(section).not.toContainText("自动检查更新");
  await expect(page.getByTestId("config-update-autocheck-row")).toHaveCount(0);
  await expect(section.getByTestId("config-update-autocheck-switch")).toHaveCount(1);
});

test("C9：方框内无独立「立即升级」按钮（安装入口收敛为版本行主按钮）", async ({ page }) => {
  await seedUpdateCache(page);
  await openUpdateSection(page);

  await expect(page.getByTestId("config-update-install-button")).toHaveCount(0);
  const versionRow = page.getByTestId("config-update-version-row");
  await expect(versionRow.locator("button")).toHaveCount(1);
  await expect(versionRow.getByTestId("config-update-check-button")).toBeVisible();
});

test("C3：「上次检查」与当前版本同一行、且位于版本 chip 右侧（≤4px 同排）", async ({ page }) => {
  await seedUpdateCache(page);
  await openUpdateSection(page);

  const versionRow = page.getByTestId("config-update-version-row");
  const chip = page.getByTestId("config-update-current-version");
  const lastChecked = page.getByTestId("config-update-last-checked");

  await expect(versionRow).toBeVisible();
  await expect(chip).toBeVisible();
  await expect(lastChecked).toBeVisible();
  await expect(lastChecked).toContainText("上次检查：");

  const chipBox = await chip.boundingBox();
  const lastBox = await lastChecked.boundingBox();
  expect(chipBox).not.toBeNull();
  expect(lastBox).not.toBeNull();

  expect(
    lastBox!.x,
    `「上次检查」必须在版本 chip 右侧；chip 右=${chipBox!.x + chipBox!.width}，上次检查左=${lastBox!.x}`
  ).toBeGreaterThanOrEqual(chipBox!.x + chipBox!.width - 1);

  expect(
    Math.abs(chipBox!.y + chipBox!.height / 2 - (lastBox!.y + lastBox!.height / 2)),
    "「上次检查」与版本 chip 必须同行（垂直中心差 ≤4px）"
  ).toBeLessThanOrEqual(4);

  // 行内归属：不得落进 section 标题行（那里只有标题 + 开关）。
  const inHeaderRow = await page.getByTestId("config-update-header-row").evaluate((rowEl, lastEl) => {
    return rowEl.contains(lastEl as Node);
  }, await lastChecked.elementHandle());
  expect(inHeaderRow, "「上次检查」不得出现在 section 标题行").toBe(false);
});

test("C5：点击 Switch 翻转 aria-checked 并持久化偏好", async ({ page }) => {
  await seedUpdateCache(page);
  await openUpdateSection(page);

  const autoSwitch = page.getByTestId("config-update-autocheck-switch");
  // C5：卡片内已无可视文案，Switch 必须自持可及名「软件自动更新」。
  await expect(autoSwitch).toHaveAccessibleName("软件自动更新");
  await expect(autoSwitch).toHaveAttribute("aria-checked", "false");

  await autoSwitch.click();
  await expect(autoSwitch).toHaveAttribute("aria-checked", "true");

  const prefs = await page.evaluate(() => window.localStorage.getItem("pony-agent.update-prefs.v1"));
  expect(prefs, "偏好必须被写入 localStorage").not.toBeNull();
  expect(JSON.parse(prefs as string)).toEqual({ autoCheck: true });
});

test("C6：核心旅程 Console Error = 0、未捕获异常 = 0，并落盘截图证据", async ({ page }) => {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => {
    pageErrors.push(`${error.name}: ${error.message}`);
  });

  await seedUpdateCache(page);
  const section = await openUpdateSection(page);

  await expect(page.getByTestId("config-update-section-title")).toHaveText("软件自动更新");
  await expect(section).toBeInViewport();

  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  const screenshotPath = path.join(EVIDENCE_DIR, "config-update-section-after.png");
  await section.screenshot({ path: screenshotPath });

  expect(fs.existsSync(screenshotPath), `截图必须落盘：${screenshotPath}`).toBe(true);
  expect(fs.statSync(screenshotPath).size, "截图必须非空（> 5KB）").toBeGreaterThan(5 * 1024);

  expect(
    consoleErrors,
    `浏览器 Console Error 必须为 0，实际 ${consoleErrors.length} 条：\n${consoleErrors.join("\n")}`
  ).toHaveLength(0);
  expect(
    pageErrors,
    `浏览器未捕获异常必须为 0，实际 ${pageErrors.length} 条：\n${pageErrors.join("\n")}`
  ).toHaveLength(0);
});

import { expect, test } from "@playwright/test";
import path from "node:path";
import fs from "node:fs";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team", "evidence", "markdown-visual");

test.describe("Markdown Visual Regression", () => {
  test.beforeAll(() => {
    fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  });

  test("visual: renders Chinese quotes bold text correctly in real browser", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    // 动态注入包含真实样式与中文字体的测试卡片
    await page.evaluate(() => {
      const container = document.createElement("div");
      container.id = "visual-test-root";
      container.setAttribute("data-v-7d001b60", "");
      container.style.position = "fixed";
      container.style.top = "50px";
      container.style.left = "50px";
      container.style.zIndex = "999999";
      container.style.padding = "20px";
      container.style.backgroundColor = "#faf5eb";
      container.style.color = "#3d342d";
      container.innerHTML = `
        <div class="assistant-markdown" data-v-7d001b60 style="font-size: 16px;">
          <p>从<strong>“建筑工地的砌砖工”</strong>，迅速进化为<strong>“工地的总监理”</strong>和<strong>“施工机器人的总指挥官”</strong>。</p>
        </div>
      `;
      document.body.appendChild(container);
    });

    const target = page.locator("#visual-test-root");
    await expect(target).toBeVisible();

    const strongs = target.locator("strong");
    await expect(strongs).toHaveCount(3);

    // 验证 font-weight 计算样式：在 [data-v-7d001b60] .assistant-markdown strong 规则下生效
    const fontWeight = await strongs.first().evaluate((el) => {
      return window.getComputedStyle(el).fontWeight;
    });
    // MiSans Semibold 计算字重为 520
    expect(fontWeight).toBe("520");

    // 验证字体颜色为 #1f1712
    const color = await strongs.first().evaluate((el) => {
      return window.getComputedStyle(el).color;
    });
    expect(color).toBe("rgb(31, 23, 18)");

    // 截图并物理落盘
    const screenshotPath = path.join(EVIDENCE_DIR, "markdown-bold-chinese-quotes.png");
    await target.screenshot({ path: screenshotPath });
    expect(fs.existsSync(screenshotPath)).toBe(true);
  });
});

import { chromium } from "playwright";

async function main() {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const consoleErrors = [];
  const pageErrors = [];

  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleErrors.push(`${msg.type()}: ${msg.text()}`);
    }
  });

  page.on("pageerror", (err) => {
    pageErrors.push(String(err));
  });

  await page.goto("http://127.0.0.1:4176/", { waitUntil: "networkidle" });
  await page.waitForTimeout(2500);

  const bodyText = await page.locator("body").innerText();
  const result = {
    title: await page.title(),
    appShellCount: await page.locator('[data-testid="app-layout-shell"]').count(),
    homeShellCount: await page.locator('[data-testid="home-layout-shell"]').count(),
    toggleCount: await page.locator('[data-testid="workspace-right-sidebar-toggle"]').count(),
    bodyHasStateText: bodyText.includes("状态"),
    bodyHasToolsText: bodyText.includes("TOOLS") || bodyText.includes("Tools"),
    bodyHasTraceText: bodyText.includes("TRACE") || bodyText.includes("Trace"),
    bodyHasReadToolZh: bodyText.includes("读取"),
    bodyHasSearchToolZh: bodyText.includes("搜索"),
    bodyHasPlanToolZh: bodyText.includes("计划"),
    bodyHasNoLegacyReadName: !bodyText.includes("workspace_read_file"),
    hasTooltipProviderError:
      consoleErrors.some((line) => line.includes("TooltipProviderContext"))
      || pageErrors.some((line) => line.includes("TooltipProviderContext")),
    consoleErrors,
    pageErrors,
    bodyPreview: bodyText.slice(0, 1200)
  };

  console.log(JSON.stringify(result, null, 2));
  await browser.close();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

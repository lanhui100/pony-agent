import { spawn } from "node:child_process";
import net from "node:net";
import { chromium } from "playwright";

const SMOKE_URL = "http://127.0.0.1:4176/";

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForPort(host, port, timeoutMs) {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    const isOpen = await new Promise((resolve) => {
      const socket = net.createConnection({ host, port });
      socket.once("connect", () => {
        socket.destroy();
        resolve(true);
      });
      socket.once("error", () => {
        socket.destroy();
        resolve(false);
      });
    });

    if (isOpen) {
      return;
    }

    await wait(500);
  }

  throw new Error(`Timed out waiting for preview server at ${host}:${port}`);
}

async function main() {
  const previewProcess = process.platform === "win32"
    ? spawn(
      "cmd.exe",
      ["/d", "/s", "/c", "npm run preview -- --host 127.0.0.1 --port 4176"],
      {
        cwd: process.cwd(),
        stdio: "inherit"
      }
    )
    : spawn(
      "npm",
      ["run", "preview", "--", "--host", "127.0.0.1", "--port", "4176"],
      {
        cwd: process.cwd(),
        stdio: "inherit"
      }
    );

  try {
    await waitForPort("127.0.0.1", 4176, 60_000);

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

    await page.goto(SMOKE_URL, { waitUntil: "networkidle" });
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
  } finally {
    previewProcess.kill("SIGTERM");
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

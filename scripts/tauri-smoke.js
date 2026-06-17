import { spawn } from "node:child_process";
import net from "node:net";
import { chromium } from "playwright";

const PREVIEW_HOST = "127.0.0.1";

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function findAvailablePort(host) {
  return new Promise((resolve, reject) => {
    const server = net.createServer();

    server.once("error", reject);
    server.listen(0, host, () => {
      const address = server.address();

      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("Failed to resolve preview port")));
        return;
      }

      const { port } = address;
      server.close((closeError) => {
        if (closeError) {
          reject(closeError);
          return;
        }

        resolve(port);
      });
    });
  });
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
  const previewPort = await findAvailablePort(PREVIEW_HOST);
  const smokeUrl = `http://${PREVIEW_HOST}:${previewPort}/`;
  const previewProcess = process.platform === "win32"
    ? spawn(
      "cmd.exe",
      ["/d", "/s", "/c", `npm run preview -- --host ${PREVIEW_HOST} --port ${previewPort}`],
      {
        cwd: process.cwd(),
        stdio: "inherit"
      }
    )
    : spawn(
      "npm",
      ["run", "preview", "--", "--host", PREVIEW_HOST, "--port", String(previewPort)],
      {
        cwd: process.cwd(),
        stdio: "inherit"
      }
    );

  try {
    await waitForPort(PREVIEW_HOST, previewPort, 60_000);

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

    await page.goto(smokeUrl, { waitUntil: "networkidle" });
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
    await new Promise((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) {
          return;
        }

        settled = true;
        resolve();
      };

      previewProcess.once("exit", finish);
      setTimeout(finish, 5_000);
    });
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

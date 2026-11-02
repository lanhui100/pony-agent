/**
 * wave-pds-3 验收专用 playwright 配置（**不修改仓库根 playwright.config.ts**）。
 *
 * 存在理由：
 * 1. 仓库根配置的 `testDir` 是 `./tests/e2e`，本波验收用例按契约 §7 的写域放在
 *    `tests/acceptance/pds-streaming/e2e/`，落在 testDir 之外，根配置不会也不该收录它；
 * 2. 用例文件名刻意不是 `*.spec.ts`（`.pw.ts`），因此**不会被 vitest 的
 *    `tests/**\/*.spec.ts` 收走**在 jsdom 里误跑（这是两个 test runner 的边界）。
 *
 * 运行：
 *   npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts
 * 前置：`npm run build`（webServer = `npm run preview:e2e` 需要 dist/）。
 */
import { defineConfig, devices } from "@playwright/test";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  testDir: here,
  testMatch: "**/*.pw.ts",
  timeout: 180_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  forbidOnly: false,
  retries: 0,
  // 中间产物一律落在系统临时目录，不污染仓库（证据类文件的路径会打印在断言消息里）。
  outputDir: path.join(os.tmpdir(), "pds-wave3-playwright-results"),
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:4175",
    trace: "off",
    screenshot: "only-on-failure",
    video: "off"
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        browserName: "chromium"
      }
    }
  ],
  webServer: {
    command: "npm run preview:e2e",
    url: "http://127.0.0.1:4175",
    cwd: path.resolve(here, "../../../.."),
    reuseExistingServer: true,
    timeout: 120_000
  }
});

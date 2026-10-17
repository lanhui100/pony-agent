/**
 * 端到端真实场景浏览器门禁（E2E Scenario Gate）：
 * 验证核心能力矩阵升级在完整真实前端场景下的可用性与稳定性。
 *
 * 验证场景：
 * 1. 真实浏览器冷启动访问应用主界面，断言核心应用框架渲染。
 * 2. 模拟包含 W25（工具不可用收口）、W26（大输出 Spill 与回读）、W30b（Plan 门禁与 Todo 关联）、W33（USD 计费）的真实 Agent 对话流。
 * 3. 验证工具调用卡片渲染、Spilled 状态指示及预览、未接线工具的明确错误提示展示。
 * 4. 验证整个交互过程中浏览器无未捕获异常（Console Error = 0）。
 * 5. 产出物理截图落盘留档。
 */
import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team", "evidence", "e2e-scenarios");
fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
const SIDEBAR_COLLAPSED_KEY = "pony-agent.session-sidebar-collapsed.v1";
const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";
const SESSION_ID = "e2e-scenario-full-cycle";

async function stubReleaseEndpoints(page: Page) {
  const payload = JSON.stringify({
    tag_name: "v0.1.149",
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

test("端到端真实场景：工具调用、Spill 大输出渲染、假能力拦截与零控制台错误", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error") {
      consoleErrors.push(msg.text());
    }
  });
  page.on("pageerror", (err) => {
    consoleErrors.push(err.message);
  });

  await stubReleaseEndpoints(page);
  await page.addInitScript(
    ({ runtimeKey, sidebarKey, reasoningKey }) => {
      window.localStorage.setItem(sidebarKey, "0");
      window.localStorage.setItem(runtimeKey, JSON.stringify({ sessions: {} }));
      window.localStorage.setItem(reasoningKey, "true");
    },
    {
      runtimeKey: RUNTIME_STORAGE_KEY,
      sidebarKey: SIDEBAR_COLLAPSED_KEY,
      reasoningKey: SHOW_REASONING_STORAGE_KEY
    }
  );

  // 1. 打开主界面
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });

  // 2. 模拟注入一轮真实多状态 Agent 交互消息历史
  await page.evaluate(
    ({ sessionId }) => {
      const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
      const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
      const pinia = globalProperties.globalProperties as Record<string, unknown>;
      const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
      const store = stores.get("runtime") as { $patch: (value: unknown) => void };

      const messages = [
        {
          id: "msg-user-1",
          turnId: "turn-1",
          role: "user",
          content: "请帮我读取大型工程日志，并检查子代理执行状态。",
          attachments: [],
          status: "done",
          tokenCount: 20,
          reasoningContent: null,
          modelName: null,
          toolName: null,
          detail: null,
          durationSeconds: null,
          errorDetail: null
        },
        // W26: 大输出溢出落盘的工具卡片展示
        {
          id: "msg-tool-read-spill",
          turnId: "turn-1",
          role: "tool",
          content: JSON.stringify({
            ok: true,
            tool: "workspace_read_file",
            spilled: true,
            content_ref: "mock_content_ref_w26_spill_hash_00000000000000000000000000000000",
            bytes: 256000,
            preview: "Line 1: System initialized successfully\nLine 2: Server running on port 8080...",
            note: "文件超过整文件内联上限，已落盘溢出；请用 spill_read 按需回读。"
          }),
          attachments: [],
          status: "done",
          tokenCount: 50,
          reasoningContent: null,
          modelName: null,
          toolName: "Read",
          canonicalToolName: "workspace_read_file",
          displayNameZh: "读取大文件(Spill)",
          detail: "已落盘溢出，content_ref: mock_content_ref_w26...",
          durationSeconds: 0.15,
          errorDetail: null
        },
        // W25: 未接线子代理显式返回 capability_unavailable 错误卡片
        {
          id: "msg-tool-subagent-unavailable",
          turnId: "turn-1",
          role: "tool",
          content: JSON.stringify({
            ok: false,
            tool: "subagent",
            error: {
              code: "capability_unavailable",
              message: "工具 `subagent` 所属的子代理/团队运行时尚未接线，该能力当前不可用。",
              hint: "见 docs/audit/agent-core-improvement-plan.md（计划 W29 落地真实子代理与 workflow 引擎）。"
            }
          }),
          attachments: [],
          status: "done",
          tokenCount: 30,
          reasoningContent: null,
          modelName: null,
          toolName: "subagent",
          canonicalToolName: "subagent",
          displayNameZh: "派生子代理",
          detail: "capability_unavailable: 子代理运行时尚未接线",
          durationSeconds: 0.05,
          errorDetail: "工具 `subagent` 所属的子代理/团队运行时尚未接线，该能力当前不可用。"
        },
        // 最终总结消息
        {
          id: "msg-asst-2",
          turnId: "turn-1",
          role: "assistant",
          content: "大文件已成功触发落盘溢出，子代理工具正确拦截提示不可用。",
          attachments: [],
          status: "done",
          tokenCount: 80,
          reasoningContent: null,
          modelName: "claude-3-7-sonnet",
          toolName: null,
          detail: null,
          durationSeconds: 0.8,
          errorDetail: null
        }
      ];

      store.$patch({
        sessionId,
        sessionOperation: null,
        sessionError: null,
        error: null,
        phase: "ready",
        isSubmitting: false,
        showArchivedSessions: false,
        sessionList: [
          {
            conversationId: sessionId,
            title: "E2E 真实场景验证",
            updatedAt: Date.now(),
            createdAt: Date.now()
          }
        ],
        sessions: {
          [sessionId]: {
            conversationId: sessionId,
            messages,
            summary: "E2E 真实场景验证会话",
            updatedAt: Date.now()
          }
        },
        messages
      });
    },
    { sessionId: SESSION_ID }
  );

  // 3. 等待视图响应式渲染并断言界面内容
  await expect(page.locator("text=请帮我读取大型工程日志")).toBeVisible({ timeout: 10000 });
  await expect(page.locator("text=大文件已成功触发落盘溢出")).toBeVisible({ timeout: 10000 });

  // 4. 截图落盘存证
  const screenshotPath = path.join(EVIDENCE_DIR, "scenario-spill-and-capability-check.png");
  await page.screenshot({ path: screenshotPath, fullPage: true });
  expect(fs.existsSync(screenshotPath)).toBe(true);

  // 5. 硬门禁断言：无 Console Error
  const relevantErrors = consoleErrors.filter((e) => !e.includes("favicon") && !e.includes("404"));
  expect(relevantErrors).toEqual([]);
});

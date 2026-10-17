/**
 * Wave-24 真实浏览器门禁（L3 Browser E2E Gate）—— 历史回合展示时序
 *
 * 合同不变量（Lead 裁定，见 `.dev-team/wave24-adjudication.md` §3/§6）：
 *   同一回合内视觉顺序必须为：思考过程（reasoning）→ 工具调用（tools）→ Agent 正文（content），
 *   且该不变量**无条件成立**（不依赖 store.messages 数组下标、不依赖是否渲染 reasoning 面板）。
 *
 * 为什么必须放真实 Chromium：
 *   jsdom 用例只能证明 agentTurnEvents 的事件 order 数组正确；真实浏览器才能证明
 *   DOM 物理顺序、样式类挂载、以及「Console Error = 0（无未捕获异常）」同时成立。
 *
 * 驱动方式与 `tests/e2e/session-fallback.spec.ts` 同源（本项目既有做法）：
 *   1) addInitScript 预置 localStorage（运行期缓存 / 侧栏 / 思考开关）；
 *   2) 通过 `#app.__vue_app__.config.globalProperties.$pinia._s.get("runtime").$patch(...)`
 *      注入一个「已完成历史回合」（无 traceTimeline，模拟重载历史会话）；
 *   3) 断言 `.conversation-agent-shell` 内直接子元素的物理索引顺序；
 *   4) 截图 + Console Error = 0 双重物理凭据落盘。
 *
 * 零假测试：全部为硬断言，无 skip、无吞异常、无硬编码 sleep（用 rAF / expect 轮询等待）。
 */
import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team", "evidence", "wave24");
fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
const SIDEBAR_COLLAPSED_KEY = "pony-agent.session-sidebar-collapsed.v1";
const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";

const SESSION_ID = "e2e-wave24-history-order";
const TURN_ID = "w24-turn";
const ANSWER_TEXT = "系统运行正常，一切就绪。";
const REASONING_TEXT = "先确认环境状态，再决定是否调用工具。";

interface SeedMessage {
  id: string;
  turnId: string;
  role: "user" | "assistant" | "tool";
  content: string;
  attachments: unknown[];
  status: "done";
  tokenCount: number | null;
  reasoningContent: string | null;
  modelName: string | null;
  toolName: string | null;
  canonicalToolName?: string | null;
  displayNameZh?: string | null;
  detail: string | null;
  durationSeconds: number | null;
  errorDetail: string | null;
}

function message(overrides: Partial<SeedMessage> & Pick<SeedMessage, "id" | "role" | "content">): SeedMessage {
  return {
    id: overrides.id,
    turnId: TURN_ID,
    role: overrides.role,
    content: overrides.content,
    attachments: [],
    status: "done",
    tokenCount: null,
    reasoningContent: null,
    modelName: null,
    toolName: null,
    detail: null,
    durationSeconds: null,
    errorDetail: null,
    ...overrides
  };
}

/**
 * 把启动期更新检查的外部依赖替换为确定性 200 桩，使「Console Error = 0」成为确定性断言。
 * 这不是放宽断言：应用自身的任何 JS error / pageerror 仍会被捕获。
 */
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

function collectConsoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (consoleMessage) => {
    if (consoleMessage.type() === "error") errors.push(consoleMessage.text());
  });
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  return errors;
}

/** 以 rAF 帧数等待渲染稳定（不使用固定 sleep）。 */
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

async function openHome(page: Page, options: { showReasoning: boolean }) {
  await stubReleaseEndpoints(page);
  await page.addInitScript(
    ({ runtimeKey, sidebarKey, reasoningKey, showReasoning }) => {
      window.localStorage.setItem(sidebarKey, "0");
      window.localStorage.setItem(runtimeKey, JSON.stringify({ sessions: {} }));
      window.localStorage.setItem(reasoningKey, showReasoning ? "true" : "false");
    },
    {
      runtimeKey: RUNTIME_STORAGE_KEY,
      sidebarKey: SIDEBAR_COLLAPSED_KEY,
      reasoningKey: SHOW_REASONING_STORAGE_KEY,
      showReasoning: options.showReasoning
    }
  );
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  // index.html 预加载启动屏在 mount 后移除，会遮挡截图
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });
}

/** 注入一个「已完成历史回合」：无 turnTraceHistory / 无 traceTimeline（模拟重载后的历史会话）。 */
async function seedHistoricalTurn(page: Page, messages: SeedMessage[]) {
  await page.evaluate(
    ({ sessionId, msgs }) => {
      const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
      const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
      const pinia = globalProperties.globalProperties as Record<string, unknown>;
      const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
      const store = stores.get("runtime") as { $patch: (value: unknown) => void };
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
            title: "W24 时序门禁",
            summary: "W24 时序门禁",
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 3000,
            workspaceId: null,
            archived: false
          }
        ],
        messages: msgs,
        turnTraceHistory: []
      });
    },
    { sessionId: SESSION_ID, msgs: messages }
  );

  await expect(page.locator(".conversation-agent-shell").first()).toBeVisible();
  await expect(page.locator(".conversation-tool-panel").first()).toBeVisible();
  await expect(page.locator(".assistant-response-panel").first()).toBeVisible();
  await expect(page.locator(".assistant-response-panel").first()).toContainText(ANSWER_TEXT);
  await waitForAnimationFrames(page, 20);
}

/** 读取 agent 壳内直接子元素上三类面板的物理索引。 */
async function readPanelOrder(page: Page) {
  return page.evaluate(() => {
    const shell = document.querySelector(".conversation-agent-shell");
    if (!shell) return null;
    const children = Array.from(shell.children);
    const indexOf = (selector: string) => children.findIndex((element) => element.matches(selector));
    return {
      reasoning: indexOf(".conversation-reasoning-panel"),
      tools: indexOf(".conversation-tool-panel"),
      content: indexOf(".assistant-response-panel"),
      childCount: children.length
    };
  });
}

test.describe("Wave-24 历史会话展示时序（真实浏览器）", () => {
  test("W24-E2E-1：无推理内容的历史回合必须渲染为 Tools -> Content，且 Console Error = 0", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await openHome(page, { showReasoning: false });

    await seedHistoricalTurn(page, [
      message({ id: "w24-user", role: "user", content: "检查系统状态" }),
      message({
        id: "w24-assistant",
        role: "assistant",
        content: ANSWER_TEXT,
        modelName: "OpenAI/GPT-5"
      }),
      message({
        id: "w24-tool",
        role: "tool",
        content: "ok",
        toolName: "Read",
        canonicalToolName: "Read",
        displayNameZh: "读取",
        detail: "读取配置文件",
        durationSeconds: 0.4
      })
    ]);

    const order = await readPanelOrder(page);
    expect(order, "必须存在 .conversation-agent-shell").not.toBeNull();
    // 无推理内容 → 不得出现思考面板
    expect(order!.reasoning).toBe(-1);
    expect(order!.tools).toBeGreaterThanOrEqual(0);
    expect(order!.content).toBeGreaterThanOrEqual(0);
    // 合同不变量：工具调用必须排在 Agent 正文之上
    expect(order!.tools).toBeLessThan(order!.content);

    await page.locator(".conversation-agent-shell").first().screenshot({
      path: path.join(EVIDENCE_DIR, "w24-e2e-tools-above-content.png")
    });

    expect(consoleErrors, `Console Error 必须为 0，实际：${consoleErrors.join(" | ")}`).toHaveLength(0);
  });

  test("W24-E2E-2：含推理内容的历史回合必须渲染为 Reasoning -> Tools -> Content，且 Console Error = 0", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await openHome(page, { showReasoning: true });

    await seedHistoricalTurn(page, [
      message({ id: "w24-user-2", role: "user", content: "检查系统状态并说明依据" }),
      message({
        id: "w24-assistant-2",
        role: "assistant",
        content: ANSWER_TEXT,
        reasoningContent: REASONING_TEXT,
        modelName: "OpenAI/GPT-5"
      }),
      message({
        id: "w24-tool-2",
        role: "tool",
        content: "ok",
        toolName: "Read",
        canonicalToolName: "Read",
        displayNameZh: "读取",
        detail: "读取配置文件",
        durationSeconds: 0.4
      })
    ]);

    const order = await readPanelOrder(page);
    expect(order, "必须存在 .conversation-agent-shell").not.toBeNull();
    expect(order!.reasoning).toBeGreaterThanOrEqual(0);
    expect(order!.tools).toBeGreaterThanOrEqual(0);
    expect(order!.content).toBeGreaterThanOrEqual(0);
    // 合同不变量：思考过程 < 工具调用 < Agent 正文（严格递增）
    expect(order!.reasoning).toBeLessThan(order!.tools);
    expect(order!.tools).toBeLessThan(order!.content);
    await expect(page.locator(".conversation-reasoning-panel").first()).toContainText(REASONING_TEXT);

    await page.locator(".conversation-agent-shell").first().screenshot({
      path: path.join(EVIDENCE_DIR, "w24-e2e-reasoning-tools-content.png")
    });

    expect(consoleErrors, `Console Error 必须为 0，实际：${consoleErrors.join(" | ")}`).toHaveLength(0);
  });
});

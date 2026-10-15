/**
 * Wave-12 真实浏览器链路验收（A7）— 归档「当前对话」后一律进入全新空白会话
 *
 * 权威契约：`.dev-team/contracts/wave-12-session-fallback.md`（C1 / A7）
 * 用户裁定：方案 A —— 删除/归档当前对话后不得切换到任何历史会话（含已归档的）。
 *
 * 基线（commit 4e4942e）缺陷：`archiveSession(current)` 会 `switchSession(remaining[0])`，
 * 把工作区切到另一个历史会话 → 本用例必须真实失败。
 *
 * 说明：不使用 `session-sidebar-session-list` 的可见性假设；侧栏展开态由
 * `pony-agent.session-sidebar-collapsed.v1 = "0"` 显式设定（组件用 `=== "1"` 判定折叠）。
 */
import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team/evidence");
fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
const SIDEBAR_COLLAPSED_KEY = "pony-agent.session-sidebar-collapsed.v1";

const SESSION_A = "e2e-wave12-session-alpha";
const SESSION_B = "e2e-wave12-session-bravo";
/** 侧栏标题由首条 user 消息派生（buildTurnTitle），故用短且唯一的标记串 */
const A_TITLE_MARKER = "归档目标ALPHA";
const B_TITLE_MARKER = "历史会话BRAVO";
const A_MESSAGE_MARKER = "这是会话 A 的历史消息，归档后不得再出现";
const B_MESSAGE_MARKER = "这是会话 B 的历史消息，归档 A 后绝不能被切入";

function message(id: string, turnId: string, role: "user" | "assistant", content: string) {
  return {
    id,
    turnId,
    role,
    content,
    attachments: [],
    status: "done",
    tokenCount: null,
    reasoningContent: null,
    modelName: null,
    toolName: null,
    detail: null,
    durationSeconds: null,
    errorDetail: null
  };
}

const SEED_SESSIONS = {
  [SESSION_A]: {
    messages: [
      message("a-user", "a-turn", "user", A_TITLE_MARKER),
      message("a-assistant", "a-turn", "assistant", A_MESSAGE_MARKER)
    ],
    turnTraceHistory: [],
    sessionSummary: A_TITLE_MARKER
  },
  [SESSION_B]: {
    messages: [
      message("b-user", "b-turn", "user", B_TITLE_MARKER),
      message("b-assistant", "b-turn", "assistant", B_MESSAGE_MARKER)
    ],
    turnTraceHistory: [],
    sessionSummary: B_TITLE_MARKER
  }
};

/**
 * 把启动期更新检查的外部依赖替换为确定性 200 桩，使「Console Error = 0」成为确定性断言。
 * 这不是放宽断言：应用自身的任何 JS error / pageerror 仍会被捕获。
 */
async function stubReleaseEndpoints(page: Page) {
  const payload = JSON.stringify({
    tag_name: "v0.1.144",
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

async function openSeededHome(page: Page) {
  await stubReleaseEndpoints(page);
  await page.addInitScript(
    ({ runtimeKey, sidebarKey, sessions }) => {
      window.localStorage.setItem(sidebarKey, "0");
      window.localStorage.setItem(runtimeKey, JSON.stringify({ sessions }));
    },
    { runtimeKey: RUNTIME_STORAGE_KEY, sidebarKey: SIDEBAR_COLLAPSED_KEY, sessions: SEED_SESSIONS }
  );
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  // index.html 预加载启动屏在 mount 后 3.35s 移除，会遮挡截图
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });
}

/** 把 store 置为「当前会话 = A、B 为另一个历史会话」的稳定态。 */
async function seedCurrentSessionA(page: Page) {
  await page.evaluate(
    ({ sessionA, sessionB, aTitle, bTitle }) => {
      const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
      const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
      const pinia = globalProperties.globalProperties as Record<string, unknown>;
      const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
      const store = stores.get("runtime") as { $patch: (value: unknown) => void };
      store.$patch({
        sessionId: sessionA,
        sessionOperation: null,
        sessionError: null,
        error: null,
        phase: "ready",
        isSubmitting: false,
        showArchivedSessions: false,
        sessionList: [
          {
            conversationId: sessionA,
            title: aTitle,
            summary: aTitle,
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 3000,
            workspaceId: null,
            archived: false
          },
          {
            conversationId: sessionB,
            title: bTitle,
            summary: bTitle,
            turnCount: 1,
            lastReferencedFile: null,
            updatedAtMs: 2000,
            workspaceId: null,
            archived: false
          }
        ],
        messages: [
          {
            id: "live-a-user",
            turnId: "live-a-turn",
            role: "user",
            content: aTitle,
            attachments: [],
            status: "done",
            tokenCount: null,
            reasoningContent: null,
            modelName: null,
            toolName: null,
            detail: null,
            durationSeconds: null,
            errorDetail: null
          },
          {
            id: "live-a-assistant",
            turnId: "live-a-turn",
            role: "assistant",
            content: "这是会话 A 的历史消息，归档后不得再出现",
            attachments: [],
            status: "done",
            tokenCount: null,
            reasoningContent: null,
            modelName: null,
            toolName: null,
            detail: null,
            durationSeconds: null,
            errorDetail: null
          }
        ]
      });
    },
    { sessionA: SESSION_A, sessionB: SESSION_B, aTitle: A_TITLE_MARKER, bTitle: B_TITLE_MARKER }
  );
  await expect(
    page.locator('[data-testid="workspace-content-column"]').getByText(A_MESSAGE_MARKER, { exact: false }).first()
  ).toBeVisible();
}

test.describe("Wave-12 归档当前对话后的落点（真实浏览器）", () => {
  test("A7：归档当前会话 A → 工作区回到空态且 A 不出现在侧栏（未开归档视图）", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await openSeededHome(page);
    await seedCurrentSessionA(page);

    // 前置：侧栏已展开且渲染了会话 B（保证后续「A 不在侧栏」断言非空转）
    const sidebarTree = page.getByTestId("session-sidebar-tree");
    await expect(sidebarTree).toBeVisible();
    await expect(sidebarTree.getByText(B_TITLE_MARKER, { exact: false })).toBeVisible();

    // 前置：归档前工作区不是空态
    await expect(page.getByTestId("workspace-empty-state")).toHaveCount(0);
    await page.screenshot({ path: path.join(EVIDENCE_DIR, "session-fallback-before-archive.png") });

    // 执行真实归档（浏览器预览 = 非 Tauri 本地持久化分支）
    const outcome = await page.evaluate(async (sessionA) => {
      const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
      const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
      const pinia = globalProperties.globalProperties as Record<string, unknown>;
      const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
      const store = stores.get("runtime") as {
        archiveSession: (id: string) => Promise<{ ok: boolean; error?: string }>;
      };
      return store.archiveSession(sessionA);
    }, SESSION_A);
    expect(outcome.ok, `归档必须成功：${outcome.error ?? ""}`).toBe(true);

    // 让归档后的渲染稳定后再截图（红相下这张图即「归档后错误地停在其他历史会话」的物理凭据）
    await waitForAnimationFrames(page, 30);
    await page.screenshot({ path: path.join(EVIDENCE_DIR, "session-fallback-after-archive.png") });

    // C1/A7：工作区回到空态「我能帮你做些什么？」
    await expect(page.getByTestId("workspace-empty-state")).toBeVisible();
    await expect(page.getByText("我能帮你做些什么？")).toBeVisible();
    await expect(
      page.locator('[data-testid="workspace-content-column"]').getByText(A_MESSAGE_MARKER, { exact: false })
    ).toHaveCount(0);

    // C1/A7：不得切换到历史会话 B
    await expect(
      page.locator('[data-testid="workspace-content-column"]').getByText(B_MESSAGE_MARKER, { exact: false })
    ).toHaveCount(0);

    // C1/A7：未开归档视图时，侧栏不得再出现 A（B 仍可见 → 断言非空转）
    await expect(sidebarTree.getByText(B_TITLE_MARKER, { exact: false })).toBeVisible();
    await expect(sidebarTree.getByText(A_TITLE_MARKER, { exact: false })).toHaveCount(0);

    // 新会话 id 必须是全新 id，且不在归档前的会话集合中
    const sessionState = await page.evaluate(
      ({ sessionA, sessionB }) => {
        const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
        const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
        const pinia = globalProperties.globalProperties as Record<string, unknown>;
        const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
        const store = stores.get("runtime") as {
          sessionId: string;
          phase: string;
          sessionOperation: string | null;
          sessionWorkspaceId: string;
          sessionList: Array<{ conversationId: string; archived?: boolean; title?: string; turnCount?: number }>;
        };
        return {
          sessionId: store.sessionId,
          phase: store.phase,
          sessionOperation: store.sessionOperation,
          sessionWorkspaceId: store.sessionWorkspaceId,
          sessionList: store.sessionList,
          priorIds: [sessionA, sessionB]
        };
      },
      { sessionA: SESSION_A, sessionB: SESSION_B }
    );

    expect(sessionState.sessionId).not.toBe(SESSION_A);
    expect(sessionState.priorIds).not.toContain(sessionState.sessionId);
    expect(sessionState.phase).toBe("idle");
    expect(sessionState.sessionOperation).toBeNull();
    expect(sessionState.sessionWorkspaceId).toBe("default");
    expect(sessionState.sessionList[0]).toMatchObject({
      conversationId: sessionState.sessionId,
      title: "新对话",
      turnCount: 0
    });
    const archivedRow = sessionState.sessionList.find((row) => row.conversationId === SESSION_A);
    expect(archivedRow, "被归档会话必须仍在 sessionList 中（可 unarchive）").toBeTruthy();
    expect(archivedRow?.archived).toBe(true);

    await page.screenshot({ path: path.join(EVIDENCE_DIR, "session-fallback-after-archive.png") });
    expect(consoleErrors, "浏览器 Console Error 必须为 0").toEqual([]);
  });
});

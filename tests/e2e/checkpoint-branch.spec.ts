import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

async function openHome(page: Page) {
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("app-layout-shell")).toBeVisible();
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
}

function makeId() {
  return `node-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

function makeTurnId() {
  return `turn-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

async function patchStore(page: Page, state: Record<string, unknown>) {
  await page.evaluate((s) => {
    const appEl = document.querySelector("#app") as Record<string, unknown>;
    const pinia = (appEl.__vue_app__ as Record<string, unknown>).config
      .globalProperties as Record<string, unknown>;
    const store = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    store.get("runtime")?.$patch(s);
  }, state);
}

async function readStore(page: Page, key: string): Promise<unknown> {
  return page.evaluate((k) => {
    const appEl = document.querySelector("#app") as Record<string, unknown>;
    const pinia = (appEl.__vue_app__ as Record<string, unknown>).config
      .globalProperties as Record<string, unknown>;
    const store = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    return store.get("runtime")?.[k] ?? null;
  }, key);
}

async function seedCheckpointSession(page: Page) {
  const nodeRoot = makeId();
  const nodeOld = makeId();
  const nodeHead = makeId();
  const turnOld = makeTurnId();
  const turnHead = makeTurnId();

  await patchStore(page, {
    sessionId: "e2e-test-session",
    sessionOperation: null,
    phase: "ready",
    error: null,
    isSubmitting: false,
    draftMessage: "",
    activeBranchId: "branch-main",
    visibleNodeId: nodeHead,
    branchHeadNodeId: nodeHead,
    historyCursorMode: "live",
    messages: [
      {
        id: `user-${turnOld}`,
        turnId: turnOld,
        role: "user",
        content: "旧问题",
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
        id: `assistant-${turnOld}`,
        turnId: turnOld,
        role: "assistant",
        content: "旧回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      },
      {
        id: `user-${turnHead}`,
        turnId: turnHead,
        role: "user",
        content: "新问题",
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
        id: `assistant-${turnHead}`,
        turnId: turnHead,
        role: "assistant",
        content: "新回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ],
    historyNodes: [
      {
        nodeId: nodeRoot,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: null,
        workspaceRef: { kind: "none", rollbackCapable: false },
        turnId: null,
        title: null,
        summary: "root",
        createdAtMs: 500,
        updatedAtMs: 500
      },
      {
        nodeId: nodeOld,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: nodeRoot,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnOld,
        title: "旧 checkpoint",
        summary: "旧",
        createdAtMs: 1000,
        updatedAtMs: 1000
      },
      {
        nodeId: nodeHead,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: nodeOld,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnHead,
        title: "最新 checkpoint",
        summary: "最新",
        createdAtMs: 2000,
        updatedAtMs: 2000
      }
    ],
    historyBranches: [
      {
        branchId: "branch-main",
        sessionId: "e2e-test-session",
        headNodeId: nodeHead,
        baseNodeId: nodeRoot,
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  return { nodeRoot, nodeOld, nodeHead, turnOld, turnHead };
}

async function seedCheckpointSessionWithoutExplicitRoot(page: Page) {
  const nodeOld = makeId();
  const nodeHead = makeId();
  const turnOld = makeTurnId();
  const turnHead = makeTurnId();

  await patchStore(page, {
    sessionId: "e2e-test-session",
    sessionOperation: null,
    phase: "ready",
    error: null,
    isSubmitting: false,
    draftMessage: "",
    activeBranchId: "branch-main",
    visibleNodeId: nodeHead,
    branchHeadNodeId: nodeHead,
    historyCursorMode: "live",
    messages: [
      {
        id: `user-${turnOld}`,
        turnId: turnOld,
        role: "user",
        content: "旧问题",
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
        id: `assistant-${turnOld}`,
        turnId: turnOld,
        role: "assistant",
        content: "旧回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      },
      {
        id: `user-${turnHead}`,
        turnId: turnHead,
        role: "user",
        content: "新问题",
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
        id: `assistant-${turnHead}`,
        turnId: turnHead,
        role: "assistant",
        content: "新回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ],
    historyNodes: [
      {
        nodeId: nodeOld,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: null,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnOld,
        title: "旧 checkpoint",
        summary: "旧",
        createdAtMs: 1000,
        updatedAtMs: 1000
      },
      {
        nodeId: nodeHead,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: nodeOld,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnHead,
        title: "最新 checkpoint",
        summary: "最新",
        createdAtMs: 2000,
        updatedAtMs: 2000
      }
    ],
    historyBranches: [
      {
        branchId: "branch-main",
        sessionId: "e2e-test-session",
        headNodeId: nodeHead,
        baseNodeId: nodeOld,
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  return { nodeOld, nodeHead, turnOld, turnHead };
}

test.beforeEach(async ({ page }) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.evaluate(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });
  await page.goto("about:blank");
});

test("E2E-CHECKPOINT-001 空工作台下撤回按钮禁用", async ({ page }) => {
  await openHome(page);

  const undoButton = page.getByTestId("workspace-undo-button");
  await expect(undoButton).toBeVisible();
  await expect(undoButton).toBeDisabled();
  await expect(undoButton).toHaveAttribute("title", /没有可撤回的操作/);
});

test("E2E-CHECKPOINT-002 注入历史数据后撤回按钮可用", async ({ page }) => {
  await openHome(page);

  const nodeOld = makeId();
  const nodeHead = makeId();
  const turnOld = makeTurnId();
  const turnHead = makeTurnId();

  await patchStore(page, {
    sessionId: "e2e-test-session",
    sessionOperation: null,
    phase: "ready",
    error: null,
    isSubmitting: false,
    activeBranchId: "branch-main",
    visibleNodeId: nodeHead,
    branchHeadNodeId: nodeHead,
    historyCursorMode: "live",
    messages: [
      {
        id: `user-${turnOld}`,
        turnId: turnOld,
        role: "user",
        content: "旧问题",
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
        id: `assistant-${turnOld}`,
        turnId: turnOld,
        role: "assistant",
        content: "旧回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      },
      {
        id: `user-${turnHead}`,
        turnId: turnHead,
        role: "user",
        content: "新问题",
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
        id: `assistant-${turnHead}`,
        turnId: turnHead,
        role: "assistant",
        content: "新回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ],
    historyNodes: [
      {
        nodeId: nodeOld,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: null,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnOld,
        title: "旧 checkpoint",
        summary: "旧",
        createdAtMs: 1000,
        updatedAtMs: 1000
      },
      {
        nodeId: nodeHead,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: nodeOld,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnHead,
        title: "最新 checkpoint",
        summary: "最新",
        createdAtMs: 2000,
        updatedAtMs: 2000
      }
    ],
    historyBranches: [
      {
        branchId: "branch-main",
        sessionId: "e2e-test-session",
        headNodeId: nodeHead,
        baseNodeId: nodeOld,
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  const undoButton = page.getByTestId("workspace-undo-button");
  await expect(undoButton).toBeEnabled();

  await expect(page.getByText("旧问题")).toBeVisible();
  await expect(page.getByText("新回答")).toBeVisible();
});

test("E2E-CHECKPOINT-003 撤回按钮在有草稿时禁用", async ({ page }) => {
  await openHome(page);

  const nodeId = makeId();
  const turnId = makeTurnId();

  await patchStore(page, {
    sessionId: "e2e-test-session",
    sessionOperation: null,
    phase: "ready",
    error: null,
    isSubmitting: false,
    draftMessage: "我还没发完的草稿",
    activeBranchId: "branch-main",
    visibleNodeId: nodeId,
    branchHeadNodeId: nodeId,
    historyCursorMode: "live",
    messages: [
      {
        id: `user-old`,
        turnId: "turn-old",
        role: "user",
        content: "旧问题",
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
        id: `assistant-old`,
        turnId: "turn-old",
        role: "assistant",
        content: "旧回答",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: "browser-preview/mock-stream",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ],
    historyNodes: [
      {
        nodeId,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: null,
        workspaceRef: { kind: "none", rollbackCapable: false },
        turnId: "turn-old",
        title: "旧 checkpoint",
        summary: "旧 checkpoint",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ],
    historyBranches: [
      {
        branchId: "branch-main",
        sessionId: "e2e-test-session",
        headNodeId: nodeId,
        baseNodeId: nodeId,
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  const undoButton = page.getByTestId("workspace-undo-button");
  await expect(undoButton).toBeDisabled();
  await expect(undoButton).toHaveAttribute("title", /请先处理当前草稿/);
});

test("E2E-CHECKPOINT-004 用户消息下方的撤回按钮在有历史节点时出现", async ({ page }) => {
  await openHome(page);

  await seedCheckpointSession(page);

  await page.waitForTimeout(100);

  const actionBars = page.getByTestId("workspace-user-checkpoint-actions");
  await expect(actionBars).toHaveCount(2);

  const firstBarButtons = actionBars.first().locator("button");
  await expect(firstBarButtons.first()).toBeVisible();
  await expect(firstBarButtons.first()).toHaveAttribute("title", /仅撤回对话/);
});

for (const [label, buttonIndex] of [["仅撤回对话", 0], ["撤回对话和修改", 1]] as const) {
  test(`E2E-CHECKPOINT-005 首轮${label}可回到初始空状态`, async ({ page }) => {
    await openHome(page);
    await seedCheckpointSession(page);
    await page.waitForTimeout(100);

    const firstActionBar = page.getByTestId("workspace-user-checkpoint-actions").first();
    const targetButton = firstActionBar.locator("button").nth(buttonIndex);
    await targetButton.click();

    const confirmText = buttonIndex === 0 ? "确认仅撤回对话？" : "确认撤回对话和文件？";
    const popover = page.locator('[data-side="top"]').filter({ hasText: confirmText }).last();
    await expect(popover).toBeVisible();
    await popover.locator("button").click();

    await expect(page.getByTestId("workspace-empty-state")).toBeVisible();
    await expect(page.getByText("旧问题")).toHaveCount(0);
    await expect(page.getByText("旧回答")).toHaveCount(0);
    await expect(page.getByText("新问题")).toHaveCount(0);
    await expect(page.getByText("新回答")).toHaveCount(0);
    await expect(page.getByTestId("workspace-composer-input")).toHaveValue("");

    await expect.poll(() => readStore(page, "draftMessage")).toBe("");
    await expect.poll(() => readStore(page, "messages")).toEqual([]);
  });

  test(`E2E-CHECKPOINT-006 首轮${label}在无显式root时也可回到初始空状态`, async ({ page }) => {
    await openHome(page);
    await seedCheckpointSessionWithoutExplicitRoot(page);
    await page.waitForTimeout(100);

    const firstActionBar = page.getByTestId("workspace-user-checkpoint-actions").first();
    const targetButton = firstActionBar.locator("button").nth(buttonIndex);
    await targetButton.click();

    const confirmText = buttonIndex === 0 ? "确认仅撤回对话？" : "确认撤回对话和文件？";
    const popover = page.locator('[data-side="top"]').filter({ hasText: confirmText }).last();
    await expect(popover).toBeVisible();
    await popover.locator("button").click();

    await expect(page.getByTestId("workspace-empty-state")).toBeVisible();
    await expect(page.getByText("旧问题")).toHaveCount(0);
    await expect(page.getByText("旧回答")).toHaveCount(0);
    await expect(page.getByText("新问题")).toHaveCount(0);
    await expect(page.getByText("新回答")).toHaveCount(0);
    await expect(page.getByTestId("workspace-composer-input")).toHaveValue("");

    await expect.poll(() => readStore(page, "draftMessage")).toBe("");
    await expect.poll(() => readStore(page, "messages")).toEqual([]);
  });
}



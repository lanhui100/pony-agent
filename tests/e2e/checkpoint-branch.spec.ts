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

test.beforeEach(async ({ page }) => {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.evaluate(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });
  await page.goto("about:blank");
});

test("E2E-CHECKPOINT-001 空工作台下撤回按钮禁用且分支提示正确", async ({ page }) => {
  await openHome(page);

  const undoButton = page.getByTestId("workspace-undo-button");
  await expect(undoButton).toBeVisible();
  await expect(undoButton).toBeDisabled();
  await expect(undoButton).toHaveAttribute("title", /没有可撤回的操作/);

  const branchTrigger = page.getByTestId("workspace-branch-switcher-trigger");
  await expect(branchTrigger).toBeDisabled();
  await expect(branchTrigger).toHaveAttribute("title", /当前还没有分支/);
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

  const branchTrigger = page.getByTestId("workspace-branch-switcher-trigger");
  await expect(branchTrigger).toBeEnabled();
  await expect(branchTrigger).toHaveAttribute("title", /切换对话分支/);

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

  await page.waitForTimeout(100);

  const actionBars = page.getByTestId("workspace-user-checkpoint-actions");
  await expect(actionBars).toHaveCount(2);

  const firstBarButtons = actionBars.first().locator("button");
  await expect(firstBarButtons.first()).toBeVisible();
  await expect(firstBarButtons.first()).toHaveAttribute("title", /仅撤回对话/);
});

test("E2E-BRANCH-001 分支切换器显示已有分支", async ({ page }) => {
  await openHome(page);

  await patchStore(page, {
    sessionId: "e2e-test-session",
    sessionOperation: null,
    phase: "ready",
    error: null,
    isSubmitting: false,
    activeBranchId: "branch-main",
    visibleNodeId: "node-head",
    branchHeadNodeId: "node-head",
    historyCursorMode: "live",
    historyBranches: [
      {
        branchId: "branch-main",
        sessionId: "e2e-test-session",
        headNodeId: "node-head",
        baseNodeId: "node-root",
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      },
      {
        branchId: "branch-alt",
        sessionId: "e2e-test-session",
        headNodeId: "node-alt-head",
        baseNodeId: "node-root",
        forkedFromNodeId: "node-old",
        label: "alt",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  await page.waitForTimeout(50);

  const trigger = page.getByTestId("workspace-branch-switcher-trigger");
  await expect(trigger).toBeEnabled();

  await trigger.click();
  await page.waitForTimeout(50);

  const menu = page.getByTestId("workspace-branch-switcher-menu");
  await expect(menu).toBeVisible();
  await expect(menu).toContainText("main");
  await expect(menu).toContainText("alt");
});

test("E2E-BRANCH-002 提交中禁切分支", async ({ page }) => {
  await openHome(page);

  await patchStore(page, {
    sessionId: "e2e-test-session",
    sessionOperation: null,
    phase: "calling_model",
    error: null,
    isSubmitting: true,
    activeBranchId: "branch-main",
    visibleNodeId: "node-head",
    branchHeadNodeId: "node-head",
    historyCursorMode: "live",
    historyBranches: [
      {
        branchId: "branch-main",
        sessionId: "e2e-test-session",
        headNodeId: "node-head",
        baseNodeId: "node-root",
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      },
      {
        branchId: "branch-alt",
        sessionId: "e2e-test-session",
        headNodeId: "node-alt-head",
        baseNodeId: "node-root",
        forkedFromNodeId: "node-old",
        label: "alt",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  await page.waitForTimeout(50);

  const trigger = page.getByTestId("workspace-branch-switcher-trigger");
  await expect(trigger).toBeDisabled();
});

test("E2E-BRANCH-003 agent 分支按钮展示", async ({ page }) => {
  await openHome(page);

  const nodeSource = makeId();
  const nodeHead = makeId();
  const turnSource = makeTurnId();
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
        id: `user-${turnSource}`,
        turnId: turnSource,
        role: "user",
        content: "源问题",
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
        id: `assistant-${turnSource}`,
        turnId: turnSource,
        role: "assistant",
        content: "源回答",
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
        nodeId: nodeSource,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: null,
        workspaceRef: { kind: "host_snapshot", rollbackCapable: true },
        turnId: turnSource,
        title: "源 checkpoint",
        summary: "源",
        createdAtMs: 1000,
        updatedAtMs: 1000
      },
      {
        nodeId: nodeHead,
        sessionId: "e2e-test-session",
        branchId: "branch-main",
        parentNodeId: nodeSource,
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
        baseNodeId: nodeSource,
        label: "main",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      },
      {
        branchId: "branch-fork",
        sessionId: "e2e-test-session",
        headNodeId: nodeHead,
        baseNodeId: nodeSource,
        forkedFromNodeId: nodeSource,
        label: "fork-1",
        createdAtMs: Date.now(),
        updatedAtMs: Date.now()
      }
    ]
  });

  await page.waitForTimeout(100);

  const agentActions = page.getByTestId("workspace-agent-branch-actions").first();
  await expect(agentActions).toBeVisible();

  await expect(agentActions.getByTitle("创建分支")).toBeVisible();
  await expect(agentActions.getByTitle("查看和切换分支")).toBeVisible();
});

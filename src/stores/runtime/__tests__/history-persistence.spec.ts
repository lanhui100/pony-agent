/**
 * 场景 4（task-1）· 历史与持久化：
 * `persistHistory` 落盘 payload 形状、`checkoutHistoryNode` / `switchHistoryBranch` 游标事务、
 * `initializeSessions` 的 local-cache 冷启动恢复。
 *
 * 冻结判据：
 * - `persistHistory` 的 payload 是**逐字段**判据（含 `runningTurnId` 只写不读、`turnTraceHistory`
 *   明确不落盘、`canonicalTerminalPhase` 条件写、运行态映射 buffer 截断）；
 * - 历史操作的 messageDelta 必须推进 `messageRevision`（CAS 基准），否则后续 delta 全被丢弃。
 */
import { describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

vi.mock("@/lib/runtime/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/utils")>();
  return {
    ...actual,
    waitForNextPaint: () => Promise.resolve(),
    runLowPriorityTurnWork: (callback: () => void) => callback()
  };
});

import { RUNTIME_STORAGE_KEY } from "@/lib/runtime/constants";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import type { AttachmentAsset, ChatMessage, ExecutionCheckpoint, HistoryBranch, HistoryNode } from "@/types/runtime";
import {
  BASE_TIME,
  drainMicrotasks,
  expectGolden,
  persistedSnapshot,
  readPersistedCache,
  runtimeSnapshot,
  seedStore,
  setupRuntimeHarness
} from "./harness";

const harness = setupRuntimeHarness(tauriMocks);

const checkpoint: ExecutionCheckpoint = {
  contractVersion: "1.0",
  turnId: "turn-9",
  sessionId: "rt-persist",
  runId: "run-persist",
  checkpointKind: "runtime_control",
  resumable: true,
  replayable: false,
  status: "completed",
  phase: "completed",
  providerName: "OpenAI",
  completedHops: 2,
  maxHops: 8,
  traceSteps: [],
  toolActivities: [],
  startedAtMs: BASE_TIME - 9_000,
  updatedAtMs: BASE_TIME - 1_000
};

const attachmentAsset: AttachmentAsset = {
  id: "asset-1",
  sessionId: "rt-persist",
  name: "report.txt",
  mimeType: "text/plain",
  relativePath: "attachments/report.txt",
  sizeBytes: 42,
  createdAtMs: BASE_TIME - 3_000,
  status: "active",
  referenceCount: 1
};

function message(partial: Partial<ChatMessage> & Pick<ChatMessage, "id" | "turnId" | "role" | "content">): ChatMessage {
  return {
    attachments: [],
    status: "done",
    tokenCount: null,
    ...partial
  };
}

const usersMessage = message({ id: "m-1", turnId: "turn-1", role: "user", content: "第一问", tokenCount: 30 });
const assistantMessage = message({ id: "m-2", turnId: "turn-1", role: "assistant", content: "第一答", tokenCount: 15 });

const nodeOne: HistoryNode = {
  nodeId: "node-1",
  sessionId: "rt-persist",
  parentNodeId: null,
  branchId: "branch-1",
  kind: "turn_committed",
  turnId: "turn-1",
  summary: "第一轮",
  createdAtMs: BASE_TIME - 8_000
};

const nodeTwo: HistoryNode = {
  nodeId: "node-2",
  sessionId: "rt-persist",
  parentNodeId: "node-1",
  branchId: "branch-1",
  kind: "turn_committed",
  turnId: "turn-2",
  summary: "第二轮",
  createdAtMs: BASE_TIME - 7_000
};

const branchOne: HistoryBranch = {
  branchId: "branch-1",
  sessionId: "rt-persist",
  baseNodeId: "node-1",
  headNodeId: "node-2",
  label: "主线",
  createdAtMs: BASE_TIME - 8_000,
  updatedAtMs: BASE_TIME - 7_000
};

describe("场景 4 · 历史与持久化 golden master", () => {
  it("persistHistory 落盘 payload 形状逐字冻结（含条件字段与运行态映射截断）", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionId: "rt-persist",
      phase: "completed",
      messages: [usersMessage, assistantMessage],
      attachmentAssets: [attachmentAsset],
      sessionSummary: "持久化摘要",
      providerRequestedName: "OpenAI",
      providerName: "OpenAI",
      providerProtocol: "openai",
      providerModel: "gpt-5",
      providerSource: "primary",
      providerMode: "standard",
      fallbackReason: "fallback-persist",
      inputTokens: 30,
      outputTokens: 15,
      totalTokens: 45,
      firstTokenLatencyMs: 120,
      visibleNodeId: "node-2",
      branchHeadNodeId: "node-2",
      activeBranchId: "branch-1",
      cursorVersion: 9,
      historyCursorMode: "live",
      historyNodes: [nodeOne, nodeTwo],
      historyBranches: [branchOne],
      initialRollbackActive: false,
      latestExecutionCheckpoint: checkpoint,
      activeTurnId: "turn-running-9",
      sessionWorkspaceId: "ws-frozen",
      runningSessionMap: {
        "rt-bg": {
          turnId: "turn-bg",
          phase: "calling_model",
          textBuffer: "x".repeat(5_200),
          reasoningBuffer: "r".repeat(2_100)
        }
      },
      completedSessionSet: { "rt-done": true },
      failedSessionSet: { "rt-fail": true }
    });
    // turnTraceHistory 明确不落盘（由后端权威持有）——先造一条证明其被排除。
    store.upsertTurnTrace("turn-1", { phase: "completed", traceId: "trace-persist" }, false);

    store.persistHistory();

    const persisted = persistedSnapshot("rt-persist");
    const payload = persisted.payload as Record<string, any>;
    expect(payload.cachedStateVersion).toBe(2);
    expect(payload.phase).toBe("completed");
    expect(payload.canonicalTerminalPhase).toBe("completed");
    expect(payload.runningTurnId).toBe("turn-running-9");
    expect(payload.sessionWorkspaceId).toBe("ws-frozen");
    expect(payload.cursorVersion).toBe(9);
    expect(payload.checkpoint).toEqual(checkpoint);
    // 只写不读的键：既有行为，Wave-1 不清理。
    expect(Object.keys(payload)).toContain("runningTurnId");
    // trace 数据不落盘（只落在后端）。
    expect(Object.keys(payload)).not.toContain("turnTraceHistory");
    // 运行态映射 buffer 截断：textBuffer 5000 / reasoningBuffer 2000（尾部保留）。
    const runningMap = persisted.runningSessionMap as Record<string, any>;
    expect(runningMap["rt-bg"].textBuffer).toHaveLength(5_000);
    expect(runningMap["rt-bg"].reasoningBuffer).toHaveLength(2_000);
    expect(runningMap["rt-bg"].textBuffer.endsWith("x")).toBe(true);
    expect(persisted.completedSet).toEqual({ "rt-done": true });
    expect(persisted.failedSet).toEqual({ "rt-fail": true });

    await expectGolden("persist-history-payload", persisted);
  });

  it("persistHistory 空消息门禁：无消息且非初始撤回 ⇒ 删除会话键；initialRollbackActive ⇒ 强制落盘", () => {
    const store = harness.getStore();
    // 先造一份旧 payload，证明空消息路径会**删除**而不是保留旧值。
    store.$patch({ sessionId: "rt-empty", phase: "ready", messages: [usersMessage] });
    store.persistHistory();
    expect(readPersistedCache()?.sessions).toHaveProperty("rt-empty");

    store.$patch({ messages: [], phase: "idle", initialRollbackActive: false });
    store.persistHistory();
    expect((readPersistedCache()?.sessions as Record<string, unknown>)["rt-empty"]).toBeUndefined();

    // 初始撤回态：即使消息为空也必须落盘（崩溃恢复需要记住撤回视图）。
    store.$patch({ initialRollbackActive: true });
    store.persistHistory();
    expect((readPersistedCache()?.sessions as Record<string, any>)["rt-empty"].initialRollbackActive).toBe(true);
  });

  it("checkoutHistoryNode 应用 messageDelta：截断消息、推进 revision、写入历史游标并落盘", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    seedStore(store, {
      sessionId: "rt-checkout",
      messages: [
        message({ id: "m-1", turnId: "turn-1", role: "user", content: "第一问" }),
        message({ id: "m-2", turnId: "turn-1", role: "assistant", content: "第一答" }),
        message({ id: "m-3", turnId: "turn-2", role: "user", content: "第二问" }),
        message({ id: "m-4", turnId: "turn-2", role: "assistant", content: "第二答" })
      ],
      messageRevision: null,
      historyNodes: [
        { ...nodeOne, sessionId: "rt-checkout" },
        { ...nodeTwo, sessionId: "rt-checkout", turnId: "turn-2" }
      ],
      historyBranches: [{ ...branchOne, sessionId: "rt-checkout", headNodeId: "node-3" }],
      activeBranchId: "branch-1",
      cursorVersion: 10,
      phase: "ready"
    });

    backend.routeValue("checkout_history_node", {
      sessionId: "rt-checkout",
      nodeId: "node-2",
      cursor: {
        sessionId: "rt-checkout",
        visibleNodeId: "node-2",
        activeBranchId: "branch-1",
        branchHeadNodeId: "node-3",
        workspaceNodeId: "node-2",
        mode: "historical",
        cursorVersion: 11,
        isAtBranchHead: false
      },
      requestedMode: "transcript_only",
      appliedMode: "transcript_only",
      transcriptRestoreApplied: true,
      workspaceRollbackCapable: false,
      workspaceRollbackApplied: false,
      degraded: false,
      degradationReason: null,
      historyStateEvidence: null,
      historyStateAuditSummary: {
        action: { kind: "checkout", nodeId: "node-2" },
        currentContext: { visibleNodeId: "node-2" }
      },
      messageDelta: {
        sessionId: "rt-checkout",
        baseRevision: "rev-0",
        targetRevision: "rev-1",
        ops: [{ kind: "truncateAfter", messageId: "m-2" }]
      }
    });

    const result = await store.checkoutHistoryNode("node-2", "transcript_only");
    await drainMicrotasks();

    expect(result).not.toBeNull();
    expect(store.sessionError).toBeNull();
    expect(store.messages.map((item) => item.id)).toEqual(["m-1", "m-2"]);
    // CAS 基准必须推进，否则随后所有 delta 都会被 baseRevision 校验丢弃。
    expect(store.messageRevision).toBe("rev-1");
    expect(store.visibleNodeId).toBe("node-2");
    expect(store.branchHeadNodeId).toBe("node-3");
    expect(store.activeBranchId).toBe("branch-1");
    // 既有实现：checkout 结果不携带 cursorVersion ⇒ 游标版本被清空（不是保留旧值）。
    expect(store.cursorVersion).toBeNull();
    expect(store.historyCursorMode).toBe("historical");
    expect(store.isHistoricalMode).toBe(true);
    expect(store.initialRollbackActive).toBe(false);
    expect(store.latestHistoryStateAuditSummary).toEqual({
      action: { kind: "checkout", nodeId: "node-2" },
      currentContext: { visibleNodeId: "node-2" }
    });
    const persisted = persistedSnapshot("rt-checkout").payload as Record<string, any>;
    expect(persisted.visibleNodeId).toBe("node-2");
    // 依据 src/lib/runtime/history.ts:188-215 normalizeHistoryCheckoutResult 不返回 cursorVersion
    // ⇒ 落盘为 null（Wave-1 实测，勿改回 11）。
    expect(persisted.cursorVersion).toBeNull();
    expect(persisted.messages.map((item: ChatMessage) => item.id)).toEqual(["m-1", "m-2"]);

    await expectGolden("history-checkout-node", runtimeSnapshot(store));
  });

  it("switchHistoryBranch：成功推进 branch 游标并应用 delta；宿主失败⇒sessionError 且不触碰状态", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    seedStore(store, {
      sessionId: "rt-branch",
      messages: [
        message({ id: "m-1", turnId: "turn-1", role: "user", content: "第一问" }),
        message({ id: "m-2", turnId: "turn-1", role: "assistant", content: "第一答" })
      ],
      messageRevision: "rev-a",
      historyNodes: [
        { ...nodeOne, sessionId: "rt-branch" },
        { ...nodeTwo, sessionId: "rt-branch" },
        { ...nodeTwo, sessionId: "rt-branch", nodeId: "node-3", turnId: "turn-2", summary: "branch-2 头" }
      ],
      historyBranches: [
        { ...branchOne, sessionId: "rt-branch", branchId: "branch-1", headNodeId: "node-1" },
        { ...branchOne, sessionId: "rt-branch", branchId: "branch-2", baseNodeId: "node-1", headNodeId: "node-3" }
      ],
      activeBranchId: "branch-1",
      phase: "ready"
    });

    backend.routeValue("switch_history_branch", {
      sessionId: "rt-branch",
      branchId: "branch-2",
      nodeId: "node-3",
      cursor: {
        sessionId: "rt-branch",
        visibleNodeId: "node-3",
        activeBranchId: "branch-2",
        branchHeadNodeId: "node-3",
        workspaceNodeId: "node-3",
        mode: "live",
        cursorVersion: 12,
        isAtBranchHead: true
      },
      historyStateEvidence: null,
      historyStateAuditSummary: {
        action: { kind: "switch_branch", branchId: "branch-2" },
        currentContext: { visibleNodeId: "node-3" }
      },
      messageDelta: {
        sessionId: "rt-branch",
        baseRevision: "rev-a",
        targetRevision: "rev-b",
        ops: [
          {
            kind: "append",
            messages: [{ messageId: "m-3", turnId: "turn-3", role: "user", content: "追加问题", status: "done" }]
          }
        ]
      }
    });

    const result = await store.switchHistoryBranch("branch-2");
    await drainMicrotasks();

    expect(result).not.toBeNull();
    expect(store.messages.map((item) => item.id)).toEqual(["m-1", "m-2", "m-3"]);
    expect(store.messageRevision).toBe("rev-b");
    expect(store.activeBranchId).toBe("branch-2");
    expect(store.branchHeadNodeId).toBe("node-3");
    expect(store.visibleNodeId).toBe("node-3");
    // 既有实现：branch-switch 结果不携带 cursorVersion ⇒ 游标版本被清空（不是保留旧值）。
    expect(store.cursorVersion).toBeNull();
    expect(store.historyCursorMode).toBe("live");
    expect(store.sessionError).toBeNull();

    await expectGolden("history-switch-branch", runtimeSnapshot(store));

    // 宿主失败：performHistoryOperation 的 catch 收敛为 sessionError，历史与消息保持不变。
    const beforeMessages = JSON.parse(JSON.stringify(store.messages));
    const beforeHistory = runtimeSnapshot(store).d6_history;
    backend.route("switch_history_branch", () => {
      throw new Error("backend boom");
    });
    const failed = await store.switchHistoryBranch("branch-1");
    expect(failed).toBeNull();
    expect(store.sessionError).toContain("切换 branch失败");
    expect(store.agentRuntimeStatus?.mode).toBe("error");
    expect(runtimeSnapshot(store).d6_history).toEqual(beforeHistory);
    expect(JSON.parse(JSON.stringify(store.messages))).toEqual(beforeMessages);
  });

  it("initializeSessions 冷启动走 local-cache：消息/历史游标/用量/运行态映射全量恢复", async () => {
    const store = harness.getStore();
    const backend = harness.backend();
    const initSessionId = "rt-init";
    const persistedPayload = {
      cachedStateVersion: 2,
      phase: "completed",
      canonicalTerminalPhase: "completed",
      messages: [
        { id: "i-u-1", turnId: "turn-i-1", role: "user", content: "冷启动问题", attachments: [], status: "done", tokenCount: 50 },
        {
          id: "i-a-1",
          turnId: "turn-i-1",
          role: "assistant",
          content: "冷启动回答",
          status: "done",
          tokenCount: 25,
          modelName: "OpenAI/gpt-5"
        }
      ],
      attachmentAssets: [],
      sessionSummary: "冷启动摘要",
      providerRequestedName: "OpenAI",
      providerName: "OpenAI",
      providerProtocol: "openai",
      providerModel: "gpt-5",
      providerSource: "primary",
      providerMode: "standard",
      fallbackReason: null,
      inputTokens: 50,
      outputTokens: 25,
      totalTokens: 75,
      firstTokenLatencyMs: 150,
      visibleNodeId: "node-1",
      branchHeadNodeId: "node-1",
      activeBranchId: "branch-1",
      historyCursorMode: "live",
      historyNodes: [{ ...nodeOne, sessionId: initSessionId }],
      historyBranches: [{ ...branchOne, sessionId: initSessionId, headNodeId: "node-1" }],
      cursorVersion: 4,
      initialRollbackActive: false,
      checkpoint: { ...checkpoint, sessionId: initSessionId },
      runningTurnId: null,
      sessionWorkspaceId: DEFAULT_WORKSPACE_ID
    };
    window.localStorage.setItem(
      RUNTIME_STORAGE_KEY,
      JSON.stringify({
        sessions: { [initSessionId]: persistedPayload },
        runningSessionMap: {
          "rt-bg2": { turnId: "turn-bg2", phase: "calling_model", textBuffer: "bg2", reasoningBuffer: "" }
        },
        completedSet: { "rt-done": true },
        failedSet: {}
      })
    );

    backend.routeValue("list_sessions", [
      {
        conversationId: initSessionId,
        title: "冷启动会话",
        summary: "冷启动摘要",
        turnCount: 1,
        lastReferencedFile: null,
        updatedAtMs: BASE_TIME - 1_000
      }
    ]);
    // local-cache 的宿主刷新失败必须被 contained（降级为缓存视图），不得冒泡成 sessionError。
    backend.route("load_session_runtime_view", () => {
      throw new Error("host offline");
    });

    await store.initializeSessions();
    await drainMicrotasks();

    expect(store.sessionId).toBe(initSessionId);
    expect(store.sessionError).toBeNull();
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionHydrating).toBe(false);
    expect(store.messages.map((item) => item.content)).toEqual(["冷启动问题", "冷启动回答"]);
    expect(store.phase).toBe("completed");
    expect(store.isSubmitting).toBe(false);
    expect(store.sessionSummary).toBe("冷启动摘要");
    expect(store.providerName).toBe("OpenAI");
    expect(store.inputTokens).toBe(50);
    expect(store.outputTokens).toBe(25);
    expect(store.totalTokens).toBe(75);
    expect(store.visibleNodeId).toBe("node-1");
    expect(store.cursorVersion).toBe(4);
    // 既有实现：buildRuntimeViewFromPersistedState 返回 checkpoint:null ⇒ 冷启动不重建 checkpoint 明细。
    expect(store.latestExecutionCheckpoint).toBeNull();
    // 既有现象（=代码事实，非本波修复目标）：`loadPersistedRuntimeCache()` 只返回 `{ sessions }`，
    // 把 `runningSessionMap` / `completedSet` / `failedSet` 丢弃 ⇒ initializeSessions 的这三处恢复
    // 在真实链路上永远拿到空映射（写侧 persistSessionStateAndRuntimeMaps 仍会写）。
    // 已种入上述三键，逐字冻结「恢复为空」的现状。
    expect(store.runningSessionMap).toEqual({});
    expect(store.completedSessionSet).toEqual({});
    expect(store.failedSessionSet).toEqual({});

    await expectGolden("initialize-sessions-restore", runtimeSnapshot(store));
  });
});

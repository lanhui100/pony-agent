/**
 * 场景 3（task-1）· 会话切换与重置：`switchSession` / `applySessionSnapshot` / `resetSessionRuntimeState`
 *
 * 冻结判据：跨 D1/D3/D4/D5/D6 五域的清空或恢复必须与拆解前逐字一致（golden fixtures）。
 * 特别注意「重置的清空面」与「保留面」（队列 = D7、工作区树 = D2 不属于会话运行时状态，
 * `resetSessionRuntimeState` 既有行为就**不清**它们），由 fixtures 逐字锁定。
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
import type { ChatMessage, ExecutionCheckpoint, HistoryBranch, HistoryNode, SessionSnapshot } from "@/types/runtime";
import { BASE_TIME, drainMicrotasks, expectGolden, runtimeSnapshot, seedStore, SESSION_ID, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);

function writePersistedCache(cache: Record<string, unknown>): void {
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

const userMessage: ChatMessage = {
  id: "seed-user-1",
  turnId: "turn-seed-1",
  role: "user",
  content: "种子用户消息",
  attachments: [],
  status: "done",
  tokenCount: 64
};

const assistantMessage: ChatMessage = {
  id: "seed-assistant-1",
  turnId: "turn-seed-1",
  role: "assistant",
  content: "种子助手消息",
  status: "done",
  tokenCount: 32,
  reasoningContent: "种子推理",
  modelName: "OpenAI/gpt-5"
};

const historyNodeA: HistoryNode = {
  nodeId: "node-1",
  sessionId: SESSION_ID,
  parentNodeId: null,
  branchId: "branch-1",
  kind: "turn_committed",
  turnId: "turn-seed-1",
  summary: "第一轮提交",
  createdAtMs: BASE_TIME - 2_000
};

const historyNodeB: HistoryNode = {
  nodeId: "node-2",
  sessionId: SESSION_ID,
  parentNodeId: "node-1",
  branchId: "branch-1",
  kind: "turn_committed",
  turnId: "turn-seed-2",
  summary: "第二轮提交",
  createdAtMs: BASE_TIME - 1_000
};

const historyBranch: HistoryBranch = {
  branchId: "branch-1",
  sessionId: SESSION_ID,
  baseNodeId: "node-1",
  headNodeId: "node-2",
  label: "主线",
  createdAtMs: BASE_TIME - 2_000,
  updatedAtMs: BASE_TIME - 1_000
};

const executionCheckpoint: ExecutionCheckpoint = {
  contractVersion: "1.0",
  turnId: "turn-seed-2",
  sessionId: SESSION_ID,
  runId: "run-seed",
  checkpointKind: "runtime_control",
  resumable: true,
  replayable: false,
  status: "running",
  phase: "calling_model",
  completedHops: 1,
  maxHops: 8,
  traceSteps: [],
  toolActivities: [],
  startedAtMs: BASE_TIME - 1_000,
  updatedAtMs: BASE_TIME - 500
};

describe("场景 3 · 会话切换与重置 golden master", () => {
  it("resetSessionRuntimeState 清空 D1/D3/D4/D5/D6 运行时态但保留队列/工作区（既有行为）", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionId: SESSION_ID,
      phase: "failed",
      error: "旧错误",
      sessionNotice: "旧提示",
      draftMessage: "旧草稿",
      sessionSummary: "旧摘要",
      providerRequestedName: "OpenAI",
      providerName: "OpenAI",
      providerProtocol: "openai",
      providerModel: "gpt-5",
      providerSource: "primary",
      providerMode: "standard",
      fallbackReason: "fallback-x",
      inputTokens: 100,
      outputTokens: 20,
      totalTokens: 120,
      firstTokenLatencyMs: 250,
      liveMetrics: { step: 0 } as never,
      activeProviderCallRecords: [
        {
          requestKind: "initial_request",
          providerSource: "provider_decision",
          providerMode: "live",
          inputTokens: 100,
          cacheHitInputTokens: 0,
          cacheMissInputTokens: 100,
          reasoningTokens: 0,
          outputTokens: 20,
          totalTokens: 120,
          firstTokenLatencyMs: 250,
          turnDurationMs: 900,
          latencyKind: "provider_stream",
          prefixMutationReasons: []
        }
      ],
      isSubmitting: true,
      activeTurnId: "turn-seed-2",
      activeTurnStartedAtMs: BASE_TIME,
      activeRunId: "run-seed",
      latestGraphRunSubmissionPlan: { command: "resume_graph_run_stream", runId: "run-seed" },
      latestGraphRunControlBoundaryEvidence: [{ boundaryId: "b-1" } as never],
      latestHistoryStateAuditSummary: { status: "ok" } as never,
      latestRunControlAuditSummary: { status: "ok" } as never,
      visibleNodeId: "node-1",
      branchHeadNodeId: "node-2",
      activeBranchId: "branch-1",
      cursorVersion: 7,
      messageRevision: "rev-7",
      historyCursorMode: "historical",
      historyNodes: [historyNodeA, historyNodeB],
      historyBranches: [historyBranch],
      initialRollbackActive: true,
      messages: [userMessage, assistantMessage],
      toolActivities: [
        {
          id: "call-seed",
          name: "workspace.read_file",
          status: "done",
          description: "读文件",
          resultText: "ok"
        }
      ],
      traceSteps: store.traceSteps.map((step) => ({ ...step, state: "error" as const })),
      traceTimeline: store.traceTimeline.map((entry) => ({ ...entry, text: "旧 timeline" })),
      streamBufferTurnId: "turn-seed-2",
      streamBufferText: "缓冲正文",
      streamBufferReasoning: "缓冲推理",
      browserPreviewRunToken: 3,
      pendingQueuedMessages: [
        { id: "queued-keep", sessionId: SESSION_ID, content: "保留的排队消息", mode: "queue", createdAt: BASE_TIME }
      ],
      queueDeliveryMode: "steer",
      toolAuthorizationMode: "read_only",
      sessionList: [{ conversationId: SESSION_ID, title: "会话", summary: "s", turnCount: 2, lastReferencedFile: null, updatedAtMs: BASE_TIME }]
    });

    store.resetSessionRuntimeState();
    await drainMicrotasks();

    // 清空面（跨域抽样）
    expect(store.phase).toBe("idle");
    expect(store.error).toBeNull();
    expect(store.sessionNotice).toBeNull();
    expect(store.draftMessage).toBe("");
    expect(store.messages).toEqual([]);
    expect(store.traceSteps.length).toBeGreaterThan(0);
    expect(store.traceTimeline.length).toBeGreaterThan(0);
    expect(store.turnTraceHistory).toEqual([]);
    expect(store.liveMetrics).toBeNull();
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();
    expect(store.visibleNodeId).toBeNull();
    expect(store.branchHeadNodeId).toBeNull();
    expect(store.historyCursorMode).toBe("live");
    expect(store.messageRevision).toBeNull();
    // 保留面（既有行为，不得在拆解中顺手改）
    expect(store.pendingQueuedMessages).toHaveLength(1);
    expect(store.queueDeliveryMode).toBe("steer");
    expect(store.toolAuthorizationMode).toBe("read_only");
    expect(store.sessionId).toBe(SESSION_ID);

    await expectGolden("session-reset-state", runtimeSnapshot(store));
  });

  it("switchSession 命中本地缓存：恢复消息/历史/用量，并按目标会话后台轮重建流式态", async () => {
    const store = harness.getStore();
    const targetSessionId = "rt-target-cached";
    writePersistedCache({
      sessions: {
        [targetSessionId]: {
          cachedStateVersion: 2,
          phase: "completed",
          canonicalTerminalPhase: "completed",
          messages: [
            { id: "m-u-1", turnId: "turn-t-1", role: "user", content: "目标会话问题", attachments: [], status: "done", tokenCount: 88 },
            {
              id: "m-a-1",
              turnId: "turn-t-1",
              role: "assistant",
              content: "目标会话回答",
              status: "done",
              tokenCount: 44,
              modelName: "OpenAI/gpt-5"
            }
          ],
          attachmentAssets: [],
          sessionSummary: "目标会话摘要",
          providerRequestedName: "OpenAI",
          providerName: "OpenAI",
          providerProtocol: "openai",
          providerModel: "gpt-5",
          providerSource: "primary",
          providerMode: "standard",
          fallbackReason: null,
          inputTokens: 88,
          outputTokens: 44,
          totalTokens: 132,
          firstTokenLatencyMs: 180,
          visibleNodeId: "node-1",
          branchHeadNodeId: "node-1",
          activeBranchId: "branch-1",
          historyCursorMode: "live",
          historyNodes: [{ ...historyNodeA, sessionId: targetSessionId, summary: "目标会话节点" }],
          historyBranches: [{ ...historyBranch, sessionId: targetSessionId }],
          cursorVersion: 3,
          initialRollbackActive: false,
          checkpoint: null,
          runningTurnId: null,
          sessionWorkspaceId: DEFAULT_WORKSPACE_ID
        }
      },
      runningSessionMap: {
        [targetSessionId]: {
          turnId: "turn-bg",
          phase: "calling_model",
          textBuffer: "后台缓冲",
          reasoningBuffer: "后台推理"
        }
      },
      completedSet: { "rt-other": true },
      failedSet: { "rt-other": false }
    });

    seedStore(store, {
      sessionId: SESSION_ID,
      phase: "ready",
      messages: [userMessage, assistantMessage],
      sessionSummary: "旧会话摘要",
      // switchSession 读取的是 store 内的 runningSessionMap（initializeSessions 才从缓存恢复该映射）。
      runningSessionMap: {
        [targetSessionId]: {
          turnId: "turn-bg",
          phase: "calling_model",
          textBuffer: "后台缓冲",
          reasoningBuffer: "后台推理"
        }
      }
    });

    // 关闭宿主：走纯本地缓存恢复路径（无 host-view 二次覆盖，行为确定）。
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    await store.switchSession(targetSessionId);
    await drainMicrotasks(30);

    expect(store.sessionId).toBe(targetSessionId);
    expect(store.sessionSwitchToken).toBe(1);
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionHydrating).toBe(false);
    expect(store.sessionError).toBeNull();
    expect(store.messages.map((message) => message.content)).toEqual(["目标会话问题", "目标会话回答"]);
    expect(store.sessionSummary).toBe("目标会话摘要");
    expect(store.inputTokens).toBe(88);
    expect(store.outputTokens).toBe(44);
    expect(store.historyNodes).toHaveLength(1);
    expect(store.visibleNodeId).toBe("node-1");
    expect(store.cursorVersion).toBe(3);
    // 后台运行轮恢复：解锁态按目标会话的缓冲重建。
    expect(store.activeTurnId).toBe("turn-bg");
    expect(store.isSubmitting).toBe(true);
    expect(store.phase).toBe("calling_model");
    expect(store.streamBufferText).toBe("后台缓冲");
    expect(store.streamBufferReasoning).toBe("后台推理");
    expect(store.activeStreamKind).toBe("content");
    expect(store.runningSessionMap).toEqual({});
    expect(store.agentRuntimeStatus?.mode).toBe("responding");

    await expectGolden("session-switch-cached-background-turn", runtimeSnapshot(store));
  });

  it("switchSession 未命中缓存：整域重置后以空快照落点（phase/消息/检索上下文均为恢复值）", async () => {
    const store = harness.getStore();
    const targetSessionId = "rt-target-empty";
    seedStore(store, {
      sessionId: SESSION_ID,
      phase: "failed",
      messages: [userMessage, assistantMessage],
      sessionSummary: "旧会话摘要",
      visibleNodeId: "node-1",
      historyCursorMode: "historical"
    });

    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    await store.switchSession(targetSessionId);
    await drainMicrotasks(30);

    expect(store.sessionId).toBe(targetSessionId);
    expect(store.messages).toEqual([]);
    expect(store.sessionSummary).toBe("");
    // 空快照恢复：restorePhaseFromTurnHistory([], []) 的既有落点为 idle（不是 switch 期间的 connecting）。
    expect(store.phase).toBe("idle");
    expect(store.visibleNodeId).toBeNull();
    expect(store.historyCursorMode).toBe("live");
    expect(store.sessionOperation).toBeNull();
    expect(store.sessionHydrating).toBe(false);
    expect(store.retrievedContext).not.toBeNull();

    await expectGolden("session-switch-uncached", runtimeSnapshot(store));
  });

  it("applySessionSnapshot 恢复历史游标 + messageState 快照（revision 与运行计划一并落库）", async () => {
    const store = harness.getStore();
    const snapshot = {
      conversationId: "rt-hist",
      title: "历史会话",
      summary: "历史会话摘要",
      history: [
        { role: "user", content: "历史问题", attachments: [], turnId: "turn-h-1", status: "done" },
        {
          role: "assistant",
          content: "历史回答",
          attachments: [],
          turnId: "turn-h-1",
          status: "done",
          modelName: "OpenAI/gpt-5"
        }
      ],
      attachmentAssets: [],
      turnTraceHistory: [],
      turnCount: 1,
      lastReferencedFile: null,
      updatedAtMs: BASE_TIME - 5_000
    } as unknown as SessionSnapshot;

    const runtimeView = {
      messageState: {
        sessionId: "rt-hist",
        revision: "rev-9",
        messages: [
          { messageId: "ms-u-1", turnId: "turn-h-1", role: "user", content: "快照问题", status: "done", tokenCount: 12 },
          {
            messageId: "ms-a-1",
            turnId: "turn-h-1",
            role: "assistant",
            content: "快照回答",
            status: "done",
            tokenCount: 6,
            modelName: "OpenAI/gpt-5"
          }
        ]
      },
      historyNodes: [
        { ...historyNodeA, sessionId: "rt-hist", nodeId: "h-node-1", summary: "历史节点一" },
        { ...historyNodeB, sessionId: "rt-hist", nodeId: "h-node-2", parentNodeId: "h-node-1", summary: "历史节点二" }
      ],
      historyBranches: [{ ...historyBranch, sessionId: "rt-hist", branchId: "h-branch-1", baseNodeId: "h-node-1", headNodeId: "h-node-2" }],
      historyCursor: {
        sessionId: "rt-hist",
        visibleNodeId: "h-node-1",
        activeBranchId: "h-branch-1",
        branchHeadNodeId: "h-node-2",
        workspaceNodeId: "h-node-1",
        mode: "historical",
        cursorVersion: 7,
        isAtBranchHead: false
      },
      submissionPlan: { command: "resume_graph_run_stream", runId: "run-hist", source: "checkpoint" },
      controlBoundaryEvidence: [{ boundaryId: "evidence-1" }],
      runControlAuditSummary: {
        actionEvidenceSummary: { action: "resume_graph_run_stream", runId: "run-hist" },
        currentContextProjection: { projection: "historical" }
      },
      historyStateAuditSummary: {
        action: { kind: "snapshot_restore", nodeId: "h-node-1" },
        currentContext: { visibleNodeId: "h-node-1" }
      }
    } as unknown as Parameters<typeof store.applySessionSnapshot>[3];

    store.applySessionSnapshot("rt-hist", snapshot, null, runtimeView);
    await drainMicrotasks();

    expect(store.sessionId).toBe("rt-hist");
    expect(store.messageRevision).toBe("rev-9");
    expect(store.messages.map((message) => message.content)).toEqual(["快照问题", "快照回答"]);
    expect(store.historyCursorMode).toBe("historical");
    expect(store.isHistoricalMode).toBe(true);
    expect(store.visibleNodeId).toBe("h-node-1");
    expect(store.branchHeadNodeId).toBe("h-node-2");
    expect(store.activeBranchId).toBe("h-branch-1");
    expect(store.cursorVersion).toBe(7);
    expect(store.activeRunId).toBe("run-hist");
    // 既有实现：applySessionSnapshot 只把 submissionPlan 的 runId 落到 activeRunId；
    // latestGraphRunSubmissionPlan 由外层 loadSessionState 回填（本用例直调 action ⇒ 保持 null）。
    expect(store.latestGraphRunSubmissionPlan).toBeNull();
    expect(store.latestRunControlAuditSummary).toEqual({
      actionEvidenceSummary: { action: "resume_graph_run_stream", runId: "run-hist" },
      currentContextProjection: { projection: "historical" }
    });
    expect(store.latestHistoryStateAuditSummary).toEqual({
      action: { kind: "snapshot_restore", nodeId: "h-node-1" },
      currentContext: { visibleNodeId: "h-node-1" }
    });
    const persisted = runtimeSnapshot(store).persistence as Record<string, any>;
    expect(persisted.payload.visibleNodeId).toBe("h-node-1");
    expect(persisted.payload.historyCursorMode).toBe("historical");

    await expectGolden("session-apply-snapshot-historical", runtimeSnapshot(store));
  });

  it("resetSessionRuntimeState 后 latestExecutionCheckpoint 保留（既有行为登记）", async () => {
    // 既有实现只清 latestGraphRun* / latestHistoryStateAuditSummary，**不清** latestExecutionCheckpoint。
    // 该不对称为 Wave-1 等价判据的一部分：拆解时不得顺手统一。
    const store = harness.getStore();
    seedStore(store, { sessionId: SESSION_ID, latestExecutionCheckpoint: executionCheckpoint });
    store.resetSessionRuntimeState();
    expect(store.latestExecutionCheckpoint).toEqual(executionCheckpoint);
  });
});

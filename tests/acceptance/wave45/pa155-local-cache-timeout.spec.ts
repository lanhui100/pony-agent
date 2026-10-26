/**
 * Wave-45 契约验收（L2-AT）— PA-155：`local-cache` 初始化策略的宿主机刷新必须**有界**
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave45.md`（PA155-T1/T2）。
 *
 * 缺陷事实（修复前，`src/stores/runtime.ts` `initializeSessions()` 的 `switch` 内）：
 *   - `case "host-read"` 已经过 PA-115 加固：`withTimeout(hostRead, HYDRATION_TIMEOUT_MS, "加载对话")`；
 *   - `case "local-cache"` 却是 `await this.loadSessionRuntimeViewState(...)` 裸 await ——
 *     宿主 IPC 挂起（静默丢包）时该 await **永不 settle** ⇒ `initializeSessions()` 永久 pending，
 *     `finally` 永不执行、缓存消息永远不回填：用户看到空白会话，而不是「至少有本地缓存」。
 *   语义上 `local-cache` 策略存在的意义正是「缓存可用 ⇒ 先落地缓存」，宿主刷新只是增强项，
 *   因此它挂起时**必须**回落到缓存（保留缓存并正常进入会话），而不是把整个启动拖死。
 *
 * 覆盖（契约断言 id 对照）：
 *   PA155-T1（正向 + 有界性主断言）
 *     - 种入「可触发 local-cache 策略」的持久化缓存（checkpoint 非空 + phase 非 idle）；
 *     - 宿主 `load_session_runtime_view` 永不 resolve；
 *     - `HYDRATION_TIMEOUT_MS + 1000ms` 内 `initializeSessions()` 必须 settle（resolve，不得 reject）；
 *     - 缓存消息仍然可见（`sessionId` 落到缓存会话、消息内容等于缓存快照）；
 *     - `sessionError` 为空或 warn 级（**不得**出现硬失败通道 `初始化对话失败：` + status=error），
 *       即「保留缓存并正常进入会话」；
 *     - 不永久 pending：`sessionOperation === null`。
 *   PA155-T2（L2-AT 对抗：迟到 resolve）
 *     - 宿主读在 baseline 超时 + 5s 后才返回**与缓存不同**的历史；
 *     - 仍须在有界时间内收敛，且迟到结果**不得**改写 `sessionId` / `messages` / `phase`；
 *     - 不得对同一逻辑读做重试风暴（`load_session_runtime_view` 仅 1 次）。
 *
 * 另含**常量漂移锁**：`expect(HYDRATION_TIMEOUT_MS).toBe(15000)`——超时基线被改动即红。
 *
 * 确定性：`vi.useFakeTimers()` + `vi.advanceTimersByTimeAsync()`（同 wave44/pa115 惯例），
 * 全程无真实 sleep、无壁钟断言；`beforeEach/afterEach` 清理 localStorage（谁创建谁清理）。
 *
 * 零假测试律：无 skip/todo；每条用例以具体状态断言收尾；不 import 私有函数、不断言内部变量名。
 *
 * 作者：test-agent-45（L2-AT）。红相锚定后本文件冻结只读，任何修改须 Lead 批准。
 */
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  RetrievedContextState,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot
} from "@/types/runtime";
import { CACHED_STATE_VERSION, HYDRATION_TIMEOUT_MS, RUNTIME_STORAGE_KEY } from "@/lib/runtime/constants";
import { deriveInitStrategy, loadPersistedRuntimeState } from "@/lib/runtime/sessions";
import { useRuntimeStore } from "@/stores/runtime";

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

/** 缓存会话 id（catalog 与持久化缓存必须一致，否则 deriveInitStrategy 落不到 local-cache）。 */
const CACHED_SESSION_ID = "wave45-local-cache-session";
/** 契约 §2.1 NFR：settle_deadline_ms = HYDRATION_TIMEOUT_MS + 1000。 */
const SETTLE_DEADLINE_MS = HYDRATION_TIMEOUT_MS + 1_000;
/** 对抗用例的迟到 resolve 时刻（baseline + 5s）。 */
const LATE_RESOLVE_AT_MS = HYDRATION_TIMEOUT_MS + 5_000;

const CACHED_USER_TEXT = "缓存历史：冷启动必须仍然可见的问题";
const CACHED_ASSISTANT_TEXT = "缓存历史：冷启动必须仍然可见的回答";
const LATE_USER_TEXT = "迟到历史：不得覆盖缓存的问题";

// ── 夹具（形状对齐 tests/runtime-store.spec.ts / wave44 pa115 已验证的 host view）────

function createSnapshot(partial: Partial<SessionSnapshot> = {}): SessionSnapshot {
  return {
    conversationId: partial.conversationId ?? CACHED_SESSION_ID,
    title: partial.title ?? "wave45 local-cache",
    summary: partial.summary ?? "缓存会话摘要",
    history: partial.history ?? [],
    attachmentAssets: [],
    turnTraceHistory: [],
    turnCount: partial.turnCount ?? 1,
    lastReferencedFile: null,
    updatedAtMs: partial.updatedAtMs ?? 1_700_000_000_000
  };
}

function createRetrievedContext(snapshot: SessionSnapshot): RetrievedContextState {
  const lastUserMessage =
    [...snapshot.history].reverse().find((message) => message.role === "user")?.content ?? "";

  return {
    turnContext: { userMessage: lastUserMessage, images: [], referencesImage: false },
    sessionContext: {
      conversationId: snapshot.conversationId,
      title: snapshot.title ?? "Session",
      summary: snapshot.summary,
      recentHistory: snapshot.history.map((message) => ({ ...message })),
      recentAttachmentAssets: [],
      turnCount: snapshot.turnCount,
      lastReferencedFile: null
    },
    runState: {},
    longTermMemory: { status: "empty", summary: "no long-term memory", entries: [] },
    transcript: { providerNativeMessages: [] }
  } as unknown as RetrievedContextState;
}

function createSessionRuntimeView(snapshot: SessionSnapshot): SessionRuntimeView {
  return {
    session: snapshot,
    retrieved: createRetrievedContext(snapshot),
    checkpoint: null,
    submissionPlan: null,
    controlBoundaryEvidence: null,
    historyStateAuditSummary: null,
    runControlAuditSummary: null,
    historyNodes: [],
    historyBranches: [],
    historyCursor: null,
    authorityMode: "host_authoritative",
    resolvedVisibleNodeId: null,
    activeBranchHeadNodeId: null,
    isAtBranchHead: false,
    cursorVersion: null
  } as unknown as SessionRuntimeView;
}

function sessionsCatalog(): SessionOverview[] {
  return [
    {
      conversationId: CACHED_SESSION_ID,
      title: "wave45 local-cache",
      summary: "缓存会话",
      turnCount: 1,
      lastReferencedFile: null,
      updatedAtMs: 1_700_000_000_000,
      workspaceId: null
    }
  ] as unknown as SessionOverview[];
}

/**
 * 种入**可触发 `local-cache` 策略**的持久化缓存。
 *
 * `deriveInitStrategy` 判据：`persisted != null && checkpoint != null && phase !== "idle"` ⇒ local-cache。
 * 因此 checkpoint 必须非空、phase 必须非 idle——否则用例会静默退化到 `host-read` 分支，
 * 让「local-cache 有界性」这条契约变成假绿。
 */
function seedLocalCache(): void {
  const persisted = {
    cachedStateVersion: CACHED_STATE_VERSION,
    phase: "completed",
    messages: [
      {
        id: "cached-msg-user",
        turnId: "turn-cached-1",
        role: "user",
        content: CACHED_USER_TEXT,
        status: "done"
      },
      {
        id: "cached-msg-assistant",
        turnId: "turn-cached-1",
        role: "assistant",
        content: CACHED_ASSISTANT_TEXT,
        status: "done"
      }
    ],
    attachmentAssets: [],
    sessionSummary: "缓存会话摘要",
    providerRequestedName: "cached-provider",
    providerName: "cached-provider",
    providerProtocol: "openai",
    providerModel: "cached-model",
    providerSource: "settings",
    providerMode: "api",
    fallbackReason: null,
    inputTokens: 10,
    outputTokens: 20,
    totalTokens: 30,
    firstTokenLatencyMs: 120,
    checkpoint: {
      turnId: "turn-cached-1",
      sessionId: CACHED_SESSION_ID,
      checkpointKind: "runtime_control",
      resumable: true,
      replayable: false,
      status: "completed",
      phase: "completed",
      completedHops: 1,
      maxHops: 1,
      traceSteps: [],
      toolActivities: []
    }
  };

  window.localStorage.setItem(
    RUNTIME_STORAGE_KEY,
    JSON.stringify({ sessions: { [CACHED_SESSION_ID]: persisted } })
  );
}

/**
 * 防假绿守卫：种入的缓存必须真的命中 `local-cache` 策略。
 * 若夹具退化（checkpoint 为空 / phase 回落 idle），用例会静默走到 `host-read` 分支，
 * 「local-cache 有界性」这条契约就变成对另一段代码的假绿断言。
 */
function expectLocalCacheStrategy() {
  const persisted = loadPersistedRuntimeState(CACHED_SESSION_ID);
  expect(persisted, "种入的持久化缓存必须可被 loadPersistedRuntimeState 读回").not.toBeNull();
  const strategy = deriveInitStrategy(CACHED_SESSION_ID, persisted, sessionsCatalog());
  expect(strategy.kind).toBe("local-cache");
}

/** 迟到宿主视图（PA155-T2 对抗：不得被回填）。 */
function lateHostView(): SessionRuntimeView {
  return createSessionRuntimeView(
    createSnapshot({
      history: [
        { role: "user", content: LATE_USER_TEXT },
        { role: "assistant", content: "迟到历史：不得覆盖缓存的回答" }
      ]
    })
  );
}

type HostReadMode = "hang" | "resolve-late";

function installHostMocks(mode: HostReadMode): { hostReadCalls: number } {
  const counter = { hostReadCalls: 0 };
  tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
    if (command === "list_sessions") {
      return Promise.resolve(sessionsCatalog());
    }
    if (command === "load_session_runtime_view") {
      counter.hostReadCalls += 1;
      if (mode === "resolve-late") {
        const view = lateHostView();
        return new Promise((resolve) => {
          setTimeout(() => resolve(view), LATE_RESOLVE_AT_MS);
        });
      }
      // IPC 静默丢包：永不 settle。
      return new Promise<never>(() => {});
    }
    return Promise.resolve(null);
  });
  return counter;
}

async function flushMicrotasks(times = 16): Promise<void> {
  for (let index = 0; index < times; index += 1) {
    await Promise.resolve();
  }
}

/** 记录 initializeSessions 的结算状态（resolve / reject），不依赖任何 sleep。 */
function trackSettlement(promise: Promise<unknown>) {
  const state: { settled: boolean; outcome: "resolved" | "rejected" | null } = {
    settled: false,
    outcome: null
  };
  void promise.then(
    () => {
      state.settled = true;
      state.outcome = "resolved";
    },
    () => {
      state.settled = true;
      state.outcome = "rejected";
    }
  );
  return state;
}

/** 推进 fake timers 到指定时刻并冲刷微任务，返回是否已收敛。 */
async function advanceTo(settlement: { settled: boolean }, ms: number): Promise<boolean> {
  await vi.advanceTimersByTimeAsync(ms);
  await flushMicrotasks();
  return settlement.settled;
}

function messageContents(store: ReturnType<typeof useRuntimeStore>): string[] {
  return store.messages.map((message) => message.content);
}

/**
 * 契约 PA155-T1 的「warn 级」判据：
 * 不得走硬失败通道（`初始化对话失败：` + `agentRuntimeStatus.mode === "error"`）。
 * 允许为 null（静默降级），也允许为 warn 文本（只要不把会话判成不可用）。
 */
function expectNoHardInitFailure(store: ReturnType<typeof useRuntimeStore>) {
  expect(String(store.sessionError ?? "")).not.toMatch(/^初始化对话失败：/);
  if (store.sessionError == null) {
    expect(store.agentRuntimeStatus?.mode).not.toBe("error");
  }
}

describe("PA-155 · local-cache 初始化策略的宿主刷新有界性", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  afterEach(() => {
    vi.useRealTimers();
    window.localStorage.clear();
  });

  it("PA155-T1: 缓存可用但宿主刷新永不 settle → 有界 settle、缓存消息仍可见、无硬失败", async () => {
    // Arrange：超时基线常量漂移锁（契约 NFR hydration_read_timeout_ms=15000）。
    expect(HYDRATION_TIMEOUT_MS).toBe(15000);
    seedLocalCache();
    expectLocalCacheStrategy();
    const counter = installHostMocks("hang");
    const store = useRuntimeStore();

    // Act 1：发起初始化并冲刷微任务，确认宿主读确实被等待（不得同步返回）。
    const settlement = trackSettlement(store.initializeSessions());
    await flushMicrotasks();
    // 有界性的下界锚点：宿主读挂起时初始化在 1000ms 时**不可能**已收敛
    //（若这里已 settled，说明本用例根本没走到挂起的宿主刷新路径，红相归因会失真）。
    const settledBy1s = await advanceTo(settlement, 1_000);
    expect(settledBy1s).toBe(false);
    expect(store.sessionOperation).toBe("initializing");

    // Act 2：推进到契约结算截止期。
    const settledWithinDeadline = await advanceTo(settlement, SETTLE_DEADLINE_MS - 1_000);

    // Assert 1（红相主断言）：必须有界收敛，且为 resolve（启动流程不得向调用方抛）。
    expect(settledWithinDeadline).toBe(true);
    expect(settlement.outcome).toBe("resolved");

    // Assert 2：缓存必须仍然可见（local-cache 策略的立身之本）。
    expect(store.sessionId).toBe(CACHED_SESSION_ID);
    expect(messageContents(store)).toContain(CACHED_USER_TEXT);
    expect(messageContents(store)).toContain(CACHED_ASSISTANT_TEXT);
    expect(store.messages.length).toBeGreaterThanOrEqual(2);

    // Assert 3（Gate-1 A #6 收紧）：phase 不得为任何错误/终态失败相位
    //（「保留缓存并正常进入会话」⇒ 不得把会话判成 error / failed / cancelled）。
    expect(store.phase).not.toBe("error");
    expect(store.phase).not.toBe("failed");
    expect(store.phase).not.toBe("cancelled");

    // Assert 4：保留缓存并正常进入会话 ⇒ 不得有硬失败通道（null 或 warn 级）。
    expectNoHardInitFailure(store);

    // Assert 5：不永久 pending——初始化锁必须已释放。
    expect(store.sessionOperation).toBeNull();

    // Assert 6：不得对同一逻辑读做重试风暴。
    expect(counter.hostReadCalls).toBe(1);
  });

  it("PA155-T2 (L2-AT 对抗): 宿主读在 baseline 超时 + 5s 后才 resolve → 仍须有界收敛，且迟到结果不得改写状态", async () => {
    // Arrange：超时基线常量漂移锁。
    expect(HYDRATION_TIMEOUT_MS).toBe(15000);
    seedLocalCache();
    expectLocalCacheStrategy();
    const counter = installHostMocks("resolve-late");
    const store = useRuntimeStore();

    // Act 1：推进到结算截止期（此时迟到结果尚未到达）。
    const settlement = trackSettlement(store.initializeSessions());
    await flushMicrotasks();
    const settledWithinDeadline = await advanceTo(settlement, SETTLE_DEADLINE_MS);

    // Assert 1：必须在迟到结果到达之前收敛（截止期严格早于迟到时刻）。
    expect(SETTLE_DEADLINE_MS).toBeLessThan(LATE_RESOLVE_AT_MS);
    expect(settledWithinDeadline).toBe(true);
    expect(settlement.outcome).toBe("resolved");

    // Assert 2：缓存可见 + 无硬失败（与 T1 同口径）。
    expect(messageContents(store)).toContain(CACHED_USER_TEXT);
    expectNoHardInitFailure(store);
    expect(store.sessionOperation).toBeNull();

    // 记录迟到结果到达前的快照。
    const snapshotBeforeLate = {
      sessionId: store.sessionId,
      phase: store.phase,
      messageContents: messageContents(store)
    };
    expect(snapshotBeforeLate.messageContents).not.toContain(LATE_USER_TEXT);

    // Act 2：让迟到宿主结果真正到达（补齐到 LATE_RESOLVE_AT_MS + 100ms 余量）。
    await vi.advanceTimersByTimeAsync(LATE_RESOLVE_AT_MS - SETTLE_DEADLINE_MS + 100);
    await flushMicrotasks(24);

    // Assert 3：迟到 resolve 不得改写会话身份 / 阶段 / 消息（无迟到写）。
    expect(store.sessionId).toBe(snapshotBeforeLate.sessionId);
    expect(store.phase).toBe(snapshotBeforeLate.phase);
    expect(messageContents(store)).toEqual(snapshotBeforeLate.messageContents);
    expect(messageContents(store)).not.toContain(LATE_USER_TEXT);

    // Assert 4：缓存仍是唯一可见真相，且初始化锁保持释放（不得二次进城）。
    expect(messageContents(store)).toContain(CACHED_USER_TEXT);
    expect(store.sessionOperation).toBeNull();

    // Assert 5：不得因迟到结果触发重试风暴。
    expect(counter.hostReadCalls).toBe(1);
  });
});

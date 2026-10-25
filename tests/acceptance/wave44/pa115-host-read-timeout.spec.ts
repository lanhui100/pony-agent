/**
 * Wave-44 契约验收（L2-AT）— PA-115：initializeSessions() 的 host-read 必须**有界超时**
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave44.md` §1 PA115-T1/T2、§2.1、§3。
 *
 * 缺陷事实（A1/A6/A7/A8）：
 *   `src/stores/runtime.ts:2694` `case "host-read"` 直接 `await this.loadSessionRuntimeViewState(...)`，
 *   无超时；对照同文件 `:2141` 已有 `withTimeout(loadPromise, HYDRATION_TIMEOUT_MS, "加载对话")`。
 *   Tauri IPC 全链路无超时（A4）⇒ 宿主读挂起时 `initializeSessions()` 永不 settle；
 *   `:2639` 的 3500ms watchdog 只清 `sessionOperation`、不取消 pending await（A6），
 *   外层 catch 的失败通道（A5）永远到不了 ⇒ 启动遮罩常驻（A7）、状态栏无任何错误（A8）。
 *
 * 覆盖（契约断言 id 对照）：
 *   PA115-T1（L2-T，覆盖契约 PA115-T2 主体 + PA115-T1 正常路径）
 *     - host 读**永不 settle** → ≤ HYDRATION_TIMEOUT_MS+1000ms 内 **resolve**（不得抛、不得永久 pending）；
 *     - 可见失败：`sessionError` 以 `初始化对话失败：` 开头且含 `超时`；
 *       `agentRuntimeStatus === { mode:"error", label===sessionError.trim() }` 且 ≠「正在加载最近对话…」；
 *     - 状态复位：`sessionOperation === null`、`phase` 回到调用前快照；
 *     - 恢复：随后把宿主 mock 换成正常返回，再次 `initializeSessions()` ⇒ 正常路径不回归
 *       （sessionId/消息内容/`sessionError===null`/`sessionOperation===null`/status≠error，
 *        且 `load_session_runtime_view` 在恢复调用中恰好 1 次）。
 *   PA115-T2（L2-AT 对抗，契约 PA115-T2 的迟到写分支 + Gate-1 审查项「草稿保留」）
 *     - 宿主读在 baseline 超时 + 5s 后才 resolve → 初始化仍必须在有界时间内收敛；
 *     - **草稿保留**：3500ms watchdog 触发（`sessionOperation` 解锁 = 输入区可用）后用户输入的草稿，
 *       在 15000ms 超时整包快照回滚时**不得被静默吞掉**（快照含 `draftMessage`，
 *       直接 `restoreSessionRuntimeSnapshot` 会回滚掉它）；
 *     - 迟到 resolve **不得**改写 `sessionId` / `messages` / `phase`（无迟到写）；
 *     - 不得对同一逻辑读做重试风暴（`load_session_runtime_view` 仅 1 次）。
 *
 * 另含**常量漂移锁**：`expect(HYDRATION_TIMEOUT_MS).toBe(15000)`——超时基线被改动即红。
 *
 * 确定性：`vi.useFakeTimers()` + `vi.advanceTimersByTimeAsync()`（同 `tests/runtime-store.spec.ts:2316` 惯例），
 * 全程无真实 sleep、无壁钟断言。
 *
 * 零假测试律：无 skip/todo；每条用例以具体状态断言收尾；不 import 私有函数、不断言内部变量名。
 *
 * 作者：test-agent（L2-AT）。红相锚定后本文件冻结只读，任何修改须 Lead 批准。
 */
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  RetrievedContextState,
  SessionOverview,
  SessionRuntimeView,
  SessionSnapshot
} from "@/types/runtime";
import { HYDRATION_TIMEOUT_MS } from "@/lib/runtime/constants";
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

const HOST_SESSION_ID = "wave44-host-read-session";
/** 契约 §2.1 NFR：settle_deadline_ms = HYDRATION_TIMEOUT_MS + 1000。 */
const SETTLE_DEADLINE_MS = HYDRATION_TIMEOUT_MS + 1_000;
/** 对抗用例的迟到 resolve 时刻（baseline + 5s）。 */
const LATE_RESOLVE_AT_MS = HYDRATION_TIMEOUT_MS + 5_000;

const HOST_USER_TEXT = "宿主历史：正常加载的问题";
const HOST_ASSISTANT_TEXT = "宿主历史：正常加载的回答";
const LATE_USER_TEXT = "迟到历史：不得被回填的问题";
/** 3500ms watchdog 解锁输入后、15000ms 超时降级前用户输入的草稿（Gate-1 审查项）。 */
const DRAFT_TYPED_WHILE_WAITING = "等待期间输入";

// ── 夹具（形状对齐 tests/runtime-store.spec.ts 已验证的 host view）─────────

function createSnapshot(partial: Partial<SessionSnapshot> = {}): SessionSnapshot {
  return {
    conversationId: partial.conversationId ?? HOST_SESSION_ID,
    title: partial.title ?? "wave44 host-read",
    summary: partial.summary ?? "宿主会话摘要",
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
      conversationId: HOST_SESSION_ID,
      title: "wave44 host-read",
      summary: "宿主读取场景",
      turnCount: 1,
      lastReferencedFile: null,
      updatedAtMs: 1_700_000_000_000,
      workspaceId: null
    }
  ] as unknown as SessionOverview[];
}

/** 正常宿主视图（用于 PA115-T1 的恢复阶段）。 */
function normalHostView(): SessionRuntimeView {
  return createSessionRuntimeView(
    createSnapshot({
      history: [
        { role: "user", content: HOST_USER_TEXT },
        { role: "assistant", content: HOST_ASSISTANT_TEXT }
      ]
    })
  );
}

/** 迟到宿主视图（PA115-T2 对抗：不得被回填）。 */
function lateHostView(): SessionRuntimeView {
  return createSessionRuntimeView(
    createSnapshot({
      history: [
        { role: "user", content: LATE_USER_TEXT },
        { role: "assistant", content: "迟到历史：不得被回填的回答" }
      ]
    })
  );
}

type HostReadMode = "hang" | "normal" | "resolve-late";

function installHostMocks(mode: HostReadMode): { hostReadCalls: number } {
  const counter = { hostReadCalls: 0 };
  tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
    if (command === "list_sessions") {
      return Promise.resolve(sessionsCatalog());
    }
    if (command === "load_session_runtime_view") {
      counter.hostReadCalls += 1;
      if (mode === "normal") {
        return Promise.resolve(normalHostView());
      }
      if (mode === "resolve-late") {
        const view = lateHostView();
        return new Promise((resolve) => {
          setTimeout(() => resolve(view), LATE_RESOLVE_AT_MS);
        });
      }
      return new Promise<never>(() => {});
    }
    return Promise.resolve(null);
  });
  return counter;
}

async function flushMicrotasks(times = 12): Promise<void> {
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

describe("PA-115 · initializeSessions host-read 有界超时", () => {
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

  it("PA115-T1: 宿主读永不 settle → 有界 resolve + 可见失败 + 状态复位，随后正常宿主读可恢复", async () => {
    // Arrange：超时基线常量漂移锁（契约 A3 / NFR hydration_read_timeout_ms=15000）。
    expect(HYDRATION_TIMEOUT_MS).toBe(15000);
    // Arrange：宿主 bridge 正常，但 load_session_runtime_view 永久挂起（IPC 静默丢包）。
    const hangCounter = installHostMocks("hang");
    const store = useRuntimeStore();
    const phaseBefore = store.phase;

    // Act 1：发起初始化，推进到契约结算截止期。
    const settlement = trackSettlement(store.initializeSessions());
    await flushMicrotasks();
    const settledWithinDeadline = await advanceTo(settlement, SETTLE_DEADLINE_MS);

    // Assert 1（红相主断言）：必须有界收敛，且是 resolve（外层 catch 已吸收）而非 reject。
    expect(settledWithinDeadline).toBe(true);
    expect(settlement.outcome).toBe("resolved");

    // Assert 2：失败必须可见（A5 通道），文案承载「初始化失败 + 超时」语义。
    expect(store.sessionError).toMatch(/^初始化对话失败：/);
    expect(String(store.sessionError)).toContain("超时");

    // Assert 3：状态栏必须暴露 error（A8），且不得停留在「正在加载最近对话…」。
    const status = store.agentRuntimeStatus;
    expect(status).not.toBeNull();
    expect(status?.mode).toBe("error");
    expect(status?.label).toBe(String(store.sessionError).trim());
    expect(status?.label).not.toBe("正在加载最近对话…");

    // Assert 4：状态复位（快照回滚 + finally 清理）。
    expect(store.sessionOperation).toBeNull();
    expect(store.phase).toBe(phaseBefore);

    // Act 2 / Assert 5：正常宿主读可恢复（契约 PA115-T1 正常路径不回归）。
    const normalCounter = installHostMocks("normal");
    const recoverySettlement = trackSettlement(store.initializeSessions());
    await flushMicrotasks();
    expect(await advanceTo(recoverySettlement, SETTLE_DEADLINE_MS)).toBe(true);
    expect(recoverySettlement.outcome).toBe("resolved");
    expect(store.sessionId).toBe(HOST_SESSION_ID);
    expect(store.messages.map((message) => message.content)).toEqual([
      HOST_USER_TEXT,
      HOST_ASSISTANT_TEXT
    ]);
    expect(store.sessionError).toBeNull();
    expect(store.sessionOperation).toBeNull();
    expect(store.agentRuntimeStatus?.mode).not.toBe("error");
    expect(normalCounter.hostReadCalls).toBe(1);
    expect(hangCounter.hostReadCalls).toBe(1);
  });

  it("PA115-T2 (L2-AT 对抗): 宿主读在 baseline 超时 + 5s 后才 resolve → 仍须有界收敛，且迟到结果不得改写状态", async () => {
    // Arrange：超时基线常量漂移锁（契约 A3 / NFR hydration_read_timeout_ms=15000）。
    expect(HYDRATION_TIMEOUT_MS).toBe(15000);
    // Arrange：宿主读永不「及时」返回，只在 baseline 超时之后 +5s 才 resolve。
    const counter = installHostMocks("resolve-late");
    const store = useRuntimeStore();
    const phaseBefore = store.phase;
    const sessionIdBefore = store.sessionId;

    // Act 1：先推进到 3500ms watchdog 触发点（`sessionOperation` 被解锁 ⇒ 输入区可用），
    // 模拟用户在等待期间输入草稿；再推进到结算截止期（此时迟到结果尚未到达）。
    const settlement = trackSettlement(store.initializeSessions());
    await flushMicrotasks();
    await vi.advanceTimersByTimeAsync(4_000);
    await flushMicrotasks();
    expect(store.sessionOperation).toBeNull();
    store.draftMessage = DRAFT_TYPED_WHILE_WAITING;
    // Assert 1a（调用点超时钉死）：推进到 14000ms 时**必须仍未收敛**——
    // 把调用点 `withTimeout(hostRead, <值>, ...)` 的 <值> 机械夹逼在 (14000, 16000) 区间，
    // 防止仅钉住常量层（HYDRATION_TIMEOUT_MS）而调用点被改成 1000 等更小值时测试仍绿。
    const settledBy14s = await advanceTo(settlement, 14_000 - 4_000);
    expect(settledBy14s).toBe(false);
    // Assert 1b：推进到 16000ms 结算截止期必须已收敛。
    const settledWithinDeadline = await advanceTo(settlement, SETTLE_DEADLINE_MS - 14_000);

    // Assert 1：必须在迟到结果到达之前收敛（截止期严格早于迟到时刻）。
    expect(SETTLE_DEADLINE_MS).toBeLessThan(LATE_RESOLVE_AT_MS);
    expect(settledWithinDeadline).toBe(true);
    expect(settlement.outcome).toBe("resolved");

    // Assert 2：超时/降级路径必须显式暴露失败。
    expect(store.sessionError).toMatch(/^初始化对话失败：/);
    expect(String(store.sessionError)).toContain("超时");
    expect(store.sessionOperation).toBeNull();

    // 记录迟到结果到达前的快照。
    const snapshotBeforeLate = {
      sessionId: store.sessionId,
      phase: store.phase,
      messageContents: store.messages.map((message) => message.content)
    };
    expect(snapshotBeforeLate.messageContents).not.toContain(LATE_USER_TEXT);

    // Act 2：让迟到宿主结果真正到达（补齐到 LATE_RESOLVE_AT_MS）。
    await vi.advanceTimersByTimeAsync(LATE_RESOLVE_AT_MS - SETTLE_DEADLINE_MS + 100);
    await flushMicrotasks(20);

    // Assert 3：迟到 resolve 不得改写会话身份 / 阶段 / 消息（无迟到写）。
    expect(store.sessionId).toBe(snapshotBeforeLate.sessionId);
    expect(store.phase).toBe(snapshotBeforeLate.phase);
    expect(store.messages.map((message) => message.content)).toEqual(snapshotBeforeLate.messageContents);
    expect(store.messages.map((message) => message.content)).not.toContain(LATE_USER_TEXT);

    // Assert 4：**草稿保留**——超时降级不得用整包快照回滚吞掉用户在等待期间输入的草稿，
    // 且失败仍必须可见（两者同时成立）。
    expect(store.draftMessage).toBe(DRAFT_TYPED_WHILE_WAITING);
    expect(store.sessionError).toBeTruthy();

    // Assert 5：不得因迟到结果触发重试风暴；初始快照语义保持。
    expect(counter.hostReadCalls).toBe(1);
    expect(store.phase).toBe(phaseBefore);
    expect(store.sessionId).toBe(sessionIdBefore);
  });
});

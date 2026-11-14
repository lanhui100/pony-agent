/**
 * 桌面端上游重试进度（`turn:retry`）→ 状态行「正在重试（n/m）...」的接线验收。
 *
 * 契约真值源（唯一，不可在前端二次推算）：
 *   crates/pony-agent-core/src/agent/retry.rs::RetryNotice
 *     （`attempt` = 已失败尝试数，1 起；`maxAttempts` = 本跳允许尝试总数，含首次）
 *   桌面出口：src-tauri/src/tauri_adapter.rs::TurnRetryPayload（camelCase wire）
 *   前端落点：src/stores/runtime.ts → state `retryProgress` + action `applyTurnRetryProgress`
 *   前端展示：src/lib/runtime/status.ts → `resolveAgentRuntimeStatus({ retryAttempt, retryMax })`
 *
 * 判据：
 * 1. 真实事件走线：`turn:retry` 注册 → 负载落地 → `agentRuntimeStatus.mode === "retrying"`
 *    且 label 含**真实分母** `2/10`（与后端通知同源，禁止虚构）。
 * 2. 归属 fail-closed：sessionId 缺失/非本会话 ⇒ 丢弃（后台会话重试不得污染当前状态行）。
 * 3. 物理不可信负载（分母 < 分子）⇒ 丢弃，不进状态行。
 * 4. 生命周期同点清零：首个真实 delta 到达 ⇒ `retryProgress` 归 null（不得滞留「正在重试」）。
 * 5. 无事件 ⇒ 不展示分母（缺失 retryMax，降级为「正在重试…」判据不适用：连 retryAttempt 都没有）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { ChatMessage, TurnRetryEvent, TurnStreamEvent } from "@/types/runtime";
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

const SESSION_ID = "session-retry";
const TURN_ID = "turn-retry";

type EventHandler = (event: { payload: unknown }) => void;

function createMessage(partial: Partial<ChatMessage>): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? TURN_ID,
    role: partial.role ?? "user",
    content: partial.content ?? "",
    attachments: partial.attachments ?? [],
    status: partial.status ?? "done",
    tokenCount: partial.tokenCount ?? null,
    reasoningContent: partial.reasoningContent ?? null,
    modelName: partial.modelName ?? null,
    toolName: partial.toolName ?? null,
    detail: partial.detail ?? null,
    durationSeconds: partial.durationSeconds ?? null,
    errorDetail: partial.errorDetail ?? null
  };
}

function installEventCapture(): Map<string, EventHandler> {
  const handlers = new Map<string, EventHandler>();
  tauriMocks.mockSafeListen.mockImplementation(async (name: string, handler: unknown) => {
    handlers.set(name, handler as EventHandler);
    return () => {};
  });
  return handlers;
}

function dispatch(handlers: Map<string, EventHandler>, name: string, payload: unknown): void {
  const handler = handlers.get(name);
  if (!handler) {
    throw new Error(`事件未接线：store 必须在 initializeTurnEvents 中注册 "${name}"`);
  }
  handler({ payload });
}

function retryPayload(overrides: Partial<TurnRetryEvent> = {}): TurnRetryEvent {
  return {
    turnId: TURN_ID,
    sessionId: SESSION_ID,
    attempt: 2,
    maxAttempts: 10,
    delayMs: 4000,
    errorClass: "timeout",
    ...overrides
  };
}

/** 造一个「正在流式」的活跃轮次（与 stop-stop-turn 夹具同口径）。 */
function seedStreamingTurn(store: ReturnType<typeof useRuntimeStore>): void {
  store.$patch({
    sessionId: SESSION_ID,
    isSubmitting: true,
    activeTurnId: TURN_ID,
    activeRunId: null,
    phase: "calling_model",
    messages: [
      createMessage({ id: "user-1", turnId: TURN_ID, role: "user", content: "请解释一下" }),
      createMessage({
        id: `assistant-${TURN_ID}`,
        turnId: TURN_ID,
        role: "assistant",
        content: "已有正文",
        status: "streaming"
      })
    ]
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
  localStorage.clear();
  tauriMocks.mockSafeInvoke.mockReset();
  tauriMocks.mockSafeListen.mockReset();
  tauriMocks.mockIsTauriAvailable.mockReset();
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  tauriMocks.mockSafeInvoke.mockImplementation(async () => undefined);
});

describe("桌面端上游重试进度（turn:retry → 状态行）", () => {
  it("retry_event_drives_status_line_with_real_denominator", async () => {
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    dispatch(handlers, "turn:retry", retryPayload());

    // 落地：数值逐字来自后端通知
    expect(store.retryProgress).toEqual({ turnId: TURN_ID, attempt: 2, maxAttempts: 10 });
    // 展示：真实分母 2/10（不是伪造的 1/5，也不是降级文案）
    const status = store.agentRuntimeStatus;
    expect(status?.mode).toBe("retrying");
    expect(status?.label).toBe("正在重试（2/10）...");
    expect(status?.retryAttempt).toBe(2);
    expect(status?.retryMax).toBe(10);
  });

  it("retry_event_from_other_session_is_dropped", async () => {
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    dispatch(handlers, "turn:retry", retryPayload({ sessionId: "session-other" }));

    expect(store.retryProgress).toBeNull();
    expect(store.agentRuntimeStatus?.mode).not.toBe("retrying");
  });

  it("retry_event_without_session_id_is_dropped_fail_closed", async () => {
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    dispatch(handlers, "turn:retry", retryPayload({ sessionId: "" }));

    expect(store.retryProgress).toBeNull();
    expect(store.agentRuntimeStatus?.mode).not.toBe("retrying");
  });

  it("implausible_denominator_is_dropped", async () => {
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    dispatch(handlers, "turn:retry", retryPayload({ attempt: 5, maxAttempts: 3 }));

    expect(store.retryProgress).toBeNull();
    expect(store.agentRuntimeStatus?.mode).not.toBe("retrying");
  });

  it("first_streamed_delta_clears_retry_progress", async () => {
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    dispatch(handlers, "turn:retry", retryPayload());
    expect(store.retryProgress).not.toBeNull();

    const delta: TurnStreamEvent = {
      sessionId: SESSION_ID,
      turnId: TURN_ID,
      kind: "delta",
      phase: "calling_model",
      text: "重试成功后的第一个增量"
    };
    dispatch(handlers, "turn:delta", delta);

    expect(store.retryProgress).toBeNull();
    expect(store.agentRuntimeStatus?.mode).not.toBe("retrying");
  });

  it("no_retry_event_never_shows_a_fabricated_denominator", async () => {
    const store = useRuntimeStore();
    installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    const status = store.agentRuntimeStatus;
    expect(status?.mode).not.toBe("retrying");
    expect(status?.retryAttempt).toBeUndefined();
    expect(status?.retryMax).toBeUndefined();
  });
});

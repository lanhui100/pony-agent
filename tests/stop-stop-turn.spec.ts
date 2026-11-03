// wave-60 C7 黑盒验收（独立测试者，红相先写）：桌面端「可中断停止」的呈现语义。
//
// 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C7。
// 判据：
// 1. 点击停止后**立即**（RPC ack 后，不等终态）进入 `stopping`：`phaseLabel === "正在停止…"`，
//    且状态区（`agentRuntimeStatus`）有值并含「正在停止」；停止按钮禁用外观由 isSubmitting 承载。
// 2. 收到 `turn:cancelled` 且 `stopReason === "user"` ⇒ `sessionNotice === "你已停止本轮 · 发送新消息可继续"`，
//    且**不得**用哨兵句覆盖已流式正文。
// 3. `turn:stop_requested` 到达时若本地尚未 stopping ⇒ 必须补切 stopping（跨窗口/重连路径）。
// 4. `stopping` 必须有兜底退出：终态事件到达后不得永久卡在「正在停止…」。
//
// 纪律：真实断言（不用 Exit 0 代替）、禁止 catch 吞异常 / 空断言 / it.skip。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { ChatMessage, TurnStreamEvent } from "@/types/runtime";
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

/** 被 C7 明令禁止用于覆盖正文的哨兵句（§0 非目标：仍留在落库内容，但不得覆盖实时呈现）。 */
const CANCELLED_TURN_SENTINEL = "用户终止，发送消息可继续。";
/** C7：user 停止的终态状态消息（落 `sessionNotice`，可持久到终态之后）。 */
const STOPPED_BY_USER_NOTICE = "你已停止本轮 · 发送新消息可继续";
const STREAMED_TEXT = "这是已经流式输出出来的正文";

type EventHandler = (event: { payload: unknown }) => void;

function createMessage(partial: Partial<ChatMessage>): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? "turn-stop",
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

/** 造一个「正在流式」的活跃轮次（用户消息 + 已流式助手消息）。 */
function seedStreamingTurn(store: ReturnType<typeof useRuntimeStore>, turnId = "turn-stop") {
  store.$patch({
    sessionId: "session-stop",
    isSubmitting: true,
    activeTurnId: turnId,
    activeRunId: null,
    phase: "calling_model",
    messages: [
      createMessage({ id: "user-1", turnId, role: "user", content: "请解释一下" }),
      createMessage({
        id: "assistant-1",
        turnId,
        role: "assistant",
        content: STREAMED_TEXT,
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
  tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
    if (command === "stop_turn") {
      return { turnId: "turn-stop", accepted: true, state: "stopping", stopReason: "user" };
    }
    throw new Error(`unexpected command: ${command}`);
  });
});

describe("wave-60 C7 桌面端停止呈现", () => {
  it("c7_stop_turn_enters_stopping_immediately_after_ack", async () => {
    // Arrange
    const store = useRuntimeStore();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    // Act
    const accepted = await store.stopTurn();

    // Assert：RPC ack 后**一个微任务内**即进入 stopping，不等终态
    expect(accepted).toBe(true);
    expect(store.phase).toBe("stopping");
    expect(store.phaseLabel).toBe("正在停止…");
    expect(store.isSubmitting).toBe(true);
    expect(store.activeTurnId).toBe("turn-stop");
    // 状态区必须显示「正在停止」（停止按钮禁用外观由 isSubmitting 承载）
    expect(store.agentRuntimeStatus?.label ?? "").toContain("正在停止");
  });

  it("c7_user_cancelled_shows_stopped_notice_and_keeps_streamed_body", async () => {
    // Arrange
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();
    await store.stopTurn();

    // Act：后端终态事件（带结构化 stopReason）
    const cancelledPayload: TurnStreamEvent = {
      sessionId: "session-stop",
      turnId: "turn-stop",
      kind: "cancelled",
      phase: "cancelled",
      text: CANCELLED_TURN_SENTINEL,
      error: "stopped_by_user",
      stopReason: "user"
    };
    handlers.get("turn:cancelled")?.({ payload: cancelledPayload });

    // Assert：终态呈现
    expect(store.phase).toBe("cancelled");
    expect(store.phaseLabel).toBe("已停止");
    expect(store.sessionNotice).toBe(STOPPED_BY_USER_NOTICE);
    expect(store.agentRuntimeStatus?.label ?? "").toContain("你已停止本轮");
    expect(store.isSubmitting).toBe(false);
    expect(store.activeTurnId).toBeNull();

    // Assert：已流式正文不得被哨兵句覆盖
    const assistant = store.messages.find((message) => message.role === "assistant");
    expect(assistant).toBeTruthy();
    expect(assistant?.content).toBe(STREAMED_TEXT);
    expect(assistant?.content).not.toBe(CANCELLED_TURN_SENTINEL);
  });

  it("c7_non_user_stop_reason_shows_plain_stopped_notice", async () => {
    // Arrange
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();
    await store.stopTurn();

    // Act
    handlers.get("turn:cancelled")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "cancelled",
        phase: "cancelled",
        text: CANCELLED_TURN_SENTINEL,
        error: "submission_watchdog_timeout",
        stopReason: "watchdog"
      } satisfies TurnStreamEvent
    });

    // Assert：非 user 原因不得冒充「你已停止」
    expect(store.phase).toBe("cancelled");
    expect(store.sessionNotice ?? "").toContain("已停止");
    expect(store.sessionNotice ?? "").not.toContain("你已停止本轮");
  });

  it("c7_stop_requested_event_backfills_stopping_state", async () => {
    // Arrange：本地尚未进入 stopping（如跨窗口/重连路径）
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();
    expect(store.phase).toBe("calling_model");

    // Act
    handlers.get("turn:stop_requested")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "stop_requested",
        phase: "stopping",
        stopReason: "user",
        emittedAtMs: 1_700_000_000_000
      } satisfies TurnStreamEvent
    });

    // Assert
    expect(store.phase).toBe("stopping");
    expect(store.phaseLabel).toBe("正在停止…");
  });

  it("c7_stopping_state_has_fallback_exit_on_terminal_event", async () => {
    // Arrange：卡在 stopping，等的是兜底退出
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();
    await store.stopTurn();
    expect(store.phase).toBe("stopping");

    // Act：终态以 failed（例如 watchdog 兜底）到达
    handlers.get("turn:failed")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "failed",
        phase: "failed",
        text: "This turn failed.",
        error: "submission_watchdog_timeout"
      } satisfies TurnStreamEvent
    });

    // Assert：不得永久卡在「正在停止…」
    expect(store.phase).toBe("failed");
    expect(store.phaseLabel).not.toBe("正在停止…");
    expect(store.isSubmitting).toBe(false);
  });
});

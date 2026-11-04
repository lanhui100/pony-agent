// wave-64 黑盒验收（独立测试者，红相先写）：F2 终态不得另起消息。
//
// 契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 F2。
// 判据（断言契约目标行为，不是现状）：
// 1. `turn:cancelled` 时**禁止**向助手消息 content 写入 `本轮已停止。` 占位；
//    浏览器预览取消路径（`isTauriAvailable() === false` → `cancelBrowserPreviewTurn`）同规则。
// 2. `turn:failed` 时**禁止**写入 `本轮执行失败，请查看右侧 trace。` 占位正文。
// 3. 失败轮**已产出的正文照常渲染/保留**（不得被占位正文覆盖）。
// 4. 错误明细仍保留在 store 的 `errorDetail`（不新增 UI，但事实不能丢）。
//
// 纪律（test-expert L2）：真实断言、AAA、禁止 skip / catch 吞异常 / 空断言。
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

/** F2 明令禁止的取消占位正文。 */
const CANCELLED_PLACEHOLDER = "本轮已停止。";
/** F2 明令禁止的失败占位正文（与 src/lib/runtime/constants.ts 的 DEFAULT_FAILED_TURN_MESSAGE 同字面）。 */
const FAILED_PLACEHOLDER = "本轮执行失败，请查看右侧 trace。";
const STREAMED_TEXT = "这是已经流式输出出来的正文";
const REAL_ERROR = "tool chain exploded: EACCES on /tmp/secret";

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

function seedStreamingTurn(
  store: ReturnType<typeof useRuntimeStore>,
  assistantContent: string,
  turnId = "turn-stop"
) {
  store.$patch({
    sessionId: "session-stop",
    isSubmitting: true,
    activeTurnId: turnId,
    activeRunId: null,
    phase: "calling_model",
    messages: [
      createMessage({ id: "user-1", turnId, role: "user", content: "请解释一下" }),
      // id 必须是 store 的 ensureAssistantMessage 键（`assistant-${turnId}`），
      // 否则终态事件会作用到**另一条**消息上，断言会假绿。
      createMessage({
        id: `assistant-${turnId}`,
        turnId,
        role: "assistant",
        content: assistantContent,
        status: "streaming"
      })
    ]
  });
}

function assistantOf(store: ReturnType<typeof useRuntimeStore>) {
  return store.messages.find((message) => message.role === "assistant") ?? null;
}

beforeEach(() => {
  setActivePinia(createPinia());
  localStorage.clear();
  tauriMocks.mockSafeInvoke.mockReset();
  tauriMocks.mockSafeListen.mockReset();
  tauriMocks.mockIsTauriAvailable.mockReset();
  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
});

describe("wave-64 F2 终态不得另起消息", () => {
  it("f2_cancelled_without_streamed_body_does_not_write_cancelled_placeholder", async () => {
    // Arrange：本轮从未产出正文（assistant content 为空）
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store, "");
    await store.initializeTurnEvents();

    // Act
    handlers.get("turn:cancelled")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "cancelled",
        phase: "cancelled",
        error: "stopped_by_user",
        stopReason: "user"
      } satisfies TurnStreamEvent
    });

    // Assert：不得向 content 写入取消占位正文
    const assistant = assistantOf(store);
    expect(assistant, "F2：取消后助手消息仍需存在（历史节点非空由状态行承载）").not.toBeNull();
    expect(assistant?.content, "F2：turn:cancelled 禁止写入「本轮已停止。」占位").not.toBe(CANCELLED_PLACEHOLDER);
    expect(assistant?.content ?? "").not.toContain(CANCELLED_PLACEHOLDER);
  });

  it("f2_browser_preview_cancel_does_not_write_cancelled_placeholder", async () => {
    // Arrange：浏览器预览（无 Tauri）走 cancelBrowserPreviewTurn 路径
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    const store = useRuntimeStore();
    seedStreamingTurn(store, "");

    // Act
    const accepted = await store.stopTurn();

    // Assert
    expect(accepted, "F2：浏览器预览取消必须被受理").toBe(true);
    const assistant = assistantOf(store);
    expect(assistant, "F2：取消后助手消息仍存在").not.toBeNull();
    expect(assistant?.content, "F2：浏览器预览取消路径同样禁止写入「本轮已停止。」占位").not.toBe(
      CANCELLED_PLACEHOLDER
    );
    expect(assistant?.content ?? "").not.toContain(CANCELLED_PLACEHOLDER);
  });

  it("f2_failed_turn_without_body_does_not_write_failed_placeholder", async () => {
    // Arrange
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store, "");
    await store.initializeTurnEvents();

    // Act
    handlers.get("turn:failed")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "failed",
        phase: "failed",
        error: REAL_ERROR
      } satisfies TurnStreamEvent
    });

    // Assert
    const assistant = assistantOf(store);
    expect(assistant).not.toBeNull();
    expect(assistant?.content, "F2：turn:failed 禁止写入失败占位正文").not.toBe(FAILED_PLACEHOLDER);
    expect(assistant?.content ?? "").not.toContain(FAILED_PLACEHOLDER);
    // F2：错误明细仍保留在 store（不新增 UI，但事实不能丢）
    expect(assistant?.errorDetail).toBe(REAL_ERROR);
  });

  it("f2_failed_turn_keeps_already_streamed_body", async () => {
    // Arrange：失败轮已产出正文
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store, STREAMED_TEXT);
    await store.initializeTurnEvents();

    // Act：失败终态不带 text（后端没有额外正文）
    handlers.get("turn:failed")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "failed",
        phase: "failed",
        error: REAL_ERROR
      } satisfies TurnStreamEvent
    });

    // Assert：已产出正文照常保留，不被占位/错误文案覆盖
    const assistant = assistantOf(store);
    expect(assistant?.content).toContain(STREAMED_TEXT);
    expect(assistant?.content ?? "").not.toContain(FAILED_PLACEHOLDER);
  });
});

/**
 * T2-5 红相：缺 sessionId 的流事件不得被“当前会话”吸收。
 *
 * 缺陷：`src/stores/runtime.ts:3633` 的会话归属判定是 fail-open：
 *
 *     const isEventForCurrentSession = (payload) =>
 *       !payload.sessionId || payload.sessionId === this.sessionId;
 *
 * 事件只要不带 sessionId 就被当成“属于当前会话”，于是别的会话（后台会话、并发 turn、
 * 甚至伪造/旧版本宿主）发来的 delta / phase_changed 会写进当前会话的助手消息与运行态：
 * 消息串档、phase 被外部拉走、后台会话终态污染前台视图。
 * 事件缺失归属信息时属于**信息不足**，只能 fail-closed（丢弃并留观测），不能默认归属。
 *
 * 断言口径：同一份 payload，缺 sessionId 时不得改变当前会话状态；带正确 sessionId 时必须
 * 正常吸收（正向钉子，防“一刀切全丢”把正常链路打死）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
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

type Listener = (event: { payload: Record<string, unknown> }) => void;

const CURRENT_SESSION = "session-current";
const ACTIVE_TURN = "turn-active";

describe("runtime store 会话归属 fail-closed（T2-5 红相）", () => {
  let listeners: Record<string, Listener> = {};

  beforeEach(() => {
    listeners = {};
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(undefined);
    tauriMocks.mockSafeListen.mockImplementation((event: string, handler: Listener) => {
      listeners[event] = handler;
      return Promise.resolve(vi.fn());
    });
    setActivePinia(createPinia());
  });

  function fired(event: string, payload: Record<string, unknown>) {
    const listener = listeners[event];
    expect(listener, `未注册 ${event} 监听器（用例前置条件被破坏）`).toBeDefined();
    listener({ payload });
  }

  it("缺 sessionId 的 turn:phase_changed 不得改变当前会话的 phase", async () => {
    const store = useRuntimeStore();
    store.sessionId = CURRENT_SESSION;
    store.activeTurnId = ACTIVE_TURN;
    store.phase = "ready";
    await store.initializeTurnEvents();

    fired("turn:phase_changed", {
      turnId: ACTIVE_TURN,
      eventType: "turn.tool_call_started",
      kind: "phase_changed",
      phase: "executing_tool",
      sequence: 1,
      eventId: "evt-foreign-1"
    });

    expect(
      store.phase,
      "缺 sessionId 的事件不得被当前会话吸收（fail-open 会把别的会话的 phase 写进当前会话）"
    ).toBe("ready");
  });

  it("缺 sessionId 的 turn:delta 不得写入当前会话的流缓冲与助手消息", async () => {
    const store = useRuntimeStore();
    store.sessionId = CURRENT_SESSION;
    store.activeTurnId = ACTIVE_TURN;
    store.messages = [];
    await store.initializeTurnEvents();

    fired("turn:delta", {
      turnId: ACTIVE_TURN,
      eventType: "turn.output_delta",
      kind: "delta",
      text: "FOREIGN-DELTA-LEAK",
      sequence: 1,
      eventId: "evt-foreign-2"
    });

    const absorbed = store.streamBufferText;
    const assistant = store.messages.find(
      (message) => message.turnId === ACTIVE_TURN && message.role === "assistant"
    );
    store.cancelStreamFlush();

    expect(absorbed, "缺 sessionId 的 delta 不得进入当前会话的流缓冲").toBe("");
    expect(
      assistant,
      "缺 sessionId 的 delta 不得为当前会话创建/污染助手消息"
    ).toBeUndefined();
  });

  it("正向钉子：带正确 sessionId 的事件必须照常被吸收", async () => {
    const store = useRuntimeStore();
    store.sessionId = CURRENT_SESSION;
    store.activeTurnId = ACTIVE_TURN;
    store.phase = "ready";
    store.messages = [];
    await store.initializeTurnEvents();

    fired("turn:phase_changed", {
      turnId: ACTIVE_TURN,
      sessionId: CURRENT_SESSION,
      eventType: "turn.tool_call_started",
      kind: "phase_changed",
      phase: "executing_tool",
      sequence: 1,
      eventId: "evt-own-1"
    });
    // phase 事件先断言（后续 delta 事件自带 calling_model 语义，会覆盖 phase）
    expect(store.phase, "带正确 sessionId 的 phase 事件必须照常吸收").toBe("calling_tool");

    fired("turn:delta", {
      turnId: ACTIVE_TURN,
      sessionId: CURRENT_SESSION,
      eventType: "turn.output_delta",
      kind: "delta",
      text: "OWN-DELTA",
      sequence: 2,
      eventId: "evt-own-2"
    });

    const absorbed = store.streamBufferText;
    store.cancelStreamFlush();

    expect(absorbed, "带正确 sessionId 的 delta 必须照常进入流缓冲").toBe("OWN-DELTA");
  });
});

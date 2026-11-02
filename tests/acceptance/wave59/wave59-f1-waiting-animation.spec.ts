/**
 * Wave-59 F1 冻结验收：移除与底部「思考中」重复的三点跳跃动画
 * （契约 §1，只读源 `.dev-team/contracts/wave-59-desktop-uiux.md`）。
 *
 * 场景构造：`awaiting-first-signal`（本轮已发起、尚无 reasoning/正文/工具信号）——
 * 与 `tests/HomeWorkspace.spec.ts` 既有夹具同构，保证状态真实可达而非人造。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useRuntimeStore } from "@/stores/runtime";
import { mountWorkspaceWithRealMarkdown, installWorkspaceDomStubs, settleTicks } from "./wave59-harness";

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

/** awaiting-first-signal 的最小可信状态：回合已开始、assistant 尚无任何可见信号。 */
function patchAwaitingFirstSignal(store: ReturnType<typeof useRuntimeStore>) {
  store.$patch({
    sessionId: "w59-f1-session",
    sessionOperation: null,
    sessionError: null,
    error: null,
    phase: "calling_model",
    isSubmitting: true,
    activeTurnId: "turn-f1",
    activeTurnStartedAtMs: Date.now(),
    toolActivities: [],
    streamBufferText: "",
    streamBufferReasoning: "",
    messages: [
      {
        id: "user-f1",
        turnId: "turn-f1",
        role: "user",
        content: "继续",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: null,
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ]
  });
}

describe("wave-59 F1：移除重复的等待动画", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
    installWorkspaceDomStubs();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("F1-1: awaiting-first-signal 下不得存在 .assistant-waiting-dot / .assistant-waiting-panel / assistant-awaiting-first-signal 节点", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    patchAwaitingFirstSignal(store);

    // Act
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await nextTick();

    // Assert（场景自检：确实已进入等待首个信号——回合 section 已渲染且含用户消息）
    const turnSection = wrapper.get('[data-testid="workspace-content-column"] section[data-turn-id]');
    expect(turnSection.text()).toContain("继续");

    // Assert（契约主体：三点跳跃动画整族不得存在）
    const dotProbe = {
      dot: wrapper.findAll(".assistant-waiting-dot").length,
      panel: wrapper.findAll(".assistant-waiting-panel").length,
      testid: wrapper.findAll('[data-testid="assistant-awaiting-first-signal"]').length
    };
    expect(
      dotProbe,
      `F1-1：awaiting-first-signal 状态下不得渲染任何重复等待动画节点（实际：${JSON.stringify(dotProbe)}）`
    ).toEqual({ dot: 0, panel: 0, testid: 0 });

    wrapper.unmount();
  });

  it("F1-2: 同一状态下底部状态条必须可见且标签非空（承载「思考中/准备就绪中」唯一指示）", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    patchAwaitingFirstSignal(store);

    // Act
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await nextTick();

    // Assert（状态条槽位存在且 data-visible="true"）
    const slot = wrapper.get('[data-testid="workspace-runtime-status-slot"]');
    expect(
      slot.attributes("data-visible"),
      "F1-2：等待首个信号期间底部状态条必须可见（data-visible=\"true\"）"
    ).toBe("true");

    // Assert（唯一指示：标签非空且语义为思考/准备就绪）
    const label = wrapper.get('[data-testid="agent-runtime-status-label"]');
    const labelText = label.text().trim();
    expect(labelText.length, "F1-2：agent-runtime-status-label 文本不得为空").toBeGreaterThan(0);
    expect(labelText).toMatch(/思考中|准备就绪/);

    wrapper.unmount();
  });

  it("F1-3: awaiting-first-signal 时不得残留空 agent 壳（无 agent 文章容器、无 workspace-agent-actions）", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    patchAwaitingFirstSignal(store);

    // Act
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await nextTick();

    // Assert（agent 动作行不得渲染）
    expect(
      wrapper.find('[data-testid="workspace-agent-actions"]').exists(),
      "F1-3：等待首个信号期间不得渲染 workspace-agent-actions（空 agent 壳）"
    ).toBe(false);

    // Assert（agent 文章容器不得渲染：该回合 section 内只剩用户消息这一个 article）
    const turnSection = wrapper.get('[data-testid="workspace-content-column"] section[data-turn-id]');
    const articles = turnSection.findAll("article");
    expect(
      articles.map((article) => article.classes().join(" ")),
      "F1-3：等待首个信号时 shouldShowAgentArticle 必须为 false —— 回合内只允许用户消息 article"
    ).toHaveLength(1);
    expect(articles[0]!.classes().join(" ")).toContain("conversation-user-message");

    wrapper.unmount();
  });

  /**
   * F1-4（契约 §7 B-59-1 验收项，BLOCKING）：
   *
   * 缺陷序列：上一轮以 **content 流式**结束（`turn:completed`；按协议末次模型调用不发 `hop_complete`），
   * `activeStreamKind` 未被终态复位 ⇒ 残留 `"content"` ⇒ 用户立即提交下一轮、在 `turn:started`
   * 到达之前的 awaiting-first-signal 窗口被误判 `responding` 且状态条被隐藏；
   * 三点动画已按 F1 删除 ⇒ 该窗口内**零进行中指示**。
   *
   * 与 F1-1/F1-2 的差别（关键）：那两条用全新 pinia（`activeStreamKind=undefined` 走缺省回退），
   * 抓不到残留缺陷；本用例**必须**在同一 store 上先跑完一整轮 content，再走**真实 `submitTurn`**。
   *
   * 断言：状态条 `data-visible="true"`；`mode ∈ {connecting, thinking}`（不得为 `responding`）。
   */
  it("F1-4: 上一轮 content 流式结束后立即 submitTurn（尚未收到 turn:started）窗口内，状态条必须可见且 mode∈{connecting,thinking}", async () => {
    // Arrange：捕获 turn:* handler，并给出可跑真实 submitTurn 的 IPC 桩
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const SESSION = "w59-f1-4-session";
    const handlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      handlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream", runId: null };
      }
      if (command === "start_graph_run_stream") {
        return { run: { id: "run-f1-4" }, turnId: "turn-f1-4-b" };
      }
      if (command === "list_sessions") {
        return [];
      }
      if (command === "inspect_host") {
        return { runs: [] };
      }
      return null;
    });

    const emit = (eventName: string, payload: Record<string, unknown>) => {
      const handler = handlers.get(eventName);
      if (!handler) {
        throw new Error(`未捕获 ${eventName} 监听器（store 未注册该事件）`);
      }
      handler({ payload: { sessionId: SESSION, turnId: "turn-f1-4-a", ...payload } });
    };

    store.$patch({
      sessionId: SESSION,
      sessionOperation: null,
      sessionError: null,
      error: null,
      phase: "ready",
      isSubmitting: false,
      activeTurnId: null,
      activeTurnStartedAtMs: null,
      messages: [],
      toolActivities: [],
      streamBufferText: "",
      streamBufferReasoning: ""
    });
    await store.initializeTurnEvents();

    // ── 第 1 轮：真实事件驱动，以 content 流式结束（turn:completed） ──
    const TURN_A = "turn-f1-4-a";
    store.$patch({
      isSubmitting: true,
      activeTurnId: TURN_A,
      activeTurnStartedAtMs: Date.now(),
      phase: "calling_model"
    });
    emit("turn:started", {
      kind: "started",
      eventType: "turn.model_call_started",
      eventId: "f14-e1",
      sequence: 1,
      emittedAtMs: 1000
    });
    emit("turn:delta", {
      kind: "delta",
      eventType: "turn.output_delta",
      eventId: "f14-e2",
      sequence: 2,
      emittedAtMs: 1001,
      text: "第一轮正文。"
    });
    store.flushBufferedStreamText(TURN_A);
    emit("turn:completed", {
      kind: "completed",
      eventType: "turn.completed",
      phase: "completed",
      eventId: "f14-e3",
      sequence: 3,
      emittedAtMs: 1002,
      text: "第一轮正文。"
    });
    await nextTick();
    expect(
      store.isSubmitting,
      "F1-4 自检：第 1 轮必须已进入终态（turn:completed 后 isSubmitting=false）"
    ).toBe(false);

    // ── Act：立即提交下一轮；此刻【尚未】派发 turn:started（awaiting-first-signal 窗口） ──
    store.$patch({ draftMessage: "第二轮问题" });
    await store.submitTurn();
    await nextTick();
    expect(
      handlers.has("turn:started"),
      "F1-4 自检：store 必须已注册 turn:started 监听器（本用例刻意不派发它）"
    ).toBe(true);
    expect(
      store.isSubmitting,
      "F1-4 自检：submitTurn 之后必须处于「已提交、尚未收到 turn:started」的窗口"
    ).toBe(true);

    // Assert（store 状态真值 + DOM 可见性真值）
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(3, 20);

    const status = store.agentRuntimeStatus;
    const slot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    const observed = {
      mode: status?.mode ?? null,
      label: status?.label ?? null,
      statusSlotVisible: slot.attributes("data-visible") ?? null,
      waitingDots: wrapper.findAll(".assistant-waiting-dot").length
    };

    expect(
      ["connecting", "thinking"],
      `F1-4（B-59-1）：awaiting-first-signal 窗口不得被判为 responding；实际：${JSON.stringify(observed)}`
    ).toContain(observed.mode);
    expect(
      observed.statusSlotVisible,
      `F1-4（B-59-1）：该窗口状态条必须可见（data-visible="true"）；实际：${JSON.stringify(observed)}`
    ).toBe("true");

    wrapper.unmount();
  }, 20000);

  /**
   * A-59-5（契约 §7 复审 r2 登记）：**会话切换不得携带上一会话的 hop 级流式类型**。
   *
   * 缺陷：会话 A 正文流式中（`activeStreamKind="content"`）切到会话 C（C 有后台运行轮，
   * 但本 hop 尚无正文/推理），`switchSession` 的后台轮恢复路径只写
   * `activeTurnId/isSubmitting/phase/streamBuffer*`，**不复位** `activeStreamKind`
   * ⇒ 会话 C 被误判 `responding`、状态条被隐藏 —— 与 B-59-1 同类，且 F1-4 夹具（全新 pinia）
   * 抓不到跨会话残渣。
   *
   * 断言：`data-visible="true"`；`mode ∈ {connecting, thinking}`；label 非空（弱断言，不钉文案）。
   */
  async function runSessionSwitchScenario(backgroundTextBuffer: string) {
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const handlers = new Map<string, (event: { payload: Record<string, unknown> }) => void>();
    const SESSION_A = "w59-f1-5-session-a";
    const SESSION_C = "w59-f1-5-session-c";

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      handlers.set(eventName, handler as (event: { payload: Record<string, unknown> }) => void);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions" || command === "inspect_host") {
        return [];
      }
      return null;
    });

    const emit = (eventName: string, payload: Record<string, unknown>) => {
      const handler = handlers.get(eventName);
      if (!handler) {
        throw new Error(`未捕获 ${eventName} 监听器（store 未注册该事件）`);
      }
      handler({ payload: { sessionId: SESSION_A, turnId: "turn-a", ...payload } });
    };

    // ── 会话 A：真实事件驱动到「正文流式中」（activeStreamKind = "content"） ──
    store.$patch({
      sessionId: SESSION_A,
      sessionOperation: null,
      sessionError: null,
      error: null,
      phase: "ready",
      isSubmitting: false,
      activeTurnId: null,
      activeTurnStartedAtMs: null,
      messages: [],
      toolActivities: [],
      streamBufferText: "",
      streamBufferReasoning: ""
    });
    await store.initializeTurnEvents();

    store.$patch({
      isSubmitting: true,
      activeTurnId: "turn-a",
      activeTurnStartedAtMs: Date.now(),
      phase: "calling_model"
    });
    emit("turn:started", {
      kind: "started",
      eventType: "turn.model_call_started",
      eventId: "f15-a1",
      sequence: 1,
      emittedAtMs: 1000
    });
    emit("turn:delta", {
      kind: "delta",
      eventType: "turn.output_delta",
      eventId: "f15-a2",
      sequence: 2,
      emittedAtMs: 1001,
      text: "会话 A 的正文正在流式输出。"
    });
    store.flushBufferedStreamText("turn-a");
    await nextTick();

    // 自检：会话 A 确实处于 content 流式（否则本场景不成立）
    expect(
      (store.$state as unknown as { activeStreamKind?: string }).activeStreamKind,
      "F1-5 自检：会话 A 必须已进入 activeStreamKind=\"content\" 的正文流式态"
    ).toBe("content");

    // 会话 C 的后台运行轮：phase=calling_model；正文/推理缓冲区按参数构造
    store.$patch({
      runningSessionMap: {
        [SESSION_C]: {
          turnId: "turn-c",
          phase: "calling_model",
          textBuffer: backgroundTextBuffer,
          reasoningBuffer: ""
        }
      }
    });

    // ── Act：真实 switchSession（不派发任何 turn:started / turn:delta） ──
    await store.switchSession(SESSION_C);
    await nextTick();

    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(3, 20);

    const status = store.agentRuntimeStatus;
    const slot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    const observed = {
      sessionId: store.sessionId,
      activeStreamKind: (store.$state as unknown as { activeStreamKind?: string | null }).activeStreamKind ?? null,
      mode: status?.mode ?? null,
      label: status?.label ?? null,
      statusSlotVisible: slot.attributes("data-visible") ?? null
    };

    return { wrapper, observed };
  }

  it("F1-5: 会话切换不得携带上一会话的 hop 级流式类型（含「后台轮有正文缓冲 ⇒ responding」子断言）", async () => {
    // ── 场景 1（契约主体）：后台轮 textBuffer 为空 ⇒ 不得继承 content；状态条必须可见 ──
    const emptyBuffer = await runSessionSwitchScenario("");
    expect(
      ["connecting", "thinking"],
      `F1-5（A-59-5）：切到「本 hop 尚无正文」的后台轮后不得继承上一会话的 content 流式类型；实际：${JSON.stringify(emptyBuffer.observed)}`
    ).toContain(emptyBuffer.observed.mode);
    expect(
      emptyBuffer.observed.statusSlotVisible,
      `F1-5（A-59-5）：该窗口状态条必须可见（data-visible="true"）；实际：${JSON.stringify(emptyBuffer.observed)}`
    ).toBe("true");
    expect(
      (emptyBuffer.observed.label ?? "").trim().length,
      `F1-5（A-59-5）：状态文案不得为空（弱断言，不钉具体措辞）；实际：${JSON.stringify(emptyBuffer.observed)}`
    ).toBeGreaterThan(0);
    emptyBuffer.wrapper.unmount();

    // ── 场景 2（子断言，推导正确性）：后台轮 textBuffer 非空 ⇒ 应按缓冲推导为 responding ──
    const withBuffer = await runSessionSwitchScenario("后台会话正在输出的正文。");
    expect(
      withBuffer.observed.mode,
      `F1-5 子断言（A-59-5）：后台轮 textBuffer 非空时 mode 必须为 responding（按缓冲区推导，而非一律置空）；实际：${JSON.stringify(withBuffer.observed)}`
    ).toBe("responding");
    withBuffer.wrapper.unmount();
  }, 30000);
});

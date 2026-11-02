/**
 * Wave-59 F4 冻结验收：底部状态消息准确性（契约 §4，只读源
 * `.dev-team/contracts/wave-59-desktop-uiux.md`；缺陷 RC6/RC7/RC8/RC9）。
 *
 * 驱动方式（不伪造内部状态）：
 * - F4-1 / F4-2 / F4-3 用 `mockSafeListen` 捕获 store 注册的 `turn:*` handler，
 *   按真实 wire 形态（sessionId / turnId / kind / sequence / eventType）逐事件驱动
 *   turn 状态机 —— 与 `tests/runtime-store.spec.ts` 既有做法同构；
 * - 断言口径以契约点名的两处为准：store 派生的 `agentRuntimeStatus.mode`（状态真值）
 *   与 `[data-testid="workspace-runtime-status-slot"][data-visible]`（可见性真值）。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { Pinia } from "pinia";
import { nextTick } from "vue";
import { useRuntimeStore } from "@/stores/runtime";
import { resolveAgentRuntimeStatus } from "@/lib/runtime/status";
import { installWorkspaceDomStubs, mountWorkspaceWithRealMarkdown, settleTicks } from "./wave59-harness";

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

const SESSION_ID = "w59-f4-session";
const TURN_ID = "turn-f4";

type TurnHandler = (event: { payload: Record<string, unknown> }) => void;

interface RuntimeHarness {
  store: ReturnType<typeof useRuntimeStore>;
  emit: (eventName: string, payload: Record<string, unknown>) => void;
  capturedNames: () => string[];
}

/** 捕获 store 注册的全部 `turn:*` handler，并按真实 envelope 形态派发。 */
async function createRuntimeHarness(pinia: Pinia): Promise<RuntimeHarness> {
  setActivePinia(pinia);
  const store = useRuntimeStore();
  const handlers = new Map<string, TurnHandler>();

  tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  tauriMocks.mockSafeInvoke.mockResolvedValue(null);
  tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
    handlers.set(eventName, handler as TurnHandler);
    return () => {};
  });

  store.$patch({
    sessionId: SESSION_ID,
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

  return {
    store,
    emit(eventName, payload) {
      const handler = handlers.get(eventName);
      if (!handler) {
        throw new Error(`未捕获 ${eventName} 监听器（store 未注册该事件）`);
      }
      handler({
        payload: {
          sessionId: SESSION_ID,
          turnId: TURN_ID,
          ...payload
        }
      });
    },
    capturedNames: () => [...handlers.keys()]
  };
}

/** 开始一轮：isSubmitting + activeTurnId 与事件 envelope 对齐（生产由 submitTurn 完成）。 */
async function startTurn(harness: RuntimeHarness) {
  harness.store.$patch({
    isSubmitting: true,
    activeTurnId: TURN_ID,
    activeTurnStartedAtMs: Date.now(),
    phase: "calling_model"
  });
  harness.emit("turn:started", {
    kind: "started",
    eventType: "turn.model_call_started",
    eventId: "evt-started-1",
    sequence: 1,
    emittedAtMs: 1000
  });
  await nextTick();
}

/** delta 事件写入缓冲区；此处显式冲刷，保证断言看到的是「已到达消息」的内容。 */
async function emitDelta(harness: RuntimeHarness, sequence: number, delta: { text?: string; reasoningContent?: string }) {
  harness.emit("turn:delta", {
    kind: "delta",
    eventType: "turn.output_delta",
    eventId: `evt-delta-${sequence}`,
    sequence,
    emittedAtMs: 1000 + sequence,
    ...delta
  });
  harness.store.flushBufferedStreamText(TURN_ID);
  await nextTick();
}

describe("wave-59 F4：底部状态消息准确性", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    // 释放节奏不属本波范围（D-59-2 保留）；放大释放速率只为让「正文已可见」前提确定成立。
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setActivePinia(createPinia());
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    installWorkspaceDomStubs();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("F4-1: hop 内 reasoning delta 后 mode=thinking，紧随 text delta 后必须切到 mode=responding", async () => {
    // Arrange
    const pinia = createPinia();
    const harness = await createRuntimeHarness(pinia);
    await startTurn(harness);
    expect(
      harness.capturedNames(),
      "F4-1 自检：store 必须注册 turn:started / turn:delta 监听器"
    ).toEqual(expect.arrayContaining(["turn:started", "turn:delta"]));

    // Act（同 hop 内先 reasoning 再正文）
    await emitDelta(harness, 2, { reasoningContent: "先梳理一下思路。" });
    const afterReasoning = harness.store.agentRuntimeStatus;

    await emitDelta(harness, 3, { text: "正文第一句。" });
    const afterText = harness.store.agentRuntimeStatus;

    // Assert（契约：reasoning delta → thinking）
    expect(
      afterReasoning?.mode,
      `F4-1：reasoning delta 后必须为 thinking（实际：${JSON.stringify(afterReasoning)}）`
    ).toBe("thinking");

    // Assert（契约：同 hop 紧随 text delta → responding，不得因本轮曾有 reasoning 而停留 thinking，RC6）
    expect(
      afterText?.mode,
      `F4-1：正文 delta 到达后必须切到 responding（实际：${JSON.stringify(afterText)}）`
    ).toBe("responding");
  });

  it("F4-2: 工具事件后的新 hop（尚无 reasoning/text delta）→ mode=thinking 且状态条 data-visible=true", async () => {
    // Arrange
    const pinia = createPinia();
    const harness = await createRuntimeHarness(pinia);
    await startTurn(harness);

    // hop 1 产出正文，随后调用工具并完成
    await emitDelta(harness, 2, { text: "先查一下文件。" });
    harness.emit("turn:tool", {
      kind: "tool",
      eventType: "turn.tool_call_completed",
      phase: "calling_model",
      eventId: "evt-tool-3",
      sequence: 3,
      emittedAtMs: 1003,
      toolActivities: [
        {
          id: "act-f4",
          turnId: TURN_ID,
          name: "read_file",
          canonicalToolName: "Read",
          displayNameZh: "读取文件",
          status: "done"
        }
      ]
    });
    await nextTick();
    expect(harness.store.toolActivities.map((activity) => activity.status)).toEqual(["done"]);

    // hop 2（工具后第二轮模型调用）开始：此刻本 hop 尚无任何 reasoning/text delta
    harness.emit("turn:started", {
      kind: "started",
      eventType: "turn.model_call_started",
      eventId: "evt-started-4",
      sequence: 4,
      emittedAtMs: 1004
    });
    await nextTick();

    // Act（把同一 store 状态挂到真实工作区上，读可见性真值）
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(2, 20);

    // Assert（契约：新 hop 等待首信号 → thinking（RC7）+ 状态条重新可见（RC8））
    const status = harness.store.agentRuntimeStatus;
    const slot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(
      slot.exists(),
      "F4-2：运行期间状态条槽位必须存在（wave-14 运行期始终可见）"
    ).toBe(true);

    // A-59-3（审查裁定）：只断言状态真值（mode）与可见性真值（data-visible），
    // 不钉死 label 文案（文案属展示层，允许后续措辞调整）。
    const observed = {
      mode: status?.mode ?? null,
      statusSlotVisible: slot.attributes("data-visible") ?? null
    };
    expect(
      observed,
      `F4-2：工具后的新 hop（本 hop 尚无 reasoning/text delta）必须为 thinking 且状态条可见，实际：${JSON.stringify(observed)}`
    ).toEqual({ mode: "thinking", statusSlotVisible: "true" });
    expect(
      (status?.label ?? "").trim().length,
      "F4-2：状态文案不得为空（仅要求非空，不约束具体措辞）"
    ).toBeGreaterThan(0);

    wrapper.unmount();
  });

  it("F4-3: 正文流式（当前 hop 有 text delta 且正文可见）→ 状态条 data-visible=false（沿用优化3）", async () => {
    // Arrange
    const pinia = createPinia();
    const harness = await createRuntimeHarness(pinia);
    await startTurn(harness);
    await emitDelta(harness, 2, { text: "正文正在流式输出。" });

    // Act
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(4, 24);

    // Assert（自检：正文确实已渲染可见，避免「无正文」下的假绿）
    // 锚点用契约级语义（回合 section 文本），不绑定内部实现类名。
    const turnText = wrapper.get('[data-testid="workspace-content-column"] section[data-turn-id]').text();
    expect(
      turnText,
      "F4-3 自检：正文必须已到达 DOM 且可见（否则本契约的「正文可见」前提不成立）"
    ).toContain("正文正在流式输出。");

    // Assert（契约：正文流式期间状态条隐藏）
    const slot = wrapper.get('[data-testid="workspace-runtime-status-slot"]');
    expect(
      slot.attributes("data-visible"),
      "F4-3：正文流式且正文可见时状态条必须隐藏（data-visible=\"false\"）"
    ).toBe("false");

    wrapper.unmount();
  });

  it("F4-4: 既有 wave-14 三例可见性语义保持（正文流式隐藏 / 工具执行可见 / 工具草稿可见）", async () => {
    // 与 `tests/workspace-runtime-status-streaming.spec.ts` 三例同构的并行守卫
    // （该 spec 本身在红相收据中作为一等证据单独执行）。

    // 例 1：正文流式 → 隐藏
    const piniaA = createPinia();
    setActivePinia(piniaA);
    const storeA = useRuntimeStore();
    storeA.$patch({
      sessionId: "sess-a",
      activeTurnId: "turn-a",
      isSubmitting: true,
      phase: "calling_model",
      messages: [
        { id: "u-a", turnId: "turn-a", role: "user", content: "你好", status: "done", tokenCount: null },
        { id: "a-a", turnId: "turn-a", role: "assistant", content: "正在为你生成回答的第一部分...", status: "pending", tokenCount: null }
      ]
    });
    const wrapperA = mountWorkspaceWithRealMarkdown(piniaA);
    await settleTicks(2, 20);
    expect(
      wrapperA.get('[data-testid="workspace-runtime-status-slot"]').attributes("data-visible"),
      "F4-4 例1：正文流式时状态条必须隐藏"
    ).toBe("false");
    wrapperA.unmount();

    // 例 2：工具执行中 → 可见 + 文案「执行工具」
    const piniaB = createPinia();
    setActivePinia(piniaB);
    const storeB = useRuntimeStore();
    storeB.$patch({
      sessionId: "sess-b",
      activeTurnId: "turn-b",
      isSubmitting: true,
      phase: "calling_tool",
      messages: [
        { id: "u-b", turnId: "turn-b", role: "user", content: "查看文件", status: "done", tokenCount: null }
      ],
      toolActivities: [{ id: "act-b", turnId: "turn-b", name: "read_file", status: "running" }]
    });
    const wrapperB = mountWorkspaceWithRealMarkdown(piniaB);
    await settleTicks(2, 20);
    const slotB = wrapperB.get('[data-testid="workspace-runtime-status-slot"]');
    expect(slotB.attributes("data-visible"), "F4-4 例2：工具执行中状态条必须可见").toBe("true");
    expect(slotB.text()).toContain("执行工具");
    wrapperB.unmount();

    // 例 3：工具草稿（planned）→ 可见 + 文案「准备调用工具」
    const piniaC = createPinia();
    setActivePinia(piniaC);
    const storeC = useRuntimeStore();
    storeC.$patch({
      sessionId: "sess-c",
      activeTurnId: "turn-c",
      isSubmitting: true,
      phase: "calling_model",
      messages: [
        { id: "u-c", turnId: "turn-c", role: "user", content: "读一下文件", status: "done", tokenCount: null },
        { id: "a-c", turnId: "turn-c", role: "assistant", content: "", status: "pending", tokenCount: null }
      ],
      toolActivities: [
        { id: "act-c", turnId: "turn-c", name: "read_file", canonicalToolName: "Read", displayNameZh: "读取文件", status: "planned" }
      ]
    });
    const wrapperC = mountWorkspaceWithRealMarkdown(piniaC);
    await settleTicks(2, 20);
    const slotC = wrapperC.get('[data-testid="workspace-runtime-status-slot"]');
    expect(slotC.attributes("data-visible"), "F4-4 例3：工具草稿状态条必须可见").toBe("true");
    expect(slotC.text()).toContain("准备调用工具");
    wrapperC.unmount();
  });

  /**
   * F4-5（契约 §7 D-59-9 / RC9）：retrying 的**可达性必须由生产路径驱动**。
   *
   * 生产路径（`submitTurn` 内的真实代码）：上一轮 assistant 的 `errorDetail` 命中
   * `isRetryableError`（超时/限流/5xx…）⇒ 新提交时把「超时重连中…」占位写进新的 pending assistant
   * ⇒ store 把它作为 retrying 的唯一可视信号（`retryAttempt` 由 store 传入 resolver）。
   * 本用例**不再**由夹具直接写 `TIMEOUT_RETRY_PENDING_MESSAGE` 自证（审查 A-59-5 / D-59-9 禁止）。
   */
  it("F4-5: retrying 由生产路径（submitTurn 遇可重试错误）真实触达，且新一轮 start 后清零", async () => {
    // Arrange：捕获 handler + 可跑真实 submitTurn 的 IPC 桩
    const pinia = createPinia();
    const handlers = new Map<string, TurnHandler>();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const SESSION = "w59-f4-5-session";
    const RETRYABLE_ERROR = "请求超时（request timeout）";

    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockImplementation(async (eventName: string, handler: unknown) => {
      handlers.set(eventName, handler as TurnHandler);
      return () => {};
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream", runId: null };
      }
      if (command === "start_graph_run_stream") {
        return { run: { id: "run-f4-5" }, turnId: "turn-f4-5-b" };
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
      handler({ payload: { sessionId: SESSION, turnId: "turn-f4-5-b", ...payload } });
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
      toolActivities: [],
      streamBufferText: "",
      streamBufferReasoning: "",
      // 上一轮以「可重试错误」失败结束（生产路径的唯一触发条件）
      messages: [
        {
          id: "user-f4-5-a",
          turnId: "turn-f4-5-a",
          role: "user",
          content: "第一轮问题",
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
          id: "assistant-f4-5-a",
          turnId: "turn-f4-5-a",
          role: "assistant",
          content: "",
          status: "error",
          tokenCount: null,
          reasoningContent: null,
          modelName: "OpenAI/GPT-5",
          toolName: null,
          detail: null,
          durationSeconds: null,
          errorDetail: RETRYABLE_ERROR
        }
      ]
    });
    await store.initializeTurnEvents();

    // 纯函数侧能力声明（route B 判定用）
    const declaresRetrying =
      resolveAgentRuntimeStatus({ isSubmitting: true, phase: "calling_model", retryAttempt: 1 })?.mode === "retrying";

    // Act：走生产路径 —— 真实 submitTurn（尚未派发 turn:started）
    store.$patch({ draftMessage: "重试这个问题" });
    await store.submitTurn();
    await nextTick();
    expect(
      store.isSubmitting,
      "F4-5 自检：submitTurn 之后必须处于提交中窗口（尚未收到 turn:started）"
    ).toBe(true);

    const retryStatus = store.agentRuntimeStatus;

    // Assert（路线 B：能力已删除 —— 生产路径不得出现 retrying）
    if (!declaresRetrying) {
      const residual = [1, 2, 3].map(
        (attempt) =>
          resolveAgentRuntimeStatus({ isSubmitting: true, phase: "calling_model", retryAttempt: attempt })?.mode
      );
      expect(
        residual,
        "F4-5：若 retrying 分支已删除，resolveAgentRuntimeStatus 不得再返回 retrying（否则删除不彻底）"
      ).not.toContain("retrying");
      expect(
        retryStatus?.mode,
        "F4-5：resolver 已无 retrying 能力时，生产路径也不得出现 retrying"
      ).not.toBe("retrying");
      return;
    }

    // Assert（路线 A：能力声明存在 ⇒ 必须由生产路径真实触达，否则即「不可达分支冒充能力」）
    expect(
      retryStatus?.mode,
      `F4-5（D-59-9）：submitTurn 遇可重试错误后必须由生产路径触达 retrying；实际：${JSON.stringify(retryStatus)}；`
        + `messages=${JSON.stringify(store.messages.map((message) => ({ role: message.role, status: message.status, content: message.content })))}`
    ).toBe("retrying");
    expect(
      (retryStatus?.label ?? "").trim().length,
      "F4-5：retrying 状态文案不得为空"
    ).toBeGreaterThan(0);

    // Assert（可见性：retrying 属高优先级态，状态条必须可见）
    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(2, 20);
    const slot = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
    expect(
      slot.exists(),
      "F4-5：retrying 期间状态条槽位必须存在"
    ).toBe(true);
    expect(
      slot.attributes("data-visible"),
      "F4-5：retrying 期间状态条必须可见（data-visible=\"true\"）"
    ).toBe("true");

    // Assert（清零：新一轮 turn:started 之后不得继续显示 retrying）
    // 注意：submitTurn 以自身 requestId 作为本轮 turnId（不是 IPC 响应的 turnId），
    // 故 turn:started 必须用 store.activeTurnId 派发，否则会被 handler 的轮次门禁丢弃。
    const activeTurnId = store.activeTurnId;
    expect(activeTurnId, "F4-5 自检：submitTurn 后必须存在 activeTurnId").toBeTruthy();
    emit("turn:started", {
      turnId: activeTurnId,
      kind: "started",
      eventType: "turn.model_call_started",
      eventId: "evt-f4-5-started",
      sequence: 10,
      emittedAtMs: 1010
    });
    await nextTick();
    expect(
      store.agentRuntimeStatus?.mode,
      "F4-5：回到 calling_model（新一轮 turn:started）后 retrying 信号必须清零，不得整轮永久显示「正在重试」"
    ).not.toBe("retrying");

    wrapper.unmount();
  }, 20000);

  /**
   * F4-6（契约 §7 A-59-4 补测）：`error` / `sessionError` / `sessionOperation`（切换 / 初始化）
   * 必须强制 `data-visible="true"`，不得被 F4-3 的「正文流式即隐藏」规则压过。
   *
   * 构造：每种态都放一条**有待展示正文的 pending assistant**（F4-3 的隐藏压力），
   * 再断言该态下状态条仍可见且 mode 属于该类态的合法集合。
   */
  it("F4-6a: error / sessionError 强制可见（不被 F4-3 压过）", async () => {
    const pendingAssistant = {
      id: "assistant-f4-6",
      turnId: "turn-f4-6",
      role: "assistant" as const,
      content: "已有可见正文，按 F4-3 本应隐藏状态条。",
      status: "pending" as const,
      tokenCount: null,
      reasoningContent: null,
      modelName: "OpenAI/GPT-5",
      toolName: null,
      detail: null,
      durationSeconds: null,
      errorDetail: null
    };
    const userMessage = {
      id: "user-f4-6",
      turnId: "turn-f4-6",
      role: "user" as const,
      content: "继续",
      status: "done" as const,
      tokenCount: null,
      reasoningContent: null,
      modelName: null,
      toolName: null,
      detail: null,
      durationSeconds: null,
      errorDetail: null
    };

    const scenarios: Array<{ name: string; patch: Record<string, unknown>; modes: string[] }> = [
      {
        name: "error",
        patch: { isSubmitting: false, error: "模型调用失败：upstream 500", sessionError: null, sessionOperation: null },
        modes: ["error"]
      },
      {
        name: "sessionError",
        patch: { isSubmitting: false, error: null, sessionError: "切换到该对话失败", sessionOperation: null },
        modes: ["error"]
      }
    ];

    for (const scenario of scenarios) {
      const pinia = createPinia();
      setActivePinia(pinia);
      const store = useRuntimeStore();
      store.$patch({
        sessionId: `w59-f4-6a-${scenario.name}`,
        phase: "calling_model",
        activeTurnId: "turn-f4-6",
        activeTurnStartedAtMs: Date.now(),
        toolActivities: [],
        messages: [userMessage, pendingAssistant] as never,
        ...scenario.patch
      });

      const wrapper = mountWorkspaceWithRealMarkdown(pinia);
      await settleTicks(2, 20);
      const slotProbe = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
      const observed = {
        scenario: scenario.name,
        mode: store.agentRuntimeStatus?.mode ?? null,
        visible: slotProbe.attributes("data-visible") ?? null
      };

      expect(
        scenario.modes,
        `F4-6a（${scenario.name}）：mode 必须为 ${JSON.stringify(scenario.modes)}；实际：${JSON.stringify(observed)}`
      ).toContain(observed.mode);
      expect(
        observed.visible,
        `F4-6a（${scenario.name}）：该态必须强制状态条可见（不被 F4-3 压过）；实际：${JSON.stringify(observed)}`
      ).toBe("true");

      wrapper.unmount();
    }
  }, 20000);

  it("F4-6b: sessionOperation（switching / initializing）强制可见（不被 F4-3 压过）", async () => {
    const pendingAssistant = {
      id: "assistant-f4-6b",
      turnId: "turn-f4-6b",
      role: "assistant" as const,
      content: "已有可见正文，按 F4-3 本应隐藏状态条。",
      status: "pending" as const,
      tokenCount: null,
      reasoningContent: null,
      modelName: "OpenAI/GPT-5",
      toolName: null,
      detail: null,
      durationSeconds: null,
      errorDetail: null
    };
    const userMessage = {
      id: "user-f4-6b",
      turnId: "turn-f4-6b",
      role: "user" as const,
      content: "继续",
      status: "done" as const,
      tokenCount: null,
      reasoningContent: null,
      modelName: null,
      toolName: null,
      detail: null,
      durationSeconds: null,
      errorDetail: null
    };

    for (const operation of ["switching", "initializing", "deleting"] as const) {
      const pinia = createPinia();
      setActivePinia(pinia);
      const store = useRuntimeStore();
      store.$patch({
        sessionId: `w59-f4-6b-${operation}`,
        phase: "calling_model",
        activeTurnId: "turn-f4-6b",
        activeTurnStartedAtMs: Date.now(),
        toolActivities: [],
        isSubmitting: false,
        error: null,
        sessionError: null,
        sessionOperation: operation,
        messages: [userMessage, pendingAssistant] as never
      });

      const wrapper = mountWorkspaceWithRealMarkdown(pinia);
      await settleTicks(2, 20);
      const slotProbe = wrapper.find('[data-testid="workspace-runtime-status-slot"]');
      const observed = {
        operation,
        mode: store.agentRuntimeStatus?.mode ?? null,
        visible: slotProbe.attributes("data-visible") ?? null
      };

      expect(
        observed.mode,
        `F4-6b（${operation}）：会话级操作必须给出 connecting 态；实际：${JSON.stringify(observed)}`
      ).toBe("connecting");
      expect(
        observed.visible,
        `F4-6b（${operation}）：该态必须强制状态条可见（不被 F4-3 压过）；实际：${JSON.stringify(observed)}`
      ).toBe("true");

      wrapper.unmount();
    }
  }, 20000);

  /**
   * F4-6c（契约 §7 A-59-4）：≥3 连续 hop 的 `activeStreamKind` 复位。
   * 每个 hop：reasoning delta → thinking；text delta → responding；工具完成 + 新 hop start → 重新 thinking。
   * 3 个 hop 全部按 hop 级语义切换，即证明复位链路（D-59-3）不是「一轮只对一次」。
   */
  it("F4-6c: ≥3 连续 hop 均按 hop 级语义切换（reasoning→thinking / text→responding / 新 hop→thinking）", async () => {
    const pinia = createPinia();
    const harness = await createRuntimeHarness(pinia);
    const { store, emit } = harness;
    await startTurn(harness);

    const hopModes: Array<{ hop: number; afterReasoning: string | null; afterText: string | null }> = [];
    const HOP_COUNT = 3;

    for (let hop = 1; hop <= HOP_COUNT; hop += 1) {
      if (hop > 1) {
        // 上一 hop 的工具已完成；新 hop 由 turn:started（新的模型调用）开启
        harness.emit("turn:started", {
          kind: "started",
          eventType: "turn.model_call_started",
          eventId: `f4-6c-started-${hop}`,
          sequence: 2 * hop + 10,
          emittedAtMs: 1000 + hop * 10
        });
        await nextTick();
      }

      await emitDelta(harness, 2 * hop + 11, { reasoningContent: `第 ${hop} 个 hop 的思考。` });
      const afterReasoning = store.agentRuntimeStatus?.mode ?? null;

      await emitDelta(harness, 2 * hop + 12, { text: `第 ${hop} 个 hop 的正文。` });
      const afterText = store.agentRuntimeStatus?.mode ?? null;

      hopModes.push({ hop, afterReasoning, afterText });

      // 本 hop 结束：工具调用完成（下一轮循环用新的 turn:started 开新 hop）
      harness.emit("turn:tool", {
        kind: "tool",
        eventType: "turn.tool_call_completed",
        phase: "calling_model",
        eventId: `f4-6c-tool-${hop}`,
        sequence: 2 * hop + 13,
        emittedAtMs: 1000 + hop * 10 + 3,
        toolActivities: [
          {
            id: `f4-6c-act-${hop}`,
            turnId: TURN_ID,
            name: "read_file",
            canonicalToolName: "Read",
            displayNameZh: "读取文件",
            status: "done"
          }
        ]
      });
      await nextTick();
    }

    expect(
      hopModes,
      `F4-6c：每个 hop 都必须 reasoning→thinking、text→responding；实际：${JSON.stringify(hopModes)}`
    ).toEqual([
      { hop: 1, afterReasoning: "thinking", afterText: "responding" },
      { hop: 2, afterReasoning: "thinking", afterText: "responding" },
      { hop: 3, afterReasoning: "thinking", afterText: "responding" }
    ]);
  }, 20000);
});

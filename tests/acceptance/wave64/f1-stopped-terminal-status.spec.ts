// wave-64 黑盒验收（独立测试者，红相先写）：F1 终态状态消息的唯一承载面。
//
// 契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 F1 / §4（终态状态派生优先序）。
// 判据（断言的是**契约要求的目标行为**，不是当前实现现状）：
// 1. `turn:cancelled` 后：`agentRuntimeStatus.mode === "stopped"`、`label === "已停止"`，
//    且 `sessionNotice === null`（停止文案不再写 notice）。
// 2. `stopped` 语义色为中性 stone：状态栏 HTML 内**不得**出现 emerald / 绿系。
// 3. `stopped` 图标为 lucide `Pause`（`data-testid=agent-runtime-status-icon` / `data-mode=stopped`），
//    不得出现 `CheckCircle2`（info 专用）。
// 4. `stopped` 无 shimmer 扫光、无耗时计时（`startTime === null`，无 elapsed 节点）。
// 5. `turn:failed` 后：`agentRuntimeStatus.mode === "error"` 且 label 为**真实错误文案**（不是占位）。
//
// 纪律（test-expert L2）：真实断言、AAA、禁止 skip / catch 吞异常 / 空断言。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AgentRuntimeStatusBar from "@/components/chat/AgentRuntimeStatusBar.vue";
import type { AgentRuntimeStatus } from "@/lib/runtime/status";
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

/** 后端取消哨兵句（§0 非目标：仍留在落库内容，但不得冒充终态文案）。 */
const CANCELLED_TURN_SENTINEL = "用户终止，发送消息可继续。";
/** F1 目标终态标签（唯一状态消息）。 */
const STOPPED_LABEL = "已停止";
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

function seedStreamingTurn(store: ReturnType<typeof useRuntimeStore>, turnId = "turn-stop") {
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
});

describe("wave-64 F1 终态状态消息（stopped 唯一承载面）", () => {
  it("f1_turn_cancelled_projects_stopped_mode_and_clears_session_notice", async () => {
    // Arrange
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    // Act：后端终态（user 取消）
    handlers.get("turn:cancelled")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "cancelled",
        phase: "cancelled",
        text: CANCELLED_TURN_SENTINEL,
        error: "stopped_by_user",
        stopReason: "user"
      } satisfies TurnStreamEvent
    });

    // Assert：终态收敛到 stopped 模式，且停止文案不再写 sessionNotice
    expect(store.phase).toBe("cancelled");
    expect(store.isSubmitting).toBe(false);
    const status = store.agentRuntimeStatus;
    expect(status, "F1：turn:cancelled 后必须有终态状态消息（不得为 null）").not.toBeNull();
    expect(status?.mode, "F1：turn:cancelled 后 mode 必须是 stopped").toBe("stopped");
    expect(status?.label, "F1：stopped 的 label 必须是「已停止」").toBe(STOPPED_LABEL);
    expect(status?.startTime, "F1：stopped 无耗时计时（startTime 为 null）").toBeNull();
    expect(store.sessionNotice, "F1：停止文案不得写入 sessionNotice").toBeNull();
  });

  it("f1_non_user_stop_reason_also_projects_stopped_without_notice", async () => {
    // Arrange
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();

    // Act：非 user 原因（watchdog）同样是「已停止」终态，而不是 info 提示
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

    // Assert
    expect(store.agentRuntimeStatus?.mode).toBe("stopped");
    expect(store.sessionNotice).toBeNull();
  });

  it("f1_turn_failed_projects_error_mode_with_real_error_label", async () => {
    // Arrange
    const store = useRuntimeStore();
    const handlers = installEventCapture();
    seedStreamingTurn(store);
    await store.initializeTurnEvents();
    const realError = "tool chain exploded: EACCES on /tmp/secret";

    // Act
    handlers.get("turn:failed")?.({
      payload: {
        sessionId: "session-stop",
        turnId: "turn-stop",
        kind: "failed",
        phase: "failed",
        text: "This turn failed.",
        error: realError
      } satisfies TurnStreamEvent
    });

    // Assert：error 模式 + 真实错误文案（不得是「本轮执行失败，请查看右侧 trace。」之类的占位）
    const status = store.agentRuntimeStatus;
    expect(status?.mode).toBe("error");
    expect(status?.label).toBe(realError);
    expect(status?.label).not.toContain("请查看右侧 trace");
  });

  it("f1_stopped_status_bar_renders_pause_icon_with_neutral_tone", () => {
    // Arrange
    const status: AgentRuntimeStatus = {
      mode: "stopped",
      label: STOPPED_LABEL,
      startTime: null
    };

    // Act
    const wrapper = mount(AgentRuntimeStatusBar, { props: { status } });
    const html = wrapper.html();

    // Assert：图标是 lucide Pause（svg），且 data-mode 回传 stopped
    const icon = wrapper.get('[data-testid="agent-runtime-status-icon"]');
    expect(icon.element.tagName.toLowerCase(), "F1：stopped 图标必须是 lucide 图标（svg），而不是轨道占位 span").toBe("svg");
    expect(icon.classes().join(" "), "F1：stopped 图标必须是 lucide Pause").toContain("lucide-pause");
    expect(icon.attributes("data-mode")).toBe("stopped");

    // Assert：中性 stone 语义色，禁止 emerald / 绿系；禁止 CheckCircle2（info 专用）
    expect(html, "F1：stopped 不得使用 emerald / 绿系语义色").not.toContain("emerald");
    expect(wrapper.find('[data-testid="agent-runtime-status-icon"]').classes().join(" ")).not.toContain("lucide-circle-check");
    // Assert：label 文本与「无 shimmer 扫光」
    const label = wrapper.get('[data-testid="agent-runtime-status-label"]');
    expect(label.text()).toBe(STOPPED_LABEL);
    expect(label.classes(), "F1：stopped 不得挂 shimmer 扫光类").not.toContain("runtime-status-shimmer");
    // Assert：无耗时计时
    expect(wrapper.find('[data-testid="agent-runtime-status-elapsed"]').exists()).toBe(false);
  });

  it("f1_error_status_bar_keeps_alert_icon_and_rose_tone", () => {
    // Arrange
    const status: AgentRuntimeStatus = {
      mode: "error",
      label: "工具调用失败：EACCES",
      startTime: null
    };

    // Act
    const wrapper = mount(AgentRuntimeStatusBar, { props: { status } });
    const icon = wrapper.get('[data-testid="agent-runtime-status-icon"]');

    // Assert：error 仍是 lucide AlertCircle + rose 语义色（F1 不得退化）
    expect(icon.element.tagName.toLowerCase()).toBe("svg");
    expect(icon.classes().join(" ")).toContain("lucide-circle-alert");
    expect(wrapper.get('[data-testid="agent-runtime-status-label"]').classes()).toContain("text-rose-600");
    expect(wrapper.html()).not.toContain("emerald");
  });
});

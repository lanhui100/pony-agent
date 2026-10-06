//! PA-114: ask_user 前端组件与接线 验收测试 — 红相（red phase）
//!
//! 契约依据：`.dev-team/contract-matrix-ask-user.md`（Lead 冻结）F1-1 ~ F1-4。
//!
//! 红相说明：`src/lib/runtime/ask-tools.ts` 与 `src/components/ask/AskUserToolCallCard.vue`
//! 尚不存在，本文件顶层 import 在红相时期直接解析失败（即红相证据，符合约定）；
//! 绿相实现落地后 import 解析成功，以下断言全部通过。
//!
//! 组件契约假设（按契约矩阵 F1-2/F1-3）：
//! - `AskUserToolCallCard` 接收结构化工具调用 `tool`（含 `toolName`、`callId`、`runId`、
//!   `arguments`），渲染 question（优先级 tool 参数 question → text → prompt，回退匹配
//!   pending ask 的 prompt）。
//! - 存在匹配 pending ask（按 callId/runId）时渲染 `ask-user-option` / `ask-user-typed-input`
//!   / `ask-user-answer` / `ask-user-cancel`；无匹配时渲染 `ask-user-waiting` 待命态；
//!   busy 态禁用交互。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { isAskToolName } from "@/lib/runtime/ask-tools";
import AskUserToolCallCard from "@/components/ask/AskUserToolCallCard.vue";
import WorkspaceTurnItem from "@/components/chat/WorkspaceTurnItem.vue";
import type { AgentTurnEvent, MergedToolCall, TurnBucket } from "@/components/chat/WorkspaceTurnItem.vue";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

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

/** 契约 F1-2/F1-3 规定的结构化 ask 工具调用形状（Executor 实现以此为蓝本）。 */
type AskToolCall = {
  id: string;
  toolName: string;
  canonicalToolName: string | null;
  callId: string | null;
  runId: string | null;
  arguments: Record<string, unknown> | null;
  status?: string;
};

function createAskToolCall(partial: Partial<AskToolCall> = {}): AskToolCall {
  return {
    id: partial.id ?? "tool-ask-1",
    toolName: partial.toolName ?? "ask_user",
    canonicalToolName: partial.canonicalToolName ?? "Ask",
    // 显式 null 必须保留（T2：无 callId 不绑定的探针输入）；`??` 会吞掉 null。
    callId: partial.callId !== undefined ? partial.callId : "call-1",
    runId: partial.runId !== undefined ? partial.runId : "run-1",
    arguments: partial.arguments !== undefined ? partial.arguments : { question: "继续？" },
    status: partial.status
  };
}

function createPendingAsk(partial: Partial<PendingAsk> = {}): PendingAsk {
  return {
    requestId: partial.requestId ?? "ask-1",
    requestKind: partial.requestKind ?? "interaction",
    sessionId: partial.sessionId ?? "session-1",
    runId: partial.runId ?? "run-1",
    turnId: partial.turnId ?? "turn-1",
    callId: partial.callId ?? "call-1",
    descriptorSnapshotId: partial.descriptorSnapshotId ?? "snapshot-1",
    descriptorId: partial.descriptorId ?? "builtin:ask_user",
    finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
    policyDigest: partial.policyDigest ?? "policy-digest",
    nonce: partial.nonce ?? "nonce-1",
    version: partial.version ?? 1,
    expiresAtMs: partial.expiresAtMs ?? Date.now() + 60_000,
    state: partial.state ?? "pending",
    prompt: partial.prompt ?? "继续执行?",
    options: partial.options ?? null
  };
}

function createMergedToolCall(partial: Partial<MergedToolCall> = {}): MergedToolCall {
  return {
    id: partial.id ?? "tool-1",
    toolName: partial.toolName ?? "read_file",
    canonicalToolName: partial.canonicalToolName ?? "Read",
    displayNameZh: partial.displayNameZh ?? null,
    mergeKey: partial.mergeKey ?? "Read",
    description: partial.description ?? "",
    status: partial.status ?? "done",
    durationSeconds: partial.durationSeconds ?? null,
    count: partial.count ?? 1
  };
}

function createTurnBucket(partial: Partial<TurnBucket> = {}): TurnBucket {
  return {
    turnId: partial.turnId ?? "turn-1",
    user: partial.user ?? null,
    assistant: partial.assistant ?? null,
    tools: partial.tools ?? [],
    mergedTools: partial.mergedTools ?? []
  };
}

async function flushAsync() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

describe("PA-114 Stage 1 ask_user acceptance (F1)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // ── F1-1 ──────────────────────────────────────────────────────────────────

  it("F1-1 isAskToolName recognizes every ask tool spelling", () => {
    expect(isAskToolName("ask_user")).toBe(true);
    expect(isAskToolName("ask")).toBe(true);
    expect(isAskToolName("Ask")).toBe(true);
    expect(isAskToolName("builtin:ask")).toBe(true);
    expect(isAskToolName("builtin:ask_user")).toBe(true);
    expect(isAskToolName("read_file")).toBe(false);
    expect(isAskToolName("")).toBe(false);
  });

  // ── F1-2 ──────────────────────────────────────────────────────────────────

  it("F1-2 renders the question from the tool arguments (question first)", () => {
    const tool = createAskToolCall({ arguments: { question: "继续？" } });
    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    expect(wrapper.text()).toContain("继续？");
    wrapper.unmount();
  });

  it("F1-2 falls back to the text argument when question is absent", () => {
    const tool = createAskToolCall({ arguments: { text: "请确认操作" } });
    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    expect(wrapper.text()).toContain("请确认操作");
    wrapper.unmount();
  });

  it("F1-2 falls back to the prompt argument when question/text are absent", () => {
    const tool = createAskToolCall({ arguments: { prompt: "确认继续执行？" } });
    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    expect(wrapper.text()).toContain("确认继续执行？");
    wrapper.unmount();
  });

  it("F1-2 falls back to the matching pending ask prompt", () => {
    const store = useAskStore();
    store.pendingAsks = [createPendingAsk({ prompt: "请确认网络变更" })];
    const tool = createAskToolCall({ arguments: null });
    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    expect(wrapper.text()).toContain("请确认网络变更");
    wrapper.unmount();
  });

  // ── F1-3 ──────────────────────────────────────────────────────────────────

  it("F1-3 renders options, typed input, answer and cancel when a matching pending ask exists", () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-1",
        prompt: "继续？",
        options: ["是", "否"]
      })
    ];
    const tool = createAskToolCall({ callId: "call-1", runId: "run-1" });

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    expect(wrapper.get('[data-testid="ask-user-option"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="ask-user-typed-input"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="ask-user-answer"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="ask-user-cancel"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="ask-user-waiting"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("F1-3 clicking an option calls store.answer with the option value", async () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-1",
        prompt: "继续？",
        options: ["是", "否"]
      })
    ];
    const tool = createAskToolCall({ callId: "call-1", runId: "run-1" });
    const answerSpy = vi.spyOn(store, "answer").mockResolvedValue(true);

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    const optionButtons = wrapper.findAll('[data-testid="ask-user-option"]');
    expect(optionButtons.length).toBeGreaterThan(0);
    await optionButtons[0].trigger("click");
    await flushAsync();

    expect(answerSpy).toHaveBeenCalled();
    const [request, value] = answerSpy.mock.calls[0];
    expect(request.requestId).toBe("ask-1");
    expect(value).toBe("是");
    wrapper.unmount();
  });

  it("F1-3 renders the waiting state when no pending ask matches", () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({ requestId: "ask-other", callId: "call-other" })
    ];
    const tool = createAskToolCall({ callId: "call-1", runId: "run-1" });

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    expect(wrapper.get('[data-testid="ask-user-waiting"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="ask-user-option"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-answer"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-cancel"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("F1-3 disables interaction while the store is answering", async () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-1",
        prompt: "继续？",
        options: ["是", "否"]
      })
    ];
    store.answeringRequestId = "ask-1";
    const tool = createAskToolCall({ callId: "call-1", runId: "run-1" });

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    await wrapper.vm.$nextTick();

    const optionButtons = wrapper.findAll('[data-testid="ask-user-option"]');
    expect(optionButtons.length).toBeGreaterThan(0);
    expect(wrapper.get('[data-testid="ask-user-answer"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="ask-user-cancel"]').attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });

  // ── F1-4 ──────────────────────────────────────────────────────────────────

  it("F1-4 WorkspaceTurnItem routes ask tool rows to AskUserToolCallCard and keeps generic rows", () => {
    const askTool: MergedToolCall = createMergedToolCall({
      id: "tool-ask-1",
      toolName: "ask_user",
      canonicalToolName: "Ask",
      displayNameZh: "提问",
      mergeKey: "Ask"
    });
    const readTool: MergedToolCall = createMergedToolCall({
      id: "tool-read-1",
      toolName: "read_file",
      canonicalToolName: "Read",
      displayNameZh: "读取",
      mergeKey: "Read"
    });

    const events: AgentTurnEvent[] = [
      { kind: "tools", key: "tools-1", order: 1, tools: [askTool, readTool] }
    ];

    const wrapper = mount(WorkspaceTurnItem, {
      props: {
        turn: createTurnBucket(),
        events,
        shouldShowAgentArticle: true,
        rollbackInFlight: false,
        confirmRollback: () => {},
        setUserMessageRef: () => {},
        setAgentMessageRef: () => {},
        handleMarkdownRenderComplete: () => {},
        shouldUseMarkdownAssistantRendering: () => false,
        shouldUseOptimizedAssistantStreaming: () => false,
        streamingReasoningStable: () => "",
        streamingReasoningFade: () => "",
        assistantDisplayedReasoningFadeStyle: () => ({}),
        assistantDisplayedReasoningFadeKey: () => 0
      },
      global: {
        stubs: {
          AskUserToolCallCard: {
            name: "AskUserToolCallCard",
            template: '<div data-testid="ask-user-tool-card" />'
          }
        }
      }
    });

    // 只有 ask 工具行被路由到 AskUserToolCallCard。
    expect(wrapper.findAll('[data-testid="ask-user-tool-card"]')).toHaveLength(1);
    // 非 ask 行保持原通用渲染。
    const genericNames = wrapper.findAll(".conversation-tool-name").map((node) => node.text());
    expect(genericNames).toContain("读取");
    wrapper.unmount();
  });

  // ── 第二轮（PA-114 fix-round-2）：callId 投影绑定 + 终态 ────────────────────
  // 契约依据：`.dev-team/fix-round2-ask-user-frontend.md` T1-T4。

  it("T1 binds to the pending ask matching the tool callId among several", async () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-first",
        callId: "call-first",
        runId: "run-1",
        prompt: "第一问",
        options: ["甲"]
      }),
      createPendingAsk({
        requestId: "ask-second",
        callId: "call-second",
        runId: "run-1",
        prompt: "第二问",
        options: ["乙"]
      })
    ];
    const tool = createAskToolCall({
      callId: "call-second",
      runId: "run-1",
      arguments: null
    });

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    // 绑定第二个 ask：其选项/问题可见，第一个 ask 的选项/问题不可见。
    expect(wrapper.text()).toContain("第二问");
    expect(wrapper.text()).not.toContain("第一问");
    const optionTexts = wrapper.findAll('[data-testid="ask-user-option"]').map((node) => node.text());
    expect(optionTexts).toContain("乙");
    expect(optionTexts).not.toContain("甲");

    // 回答走绑定的那个 ask（requestId == ask-second）。
    const answerSpy = vi.spyOn(store, "answer").mockResolvedValue(true);
    await wrapper.get('[data-testid="ask-user-option"]').trigger("click");
    await flushAsync();
    expect(answerSpy).toHaveBeenCalled();
    expect(answerSpy.mock.calls[0][0].requestId).toBe("ask-second");
    wrapper.unmount();
  });

  it("T2 never binds any pending ask when the tool has no callId", () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-1",
        callId: "call-1",
        runId: "run-1",
        prompt: "第一问",
        options: ["是"]
      })
    ];
    const tool = createAskToolCall({ callId: null, runId: "run-1", arguments: null });

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    // callId 缺失绝不命中 pendingAsks[0]：无交互控件、不显示其 prompt。
    expect(wrapper.find('[data-testid="ask-user-option"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-typed-input"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-answer"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-cancel"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("第一问");
    // 未完成态：待命。
    expect(wrapper.get('[data-testid="ask-user-waiting"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("T3 renders a static terminal state when the tool is done and no pending ask matches", () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-1",
        callId: "call-1",
        runId: "run-1",
        prompt: "第一问"
      })
    ];
    const tool = createAskToolCall({
      callId: "call-other",
      runId: "run-1",
      status: "done",
      arguments: null
    });

    const wrapper = mount(AskUserToolCallCard, { props: { tool } });
    // 静态完成态：无 spinner、无交互控件、无待命态。
    expect(wrapper.find(".animate-spin").exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-option"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-typed-input"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-answer"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-cancel"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="ask-user-waiting"]').exists()).toBe(false);
    // 显示"已回答/已完成"完成态文案。
    expect(wrapper.text()).toMatch(/已回答|已完成/);
    wrapper.unmount();
  });

  it("T4 clears typedAnswer when the bound ask changes (callId switches)", async () => {
    const store = useAskStore();
    store.pendingAsks = [
      createPendingAsk({
        requestId: "ask-a",
        callId: "call-a",
        runId: "run-1",
        prompt: "问A"
      }),
      createPendingAsk({
        requestId: "ask-b",
        callId: "call-b",
        runId: "run-1",
        prompt: "问B"
      })
    ];
    const wrapper = mount(AskUserToolCallCard, {
      props: {
        tool: createAskToolCall({ callId: "call-a", runId: "run-1", arguments: null })
      }
    });

    await wrapper.get('[data-testid="ask-user-typed-input"]').setValue("草稿答案");
    expect(
      (wrapper.get('[data-testid="ask-user-typed-input"]').element as HTMLInputElement).value
    ).toBe("草稿答案");

    // 同组件内绑定切换（call-a → call-b）：草稿必须清空。
    await wrapper.setProps({
      tool: createAskToolCall({ callId: "call-b", runId: "run-1", arguments: null })
    });
    await wrapper.vm.$nextTick();
    expect(
      (wrapper.get('[data-testid="ask-user-typed-input"]').element as HTMLInputElement).value
    ).toBe("");
    wrapper.unmount();
  });
});

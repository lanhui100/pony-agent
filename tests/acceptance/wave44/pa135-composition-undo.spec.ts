/**
 * Wave-44 契约验收（L2-T / L2-AT）— PA-135：组合态（IME）下 Ctrl/Cmd+Z 撤销必须可达
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave44.md` §1 PA135-T1/T2、§2.2、§3。
 *
 * 缺陷事实（A9/A10/A11）：
 *   `src/components/HomeWorkspace.vue:1402` 的组合态守卫是无条件 `return`，位于 undo 分支之前，
 *   使 `:1406-1413` 的 `isUndoShortcut` 分支在组合态**完全不可达**。守卫的唯一必要服务对象是
 *   `:1416-1424` 的 Enter 提交（A10）。⇒ 中文/日文输入法选词时按 Ctrl+Z 撤销被静默丢弃。
 *
 * 覆盖（契约断言 id 对照）：
 *   PA135-T1（L2-T）：组合态（isComposing:true）Ctrl+Z ⇒ `checkoutHistoryNode` **1** 次、
 *     `preventDefault` 1 次（`defaultPrevented === true`）、`submitTurn` **0** 次；
 *     同一用例内追加防假修复正控：非组合态 Ctrl+Z 仍撤销 1 次、非组合态 Enter 仍提交 1 次。
 *   PA135-T2（L2-AT 对抗）：① 组合态 Ctrl+Shift+Z（带 shift 修饰）⇒ 撤销 **0** 次（修饰键约束保持）；
 *     ② 旧式 IME 标记 `keyCode:229` + Ctrl+Z ⇒ 撤销 **1** 次（与 isComposing 等价，修复前必红）；
 *     ③ 组合态 Enter 与 Shift+Enter ⇒ `submitTurn` **0** 次、`checkoutHistoryNode` 不增加。
 *   PA135-T3（L2-AT 对抗）：textarea 处于 `document.activeElement`（真实焦点）时，组合态 Cmd+Z（metaKey）
 *     ⇒ 撤销**恰好 1 次**且 `defaultPrevented === true`（不得经 `handleWindowKeydown` 双触发）。
 *
 * 断言层级：mount 真实 `HomeWorkspace`，对 `[data-testid="workspace-composer-input"]` 派发原生
 * `KeyboardEvent`（jsdom 26 真实透传 `isComposing`/`keyCode`），只断言**行为出口**
 * （`runtimeStore.checkoutHistoryNode` / `runtimeStore.submitTurn` 调用次数与 `defaultPrevented`），
 * 不断言分支顺序或组件内部变量（契约 §3.1）。
 *
 * 零假测试律：无 skip/todo/吞断言；`PA135-T1/T2/T3` 均含真实缺陷断言，故修复前必须全红。
 *
 * 作者：test-agent（L2-AT）。红相锚定后本文件冻结只读，任何修改须 Lead 批准。
 */
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, nextTick, watch } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";
import type { ChatMessage, HistoryBranch, HistoryNode } from "@/types/runtime";

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

// ── 夹具 ────────────────────────────────────────────────────────────────

function createMessage(partial: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? "turn-1",
    role: partial.role ?? "user",
    content: partial.content ?? "hello",
    status: partial.status ?? "done",
    tokenCount: null,
    reasoningContent: null,
    modelName: null,
    toolName: null,
    detail: null,
    durationSeconds: null,
    errorDetail: null
  };
}

function createHistoryNode(partial: Partial<HistoryNode> = {}): HistoryNode {
  return {
    nodeId: partial.nodeId ?? "node-1",
    sessionId: "session-current",
    parentNodeId: partial.parentNodeId ?? "node-root",
    branchId: partial.branchId ?? "branch-main",
    forkedFromNodeId: null,
    kind: "turn_committed",
    turnId: partial.turnId ?? null,
    transcriptRef: null,
    runRef: null,
    workspaceRef: { kind: "none", rollbackCapable: false },
    summary: partial.summary ?? "checkpoint summary",
    title: partial.title ?? "checkpoint title",
    history: [],
    turnTraceHistory: [],
    turnCount: 1,
    lastReferencedFile: null,
    createdAtMs: partial.createdAtMs ?? 1000
  };
}

function createHistoryBranch(partial: Partial<HistoryBranch> = {}): HistoryBranch {
  return {
    branchId: partial.branchId ?? "branch-main",
    sessionId: "session-current",
    baseNodeId: "node-1",
    headNodeId: partial.headNodeId ?? "node-1",
    forkedFromBranchId: null,
    forkedFromNodeId: null,
    label: "main",
    createdAtMs: 1000,
    updatedAtMs: 2000
  };
}

function createProviderRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-openai",
    providers: [
      {
        id: "provider-openai",
        name: "OpenAI",
        protocol: "openai",
        baseUrl: "https://api.openai.com/v1",
        apiKeyEnvVar: "OPENAI_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-gpt5",
        models: [
          {
            id: "model-gpt5",
            name: "GPT-5",
            model: "gpt-5",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "open-ai-reasoning",
            capabilities: {
              contextWindowTokens: 128000,
              supportsTools: true,
              supportsStreaming: true,
              supportsImageInput: false,
              supportsReasoning: true
            }
          }
        ]
      }
    ]
  } as unknown as ProviderRegistry;
}

// ── 挂载壳（复用 tests/HomeWorkspace.spec.ts 已验证的最小 stub 集）──────────

const scrollToBottomSpy = vi.fn();

const ScrollAreaStub = defineComponent({
  setup(_props, { slots, expose }) {
    const viewportEl = document.createElement("div");
    expose({ viewportEl, scrollToBottom: scrollToBottomSpy });
    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

const MarkdownRendererStub = defineComponent({
  props: {
    content: { type: String, default: "" },
    streaming: { type: Boolean, default: false },
    wrapperClass: { type: String, default: "" },
    toneClass: { type: String, default: "" },
    forceMarkdownStreaming: { type: Boolean, default: false }
  },
  emits: ["render-complete"],
  setup(props, { emit }) {
    watch(
      () => [props.content, props.streaming] as const,
      ([content, streaming]) => {
        window.setTimeout(() => {
          emit("render-complete", { contentLength: content.length, streaming });
        }, 0);
      },
      { immediate: true }
    );
    return () => h("div", { class: "markdown-stub" }, props.content);
  }
});

const ButtonStub = defineComponent({
  props: {
    disabled: { type: Boolean, default: false },
    title: { type: String, default: "" }
  },
  emits: ["click"],
  setup(props, { emit, slots }) {
    return () =>
      h(
        "button",
        {
          class: "button-stub",
          type: "button",
          disabled: props.disabled,
          title: props.title,
          onClick: () => emit("click")
        },
        slots.default ? slots.default() : []
      );
  }
});

function mountWorkspace(options: { attachToBody?: boolean } = {}) {
  const providerStore = useProviderStore();
  providerStore.$patch({ registry: createProviderRegistry(), selectedReasoningEffort: null });

  return mount(
    {
      render() {
        return h(TooltipProvider, null, { default: () => h(HomeWorkspace) });
      }
    },
    {
      ...(options.attachToBody ? { attachTo: document.body } : {}),
      global: {
        directives: {
          motion: { mounted() {}, updated() {}, beforeUpdate() {} }
        },
        stubs: {
          ScrollArea: ScrollAreaStub,
          MarkdownRenderer: MarkdownRendererStub,
          Button: ButtonStub,
          Transition: true,
          TransitionGroup: true
        }
      }
    }
  );
}

/** 灌入「已完成且可撤销」的会话状态（使 `canUndoLastTurn === true`）。 */
function seedUndoableState(options: { draft?: string } = {}) {
  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionOperation: null,
    sessionId: "session-current",
    phase: "ready",
    isSubmitting: false,
    error: null,
    activeBranchId: "branch-main",
    visibleNodeId: "node-1",
    branchHeadNodeId: "node-1",
    draftMessage: options.draft ?? "",
    messages: [
      createMessage({ id: "user-1", turnId: "turn-1", role: "user", content: "上一轮问题" }),
      createMessage({ id: "assistant-1", turnId: "turn-1", role: "assistant", content: "上一轮回答" })
    ],
    historyNodes: [
      createHistoryNode({
        nodeId: "node-1",
        parentNodeId: "node-root",
        turnId: "turn-1",
        summary: "turn-1 checkpoint",
        createdAtMs: 1000
      })
    ],
    historyBranches: [createHistoryBranch({ branchId: "branch-main", headNodeId: "node-1" })]
  });
  return runtimeStore;
}

// ── 事件构造（唯一入口，保证事件形状可审计）───────────────────────────────

function keydownEvent(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
}

async function dispatchComposerKeydown(
  wrapper: ReturnType<typeof mountWorkspace>,
  event: KeyboardEvent
): Promise<void> {
  wrapper.get('[data-testid="workspace-composer-input"]').element.dispatchEvent(event);
  await nextTick();
  await flushPromises();
  await nextTick();
}

/** 撤销链路探针：`runtimeStore.checkoutHistoryNode` 为撤销的公开行为出口（契约 A11）。 */
function spyRollbackAndSubmit(runtimeStore: ReturnType<typeof useRuntimeStore>) {
  const checkoutSpy = vi
    .spyOn(runtimeStore, "checkoutHistoryNode")
    .mockImplementation(async () => undefined as never);
  const submitSpy = vi
    .spyOn(runtimeStore, "submitTurn")
    .mockImplementation(async () => undefined as never);
  return { checkoutSpy, submitSpy };
}

describe("PA-135 · 组合态 Ctrl/Cmd+Z 撤销可达性", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    scrollToBottomSpy.mockClear();
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("PA135-T1: 组合态 Ctrl+Z 必须撤销（1 次）且阻止默认；非组合态撤销/发送正控保持", async () => {
    // Arrange
    const runtimeStore = seedUndoableState({ draft: "待撤回的草稿" });
    const { checkoutSpy, submitSpy } = spyRollbackAndSubmit(runtimeStore);
    const wrapper = mountWorkspace();
    await nextTick();
    checkoutSpy.mockClear();
    submitSpy.mockClear();

    // Act 1（缺陷断言）：组合态 Ctrl+Z
    const composingEvent = keydownEvent({ key: "z", ctrlKey: true, isComposing: true });
    await dispatchComposerKeydown(wrapper, composingEvent);

    // Assert 1：组合态不得吞掉撤销快捷键。
    expect(checkoutSpy).toHaveBeenCalledTimes(1);
    expect(checkoutSpy.mock.calls[0]?.[0]).toBe("node-root");
    expect(composingEvent.defaultPrevented).toBe(true);
    expect(submitSpy).not.toHaveBeenCalled();

    // Act 2（防假修复正控）：非组合态 Ctrl+Z 仍撤销。
    checkoutSpy.mockClear();
    const plainUndoEvent = keydownEvent({ key: "z", ctrlKey: true, isComposing: false });
    await dispatchComposerKeydown(wrapper, plainUndoEvent);

    // Assert 2：正控——撤销链路确实可被本文件的派发方式触发。
    expect(checkoutSpy).toHaveBeenCalledTimes(1);
    expect(plainUndoEvent.defaultPrevented).toBe(true);

    // Act 3（防假修复正控）：非组合态 Enter 仍提交。
    checkoutSpy.mockClear();
    submitSpy.mockClear();
    const plainEnterEvent = keydownEvent({ key: "Enter", isComposing: false });
    await dispatchComposerKeydown(wrapper, plainEnterEvent);

    // Assert 3：正控——发送链路确实可被本文件的派发方式触发。
    expect(submitSpy).toHaveBeenCalledTimes(1);
    expect(checkoutSpy).not.toHaveBeenCalled();
  });

  it("PA135-T2 (L2-AT 对抗): keyCode=229 组合态 Ctrl+Z 同样撤销；修饰键约束与组合态 Enter 不发送保持", async () => {
    // Arrange
    const runtimeStore = seedUndoableState({ draft: "草稿" });
    const { checkoutSpy, submitSpy } = spyRollbackAndSubmit(runtimeStore);
    const wrapper = mountWorkspace();
    await nextTick();
    checkoutSpy.mockClear();
    submitSpy.mockClear();

    // Act 1（修饰键约束）：组合态 Ctrl+Shift+Z 不得撤销。
    const shiftEvent = keydownEvent({ key: "z", ctrlKey: true, shiftKey: true, isComposing: true });
    await dispatchComposerKeydown(wrapper, shiftEvent);

    // Assert 1：`!shiftKey` 约束保持，撤销 0 次。
    expect(checkoutSpy).not.toHaveBeenCalled();
    expect(submitSpy).not.toHaveBeenCalled();

    // Act 2（缺陷断言）：旧式 IME 标记 keyCode=229 + Ctrl+Z。
    const legacyImeUndo = keydownEvent({ key: "z", ctrlKey: true, keyCode: 229 });
    await dispatchComposerKeydown(wrapper, legacyImeUndo);

    // Assert 2：keyCode=229 必须与 isComposing 等价（修复前必红）。
    expect(checkoutSpy).toHaveBeenCalledTimes(1);
    expect(legacyImeUndo.defaultPrevented).toBe(true);

    // Act 3 / Assert 3：组合态 Enter 与 Shift+Enter 不得发送。
    const checkoutCallsBeforeEnter = checkoutSpy.mock.calls.length;
    submitSpy.mockClear();
    await dispatchComposerKeydown(wrapper, keydownEvent({ key: "Enter", isComposing: true }));
    await dispatchComposerKeydown(
      wrapper,
      keydownEvent({ key: "Enter", shiftKey: true, isComposing: true })
    );

    expect(submitSpy).not.toHaveBeenCalled();
    expect(checkoutSpy.mock.calls.length).toBe(checkoutCallsBeforeEnter);
  });

  it("PA135-T3 (L2-AT 对抗): 输入框持焦时组合态 Cmd+Z 撤销恰好 1 次（不得经 window 监听双触发）", async () => {
    // Arrange：真实焦点（activeElement）让 window 级 handleWindowKeydown 也进入判定链路。
    const runtimeStore = seedUndoableState({ draft: "草稿" });
    const { checkoutSpy, submitSpy } = spyRollbackAndSubmit(runtimeStore);
    const wrapper = mountWorkspace({ attachToBody: true });
    await nextTick();
    const input = wrapper.get('[data-testid="workspace-composer-input"]');
    (input.element as HTMLTextAreaElement).focus();
    expect(document.activeElement).toBe(input.element);
    checkoutSpy.mockClear();
    submitSpy.mockClear();

    // Act：组合态 Cmd+Z（mac 路径）
    const cmdUndoEvent = keydownEvent({ key: "z", metaKey: true, isComposing: true });
    await dispatchComposerKeydown(wrapper, cmdUndoEvent);

    // Assert：恰好 1 次撤销（双触发会是 2 次），且阻止默认、不发送。
    expect(checkoutSpy).toHaveBeenCalledTimes(1);
    expect(cmdUndoEvent.defaultPrevented).toBe(true);
    expect(submitSpy).not.toHaveBeenCalled();
  });
});

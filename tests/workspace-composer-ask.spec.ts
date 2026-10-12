import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import { useAskStore } from "@/stores/ask";
import { useRuntimeStore } from "@/stores/runtime";
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

function createPendingAsk(partial: Partial<PendingAsk> = {}): PendingAsk {
  return {
    requestId: partial.requestId ?? "ask-1",
    requestKind: partial.requestKind ?? "interaction",
    sessionId: partial.sessionId ?? "session-test",
    runId: partial.runId ?? "run-1",
    turnId: partial.turnId ?? "turn-1",
    callId: partial.callId ?? "call-1",
    descriptorSnapshotId: partial.descriptorSnapshotId ?? "snapshot-1",
    descriptorId: partial.descriptorId ?? "builtin:ask",
    finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
    policyDigest: partial.policyDigest ?? "policy-digest",
    nonce: partial.nonce ?? "nonce-1",
    version: partial.version ?? 1,
    expiresAtMs: partial.expiresAtMs ?? Date.now() + 60_000,
    state: partial.state ?? "pending",
    prompt: partial.prompt ?? "请选择接下来的操作：",
    options: partial.options ?? null
  };
}

function mountComposer(pinia = createPinia()) {
  setActivePinia(pinia);
  return mount(WorkspaceComposer, {
    props: {
      showReasoningContent: false,
      canUndoLastTurn: false,
      undoShortcutLabel: "Ctrl+Z",
      handleComposerKeydown: () => {},
      handlePrimaryAction: () => {},
      handleUndoLastTurn: () => {},
      setComposerShellRef: () => {}
    },
    global: {
      plugins: [pinia],
      stubs: {
        TooltipRoot: { template: "<div><slot /></div>" },
        TooltipTrigger: { template: "<div><slot /></div>" },
        TooltipPortal: { template: "<div><slot /></div>" },
        TooltipContent: { template: "<div><slot /></div>" }
      }
    }
  });
}

describe("WorkspaceComposer integrated Ask panel UI/UX", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve(useAskStore().pendingAsks);
      }
      return Promise.resolve(null);
    });
  });

  it("renders standard input and action bar when there is NO pending ask", () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";

    const wrapper = mountComposer(pinia);

    // 标准输入框与 action 栏存在
    expect(wrapper.find('[data-testid="workspace-composer-input"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-attach-button"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-auth-mode-trigger"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="workspace-model-menu-trigger"]').exists()).toBe(true);

    // Ask 模式元素不应存在
    expect(wrapper.find('[data-testid="composer-ask-container"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("hides input and action bar, and displays question when pending ask exists", () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    const askStore = useAskStore(pinia);
    askStore.pendingAsks = [
      createPendingAsk({
        sessionId: "session-test",
        prompt: "你是否确认发布到线上环境？"
      })
    ];

    const wrapper = mountComposer(pinia);

    // 隐藏常规输入框与 action 工具栏
    expect(wrapper.find('[data-testid="workspace-composer-input"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-attach-button"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-auth-mode-trigger"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="workspace-model-menu-trigger"]').exists()).toBe(false);

    // 显示 Ask 容器与问题
    const container = wrapper.get('[data-testid="composer-ask-container"]');
    expect(container.classes().some((c) => c.startsWith("p-") || c.startsWith("py-") || c.startsWith("px-"))).toBe(true);

    const promptEl = wrapper.get('[data-testid="composer-ask-prompt"]');
    expect(promptEl.text()).toContain("你是否确认发布到线上环境？");
    expect(promptEl.classes()).toContain("font-semibold");

    // 极简风格校验：不得含有任何边框 class
    const classList = promptEl.classes().join(" ");
    expect(classList).not.toMatch(/\bborder\b|\bborder-/);
    wrapper.unmount();
  });

  it("renders vertical options 1, 2, ... with custom input as the last item", async () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    const askStore = useAskStore(pinia);
    askStore.pendingAsks = [
      createPendingAsk({
        sessionId: "session-test",
        prompt: "请选择架构方案：",
        options: ["方案 A: 微服务架构", "方案 B: 单体架构"]
      })
    ];

    const wrapper = mountComposer(pinia);

    const optionsContainer = wrapper.get('[data-testid="composer-ask-options"]');
    // 垂直排列
    expect(optionsContainer.classes()).toContain("flex-col");

    // 前两个为预设选项，带 1、2 序号
    const optionItems = wrapper.findAll('[data-testid^="composer-ask-option-item-"]');
    expect(optionItems).toHaveLength(2);
    expect(optionItems[0].text()).toContain("1");
    expect(optionItems[0].text()).toContain("方案 A: 微服务架构");
    expect(optionItems[1].text()).toContain("2");
    expect(optionItems[1].text()).toContain("方案 B: 单体架构");

    // 最后一项是用户可以输入的自定义选项
    const customInput = wrapper.get('[data-testid="composer-ask-custom-input"]');
    expect(customInput.exists()).toBe(true);

    // 验证第一条为推荐选项，带醒目“推荐”徽章
    const badge = wrapper.get('[data-testid="composer-ask-recommended-badge"]');
    expect(badge.text()).toBe("推荐");
    expect(wrapper.findAll('[data-testid="composer-ask-recommended-badge"]')).toHaveLength(1);

    // 验证每个选项下方都有弱显示的简明注释
    const descs = wrapper.findAll('[data-testid="composer-ask-option-desc"]');
    expect(descs).toHaveLength(2);
    expect(descs[0].text().length).toBeGreaterThan(0);
    expect(descs[1].text().length).toBeGreaterThan(0);

    // 验证自定义选项下方也有弱显示注释
    const customDesc = wrapper.get('[data-testid="composer-ask-custom-desc"]');
    expect(customDesc.text()).toContain("手动输入");

    // 验证无边框：选项与输入框没有 border 类
    for (const item of optionItems) {
      expect(item.classes().join(" ")).not.toMatch(/\bborder\b|\bborder-/);
    }
    expect(customInput.classes().join(" ")).not.toMatch(/\bborder\b|\bborder-/);
    wrapper.unmount();
  });

  it("submits selected predefined option using the reused send button", async () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    const askStore = useAskStore(pinia);
    const ask = createPendingAsk({
      requestId: "ask-p1",
      sessionId: "session-test",
      prompt: "请选择构建方式：",
      options: ["Webpack", "Vite"]
    });
    askStore.pendingAsks = [ask];
    const answerSpy = vi.spyOn(askStore, "answer").mockResolvedValue(true);

    const wrapper = mountComposer(pinia);

    // 点击第二个选项 (Vite)
    const secondOption = wrapper.get('[data-testid="composer-ask-option-item-1"]');
    await secondOption.trigger("click");

    // 复用的发送按钮应为启用态
    const sendBtn = wrapper.get('[data-testid="workspace-submit-action"]');
    expect(sendBtn.attributes("disabled")).toBeUndefined();

    // 点击发送键
    await sendBtn.trigger("click");

    expect(answerSpy).toHaveBeenCalledWith(ask, "Vite");
    wrapper.unmount();
  });

  it("submits custom input option on Enter key or send button click", async () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    const askStore = useAskStore(pinia);
    const ask = createPendingAsk({
      requestId: "ask-p2",
      sessionId: "session-test",
      prompt: "请选择构建方式：",
      options: ["Webpack", "Vite"]
    });
    askStore.pendingAsks = [ask];
    const answerSpy = vi.spyOn(askStore, "answer").mockResolvedValue(true);

    const wrapper = mountComposer(pinia);

    const customInput = wrapper.get('[data-testid="composer-ask-custom-input"]');
    await customInput.setValue("Rollup 打包");

    // 按 Enter 提交
    await customInput.trigger("keydown", { key: "Enter" });

    expect(answerSpy).toHaveBeenCalledWith(ask, "Rollup 打包");
    wrapper.unmount();
  });

  it("renders minimalist textarea when there are NO predefined options", async () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    const askStore = useAskStore(pinia);
    const ask = createPendingAsk({
      requestId: "ask-no-opt",
      sessionId: "session-test",
      prompt: "请提供您的数据库连接串：",
      options: null
    });
    askStore.pendingAsks = [ask];
    const answerSpy = vi.spyOn(askStore, "answer").mockResolvedValue(true);

    const wrapper = mountComposer(pinia);

    // 应该渲染 textarea，不应渲染选项列表
    expect(wrapper.find('[data-testid="composer-ask-options"]').exists()).toBe(false);
    const textarea = wrapper.get('[data-testid="composer-ask-textarea"]');
    expect(textarea.exists()).toBe(true);
    expect(textarea.classes().join(" ")).not.toMatch(/\bborder\b|\bborder-/);

    // 输入回答并按 Enter 提交
    await textarea.setValue("postgresql://localhost:5432/mydb");
    await textarea.trigger("keydown", { key: "Enter" });

    expect(answerSpy).toHaveBeenCalledWith(ask, "postgresql://localhost:5432/mydb");
    wrapper.unmount();
  });

  it("records error to trace when ask operation encounters an error", async () => {
    const pinia = createPinia();
    const runtimeStore = useRuntimeStore(pinia);
    runtimeStore.sessionId = "session-test";
    runtimeStore.activeTurnId = "turn-1";
    const askStore = useAskStore(pinia);
    const ask = createPendingAsk({
      requestId: "ask-err-1",
      turnId: "turn-1",
      callId: "call-1",
      sessionId: "session-test",
      prompt: "是否删除文件？",
      options: ["确认", "取消"]
    });
    askStore.pendingAsks = [ask];
    vi.spyOn(askStore, "answer").mockImplementation(async () => {
      askStore.error = "应答失败：连接超时";
      return null;
    });

    const wrapper = mountComposer(pinia);

    // 选中选项并提交
    const opt = wrapper.get('[data-testid="composer-ask-option-item-0"]');
    await opt.trigger("click");
    const sendBtn = wrapper.get('[data-testid="workspace-submit-action"]');
    await sendBtn.trigger("click");

    // 验证错误被记录到了 trace
    const turnTrace = runtimeStore.turnTraceHistory.find((t) => t.turnId === "turn-1");
    expect(turnTrace?.error).toBe("应答失败：连接超时");
    const traceEntry = runtimeStore.traceTimeline.find((e) => e.error?.includes("连接超时"));
    expect(traceEntry).toBeDefined();
    expect(traceEntry?.state).toBe("error");

    wrapper.unmount();
  });
});

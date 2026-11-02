/**
 * Wave-59 F3 冻结验收：streaming 闪跳（契约 §3，只读源
 * `.dev-team/contracts/wave-59-desktop-uiux.md`；根因 RC1/RC2/RC3/RC5）。
 *
 * 分层说明（关键）：
 * - 本文件做 **DOM 结构与节点身份** 断言（jsdom 有效）：同一 `.markdown-body`
 *   元素引用必须跨内容增长 / 跨 streaming→完成态切换保持不变；
 * - **真实布局/像素级** 的逐帧采样由 `tests/e2e/wave59-stream-stability.spec.ts`（F3-5）
 *   在真实 Chromium 里完成 —— jsdom 无布局，不做像素断言。
 *
 * 断言纪律：每条用例先证明「内容确实到达了 DOM」（文本/节点自检），再断言契约；
 * 避免「渲染为空 → 契约恰好满足」的假绿。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import { useRuntimeStore } from "@/stores/runtime";
import {
  installWorkspaceDomStubs,
  mountWorkspaceWithRealMarkdown,
  settleMarkdownRender,
  settleTicks
} from "./wave59-harness";

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

/** 合成 markdown 流片段：标题 / 段落 / 列表 / 代码块 / 结尾段落。 */
const MD_CHUNKS = [
  "## 流式标题\n\n",
  "第一段正文，包含中文与 English 混排的内容。\n\n",
  "- 列表项一\n",
  "- 列表项二\n\n",
  "```ts\nconst answer = 42;\n```\n\n",
  "结尾段落，用于验证内容增长过程中的承载节点身份。\n"
] as const;

const MD_PREFIXES = MD_CHUNKS.map((_chunk, index) => MD_CHUNKS.slice(0, index + 1).join(""));

const TAIL_NODE_SELECTORS = [
  ".mr-streaming-raw",
  ".streaming-unrendered-suffix",
  ".mr-streaming-char"
] as const;

function tailNodeProbe(root: Element) {
  return {
    raw: root.querySelectorAll(".mr-streaming-raw").length,
    suffix: root.querySelectorAll(".streaming-unrendered-suffix").length,
    char: root.querySelectorAll(".mr-streaming-char").length
  };
}

function mountStreamingMarkdown(content: string) {
  return mount(MarkdownRenderer, {
    props: {
      content,
      streaming: true,
      forceMarkdownStreaming: true,
      wrapperClass: "assistant-markdown"
    }
  });
}

async function patchAssistant(
  store: ReturnType<typeof useRuntimeStore>,
  content: string,
  status: "pending" | "done"
) {
  await store.$patch({
    messages: [
      {
        id: "user-f3",
        turnId: "turn-f3",
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
      },
      {
        id: "assistant-f3",
        turnId: "turn-f3",
        role: "assistant",
        content,
        status,
        tokenCount: null,
        reasoningContent: null,
        modelName: "OpenAI/GPT-5",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ],
    isSubmitting: status === "pending",
    phase: status === "pending" ? "calling_model" : "ready",
    activeTurnId: "turn-f3",
    activeTurnStartedAtMs: Date.now()
  });
}

describe("wave-59 F3：streaming 闪跳（DOM 结构与节点身份）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    // 释放节奏不属于本波范围（D-59-2 保留），这里放大释放速率，
    // 只为了在单测里确定性地看到「全部内容已到达展示层」，与逐字符节点断言无关。
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    installWorkspaceDomStubs();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("F3-1: streaming 期间不得出现 .mr-streaming-raw / .streaming-unrendered-suffix / .mr-streaming-char 尾巴节点", async () => {
    // Arrange
    const wrapper = mountStreamingMarkdown(MD_CHUNKS[0]!);
    const samples: Array<{ step: number; raw: number; suffix: number; char: number }> = [];

    // Act + Assert（每一步都同时在「setProps 后的同步帧」与「渲染落地帧」采样）
    for (let step = 0; step < MD_PREFIXES.length; step += 1) {
      await wrapper.setProps({ content: MD_PREFIXES[step]! });
      await nextTick();
      samples.push({ step, ...tailNodeProbe(wrapper.element) });
      await settleTicks(2, 24);
      samples.push({ step, ...tailNodeProbe(wrapper.element) });
    }

    // Assert（契约主体：任何采样点都不得出现未渲染尾巴节点）
    const violations = samples.filter((sample) => sample.raw + sample.suffix + sample.char > 0);
    expect(
      violations,
      `F3-1：streaming 期间不得以独立节点呈现未渲染尾巴/逐字符；违规采样：${JSON.stringify(violations)}`
    ).toHaveLength(0);

    // Assert（自检：内容确实渲染到了 DOM，排除「什么都没渲染」的假绿）
    await settleMarkdownRender();
    const body = wrapper.find(".markdown-body");
    expect(body.exists(), "F3-1 自检：最终必须存在 .markdown-body 承载节点").toBe(true);
    const finalText = body.text();
    expect(finalText).toContain("流式标题");
    expect(finalText).toContain("列表项二");
    expect(finalText).toContain("const answer = 42;");
    expect(finalText).toContain("结尾段落");

    wrapper.unmount();
  });

  it("F3-2a: streaming 首个渲染帧起，承载节点 .markdown-body 必须已存在（stale-while-render 前提）", async () => {
    // Arrange
    const wrapper = mountStreamingMarkdown(MD_CHUNKS[0]!);

    // Act（只推进到首个渲染帧，不等待异步解析落地）
    await nextTick();

    // Assert
    expect(
      wrapper.find(".markdown-body").exists(),
      "F3-2a：streaming 首帧必须已有 .markdown-body 承载节点；若此时不存在，承载节点只能在后续帧被创建/替换，身份不可能不变"
    ).toBe(true);

    wrapper.unmount();
  });

  it("F3-2b: 内容增长过程中 .markdown-body 承载节点身份不变（不得重建）", async () => {
    // Arrange
    const wrapper = mountStreamingMarkdown(MD_CHUNKS[0]!);
    await settleMarkdownRender();

    const initial = wrapper.find(".markdown-body");
    expect(initial.exists(), "F3-2b 自检：首个渲染周期后必须存在 .markdown-body").toBe(true);
    const carrier = initial.element;

    // Act + Assert（每个增长步：同步帧 + 落地帧都必须是同一个 DOM 引用）
    for (let step = 1; step < MD_PREFIXES.length; step += 1) {
      await wrapper.setProps({ content: MD_PREFIXES[step]! });
      await nextTick();
      const syncFrame = wrapper.find(".markdown-body");
      expect(
        syncFrame.exists() && syncFrame.element === carrier,
        `F3-2b：第 ${step} 步同步帧的 .markdown-body 被替换（应是同一个 DOM 引用）`
      ).toBe(true);

      await settleTicks(2, 24);
      const settledFrame = wrapper.find(".markdown-body");
      expect(
        settledFrame.exists() && settledFrame.element === carrier,
        `F3-2b：第 ${step} 步渲染落地后 .markdown-body 被重建（应是同一个 DOM 引用）`
      ).toBe(true);
    }

    expect(wrapper.find(".markdown-body").text()).toContain("结尾段落");

    wrapper.unmount();
  });

  it("F3-3a: MarkdownRenderer streaming→完成态切换，.markdown-body 身份不变且完成帧含完整内容", async () => {
    // Arrange
    const wrapper = mountStreamingMarkdown(MD_CHUNKS[0]!);
    await settleMarkdownRender();
    const streamingCarrier = wrapper.find(".markdown-body");
    expect(streamingCarrier.exists(), "F3-3a 自检：streaming 期间必须存在 .markdown-body").toBe(true);
    const carrierEl = streamingCarrier.element;

    // Act（把剩余内容一次性补全后结束 streaming）
    await wrapper.setProps({ content: MD_PREFIXES[MD_PREFIXES.length - 1]! });
    await settleMarkdownRender();
    const beforeComplete = wrapper.find(".markdown-body");
    expect(
      beforeComplete.exists() && beforeComplete.element === carrierEl,
      "F3-3a：补全内容期间 .markdown-body 不得被重建"
    ).toBe(true);

    await wrapper.setProps({ streaming: false });
    await settleMarkdownRender();

    // Assert（契约主体 1：完成态沿用同一承载节点）
    const completed = wrapper.find(".markdown-body");
    expect(completed.exists(), "F3-3a：完成帧必须存在 .markdown-body").toBe(true);
    expect(
      completed.element,
      "F3-3a：streaming→完成态切换不得替换 .markdown-body 子树（DOM 引用必须一致）"
    ).toBe(carrierEl);

    // Assert（契约主体 2：完成帧可见文本含完整内容，无「先清空再重绘」）
    const completedText = wrapper.text();
    expect(completedText).toContain("流式标题");
    expect(completedText).toContain("第一段正文");
    expect(completedText).toContain("列表项二");
    expect(completedText).toContain("const answer = 42;");
    expect(completedText).toContain("结尾段落");

    wrapper.unmount();
  });

  it("F3-3b: 生产路径（HomeWorkspace）streaming→完成态切换，正文承载节点 .markdown-body 身份不变且完成帧含完整内容", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    await patchAssistant(store, "hello world 你好", "pending");

    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(6, 24);

    // Assert（自检 + F3-3 前半：流式期间正文承载节点必须已是 .markdown-body）
    const streamingBody = wrapper.find('[data-testid="workspace-content-column"] .markdown-body');
    expect(
      streamingBody.exists(),
      "F3-3b：流式期间正文必须由 .markdown-body 承载；否则完成态只能新建承载节点 ⇒ 子树被替换（RC5 闪跳）"
    ).toBe(true);
    const carrierEl = streamingBody.element;

    // Act（结束 streaming：assistant 转 done）
    await patchAssistant(store, "hello world 你好", "done");
    await settleMarkdownRender();

    // Assert（契约主体 1：完成态沿用同一承载节点）
    const completedBody = wrapper.find('[data-testid="workspace-content-column"] .markdown-body');
    expect(completedBody.exists(), "F3-3b：完成帧必须存在 .markdown-body 承载节点").toBe(true);
    expect(
      completedBody.element,
      "F3-3b：streaming→完成态切换不得替换正文承载节点（DOM 引用必须一致）"
    ).toBe(carrierEl);

    // Assert（契约主体 2：完成帧可见文本含完整内容）
    expect(completedBody.text()).toContain("hello world 你好");

    wrapper.unmount();
  });

  it("F3-4: WorkspaceTurnItem 内容区不得出现 .assistant-streaming-char 逐字符节点，流式与完成态共用同一容器", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    await patchAssistant(store, "hello", "pending");

    const wrapper = mountWorkspaceWithRealMarkdown(pinia);
    await settleTicks(4, 24);

    // Act + Assert（内容增长过程中反复采样逐字符节点）
    const charCounts: number[] = [];
    for (const content of ["hello world", "hello world 你好", "hello world 你好，正在流式输出。"]) {
      await patchAssistant(store, content, "pending");
      await settleTicks(3, 24);
      charCounts.push(wrapper.findAll(".assistant-streaming-char").length);
    }

    expect(
      charCounts,
      `F3-4：WorkspaceTurnItem 内容区不得出现 .assistant-streaming-char 逐字符节点（各采样点实际数量：${JSON.stringify(charCounts)}）`
    ).toEqual([0, 0, 0]);

    // Assert（共用容器：完成态必须沿用流式期间的正文承载节点，RC5 的 v-if/v-else 子树替换不得复现）
    const streamingBody = wrapper.find('[data-testid="workspace-content-column"] .markdown-body');
    expect(
      streamingBody.exists(),
      "F3-4：流式期间正文必须由 .markdown-body 承载（与完成态共用同一容器结构）"
    ).toBe(true);
    const carrierEl = streamingBody.element;

    await patchAssistant(store, "hello world 你好，正在流式输出。", "done");
    await settleMarkdownRender();

    const bodyAfter = wrapper.find('[data-testid="workspace-content-column"] .markdown-body');
    expect(bodyAfter.exists(), "F3-4：完成态必须存在 .markdown-body 承载节点").toBe(true);
    expect(
      bodyAfter.element,
      "F3-4：流式与完成态必须共用同一 DOM 容器（承载节点被整棵替换即为 RC5 闪跳）"
    ).toBe(carrierEl);

    wrapper.unmount();
  });
});

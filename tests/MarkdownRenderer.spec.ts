/**
 * MarkdownRenderer 渲染契约单测。
 *
 * wave-59 F3 契约演进（Lead 授权迁移，原意图不变、仅迁移断言锚点）：
 *   「未渲染尾巴以独立块级节点（.mr-streaming-raw / .streaming-unrendered-suffix）呈现」
 *   「逐字符 span（.mr-streaming-char）」的原断言，迁移为 F3-1：
 *   streaming 期间**不得**出现这三类节点；原用例对「内容最终完整渲染」的语义断言全部保留。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";

async function flushStreamingRender(waitMs = 220) {
  await new Promise<void>((resolve) => window.setTimeout(resolve, waitMs));
  await nextTick();
}

/** F3-1：streaming 期间不得出现的未渲染尾巴/逐字符节点（三类）。 */
function streamingTailNodes(wrapper: ReturnType<typeof mount>) {
  return {
    raw: wrapper.findAll(".mr-streaming-raw").length,
    suffix: wrapper.findAll(".streaming-unrendered-suffix").length,
    char: wrapper.findAll(".mr-streaming-char").length
  };
}

async function waitForMarkdownBody(wrapper: ReturnType<typeof mount>) {
  await vi.waitFor(() => {
    expect(wrapper.find(".markdown-body").exists()).toBe(true);
  }, { timeout: 5000, interval: 40 });
  await nextTick();
}

describe("MarkdownRenderer", () => {
  beforeEach(() => {
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders streaming partial markdown with auto-closed fenced code blocks", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: [
          "```ts",
          "const answer = 42;"
        ].join("\n"),
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender();

    expect(wrapper.html()).toContain('<pre class="code-block-cream"><code');
    expect(wrapper.text()).toContain("const answer = 42;");
    expect(wrapper.find(".streaming-unrendered-suffix").exists()).toBe(false);
  });

  it("uses plain text fast path for simple streaming content without markdown syntax", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "hello",
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await nextTick();

    // 纯文本快路径：直接显示在 .markdown-body 中，跳过 markdown 解析和 v-html 替换
    expect(wrapper.find(".markdown-body").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").text()).toBe("hello");
    expect(wrapper.find(".streaming-unrendered-suffix").exists()).toBe(false);
  });

  it("emits render-complete for plain text streaming content once additional content arrives", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "hello",
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await wrapper.setProps({ content: "hello world with more plain text arriving" });
    await nextTick();

    const events = wrapper.emitted("render-complete") ?? [];
    expect(events.length).toBeGreaterThanOrEqual(1);
    const lastEvent = events[events.length - 1]![0];
    expect(lastEvent.contentLength).toBe("hello world with more plain text arriving".length);
    expect(lastEvent.streaming).toBe(true);
  });

  it("uses markdown parser when streaming content has syntax", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "**bold text** and `code`",
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender();

    // 含有 markdown 语法的内容走正常解析路径
    expect(wrapper.find(".markdown-body").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>");
    expect(wrapper.find(".markdown-body").text()).toContain("bold text");
  });

  it("keeps rendering new markdown content while forceMarkdownStreaming is active", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "hello",
        streaming: true,
        forceMarkdownStreaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").text()).toBe("hello");

    await wrapper.setProps({ content: "hello **world**" });
    // F3-1：新增字符不得以独立尾巴节点/逐字符 span 呈现（不新起块、不逐字重播）
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>world</strong>");
  });

  it("can force the markdown streaming path for simple text", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "hello world",
        streaming: true,
        forceMarkdownStreaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender();

    expect(wrapper.find(".markdown-body").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").html()).toContain("<p>hello world</p>");

    await wrapper.setProps({ streaming: false });
    await waitForMarkdownBody(wrapper);

    expect(wrapper.find(".markdown-body").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").html()).toContain("<p>hello world</p>");
  });

  it("renders markdown incrementally while forceMarkdownStreaming is active", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "**bold text**",
        streaming: true,
        forceMarkdownStreaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await nextTick();
    // F3-1：首个 partial render 前不再以独立块级节点做「原始文本回退」
    // （D-59-1 stale-while-render：未渲染尾巴不单独成块，文字允许滞后 ≤1 个 render 周期）。
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>bold text</strong>");
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });

    await wrapper.setProps({ streaming: false });
    await waitForMarkdownBody(wrapper);
    // 流式结束后保留最终 markdown HTML。
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>bold text</strong>");
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });
  });

  it("renders the streaming suffix without any separate unrendered-suffix node (F3-1)", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "abcd",
        streaming: true,
        forceMarkdownStreaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    // wave-59 F3-1 迁移：原断言「尾巴以逐字符 span 呈现（4 个字节点）」改为
    // 「不得出现逐字符/独立尾巴节点」，同时保留「内容最终完整可见」的原语义。
    await nextTick();
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").text()).toBe("abcd");
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });

    // 解析完成后追加的新内容：同样不得以尾部字符层呈现，须在下一个 render 周期合并进 HTML。
    await wrapper.setProps({ content: "abcdefgh" });
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").text()).toBe("abcdefgh");
    expect(streamingTailNodes(wrapper)).toEqual({ raw: 0, suffix: 0, char: 0 });
  });

  /**
   * A-59-1 回归（契约 §7，审查 review-join-wave59 CONCURRENCY）：
   * `scheduleNonStreamingRender` 在「内容空白」时提前 return，却已写入 `renderInFlightVersion`；
   * 该在飞标记只有 `executeRender` 收尾会清 → 之后同实例的流式更新被 `scheduleStreamingRender`
   * 只记为 dirty 并永久吞掉（补排闭环断裂，正文永远停在旧快照）。
   *
   * 判据：清空后再次喂入流式内容，必须在**合理时限内**渲染出新内容（条件等待，禁止盲等）。
   */
  it("A-59-1: 非流式空白内容之后，同一实例的流式内容仍必须被渲染（在飞标记不得泄漏）", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "**旧内容**",
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender(200);
    expect(wrapper.find(".markdown-body").html()).toContain("旧内容");

    // 触发空白内容的非流式排程路径（历史上在此泄漏在飞标记）
    await wrapper.setProps({ content: "", streaming: false });
    await nextTick();
    await flushStreamingRender(60);

    // 再次流式输出新内容
    await wrapper.setProps({ content: "**新内容**", streaming: true });
    await nextTick();

    await vi.waitFor(
      () => {
        expect(
          wrapper.find(".markdown-body").html(),
          "A-59-1：在飞标记泄漏会切断补排闭环 —— 正文停在旧快照（新内容永不渲染）"
        ).toContain("新内容");
      },
      { timeout: 3000, interval: 20 }
    );

    wrapper.unmount();
  });

  it("transitions from plain text streaming to final markdown render when streaming ends", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "hello world",
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await nextTick();

    // 流式时为纯文本
    expect(wrapper.find(".markdown-body").text()).toBe("hello world");

    await wrapper.setProps({ streaming: false });
    await flushStreamingRender();
    await vi.waitFor(() => {
      const events = wrapper.emitted("render-complete") ?? [];
      expect(events.some((event) => event[0]?.streaming === false)).toBe(true);
    }, { timeout: 5000, interval: 40 });
    await nextTick();

    // 完成后转为 markdown 渲染（"hello world" 会被 marked 包装为 <p>hello world</p>）
    expect(wrapper.find(".markdown-body").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").text()).toContain("hello world");
    // 验证 render-complete 事件在 streaming=false 时也发出
    const events = wrapper.emitted("render-complete") ?? [];
    const finalEvent = events.find((e) => e[0]?.streaming === false);
    expect(finalEvent).toBeDefined();
  });

  it("keeps the previous rendered markdown visible while final rendering is queued", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "**bold text**",
        streaming: true,
        forceMarkdownStreaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>bold text</strong>");

    await wrapper.setProps({ streaming: false });
    await waitForMarkdownBody(wrapper);

    expect(wrapper.find(".markdown-body").html()).toContain("<strong>bold text</strong>");
  });

  it("re-renders the final truthful markdown when streaming ends", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: [
          "```ts",
          "const answer = 42;"
        ].join("\n"),
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await flushStreamingRender();
    await wrapper.setProps({ streaming: false });
    await flushStreamingRender();
    await vi.waitFor(() => {
      const events = wrapper.emitted("render-complete") ?? [];
      expect(events.some((event) => event[0]?.streaming === false)).toBe(true);
    }, { timeout: 5000, interval: 40 });

    const renderCompleteEvents = wrapper.emitted("render-complete") ?? [];
    expect(renderCompleteEvents.some((event) => event[0]?.streaming === false)).toBe(true);
    expect(wrapper.find(".streaming-unrendered-suffix").exists()).toBe(false);
  });
});

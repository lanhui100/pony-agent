import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";

async function flushStreamingRender(waitMs = 220) {
  await new Promise<void>((resolve) => window.setTimeout(resolve, waitMs));
  await nextTick();
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
    expect(wrapper.findAll(".mr-streaming-char").map((node) => node.text()).join("")).toBe("**world**");

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
    // 首次 partial render 前以原始文本作为可见回退。
    expect(wrapper.find(".streaming-unrendered-suffix").exists()).toBe(false);
    expect(wrapper.find(".mr-streaming-raw").exists()).toBe(true);
    expect(wrapper.text()).toBe("**bold text**");

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>bold text</strong>");
    expect(wrapper.find(".mr-streaming-raw").exists()).toBe(false);

    await wrapper.setProps({ streaming: false });
    await waitForMarkdownBody(wrapper);
    // 流式结束后保留最终 markdown HTML。
    expect(wrapper.find(".markdown-body").html()).toContain("<strong>bold text</strong>");
    expect(wrapper.find(".mr-streaming-raw").exists()).toBe(false);
  });

  it("shows only the unrendered streaming suffix as individual characters", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "abcd",
        streaming: true,
        forceMarkdownStreaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await nextTick();
    expect(wrapper.findAll(".mr-streaming-char")).toHaveLength(4);

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").text()).toBe("abcd");
    expect(wrapper.findAll(".mr-streaming-char")).toHaveLength(0);

    // 解析完成后追加的新内容先以尾部字符层显示，下一轮 partial render 合并回 HTML。
    await wrapper.setProps({ content: "abcdefgh" });
    const charsAfter = wrapper.findAll(".mr-streaming-char");
    expect(charsAfter).toHaveLength(4);
    expect(charsAfter[0]!.text()).toBe("e");
    expect(charsAfter[3]!.text()).toBe("h");

    await flushStreamingRender();
    expect(wrapper.find(".markdown-body").text()).toBe("abcdefgh");
    expect(wrapper.findAll(".mr-streaming-char")).toHaveLength(0);
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

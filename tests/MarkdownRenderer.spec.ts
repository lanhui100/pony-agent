import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";

async function flushStreamingRender(waitMs = 220) {
  await new Promise<void>((resolve) => window.setTimeout(resolve, waitMs));
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

    expect(wrapper.html()).toContain("<pre><code");
    expect(wrapper.text()).toContain("const answer = 42;");
    expect(wrapper.find(".streaming-unrendered-suffix").exists()).toBe(false);
  });

  it("shows the unrendered suffix while a small streaming fragment stays below the render threshold", async () => {
    const wrapper = mount(MarkdownRenderer, {
      props: {
        content: "hello",
        streaming: true,
        wrapperClass: "assistant-markdown"
      }
    });

    await nextTick();

    expect(wrapper.find(".streaming-unrendered-suffix").text()).toBe("hello");
    expect(wrapper.find(".markdown-body").exists()).toBe(false);
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

    const renderCompleteEvents = wrapper.emitted("render-complete") ?? [];
    expect(renderCompleteEvents.some((event) => event[0]?.streaming === false)).toBe(true);
    expect(wrapper.find(".streaming-unrendered-suffix").exists()).toBe(false);
  });
});

import { computed, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStreamingPresentationState } from "@/lib/useStreamingPresentationState";
import type { ChatMessage } from "@/types/runtime";

function createAssistantMessage(content: string, status: ChatMessage["status"] = "pending"): ChatMessage {
  return {
    id: "assistant-1",
    turnId: "turn-1",
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
  };
}

describe("useStreamingPresentationState", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("releases four chars on first sync for content longer than release rate", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello world this is a test")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("hell");
  });

  it("releases the next four chars on second sync", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("abcdefgh")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(8);
  });

  it("paces large content with multiple sync calls", () => {
    const content = "x".repeat(50);
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(8);

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(12);

    // 多次同步后应渐进释放，始终是前缀
    for (let i = 0; i < 10; i++) {
      state.syncStreamingPresentationState();
    }
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content);
  });

  it("releases short content in one sync when it fits the release rate", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hi")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("hi");
  });

  it("releases all content immediately when message is done", () => {
    const content = "hello this is a longer streamed assistant delta";
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    // 先同步一次（pending），释放 4 个字符
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);

    // 改为 done
    messages.value = [createAssistantMessage(content, "done")];
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content);
  });

  it.each(["done", "error", "cancelled"] as const)(
    "reveals all content when assistant becomes %s",
    (status) => {
      const content = "hello this is a longer streamed assistant delta";
      const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
      const state = useStreamingPresentationState(computed(() => messages.value));

      state.syncStreamingPresentationState();
      expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);

      messages.value = [createAssistantMessage(content, status)];
      state.syncStreamingPresentationState();

      expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content);
    }
  );

  it("handles content truncation gracefully (resets to shorter text)", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello world")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);

    // 内容被截断（极小概率 reset）
    messages.value = [createAssistantMessage("hi")];
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("hi");
  });

  it("cleans up state when message is removed", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello world", "pending")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);

    // 移除消息
    messages.value = [];
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(null)).toBe("");
  });

  it("handles multiple pending messages independently", () => {
    const msg1 = { ...createAssistantMessage("hello world"), id: "a1" };
    const msg2 = { ...createAssistantMessage("short"), id: "a2" };
    const messages = ref<ChatMessage[]>([msg1, msg2]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(msg1)).toHaveLength(4);
    expect(state.assistantDisplayContent(msg2)).toHaveLength(4);

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(msg1)).toHaveLength(8);
    expect(state.assistantDisplayContent(msg2)).toBe("short");
  });

  it("supports custom release rate via localStorage", () => {
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "2");
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello world")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(2);

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(4);
  });

  it("reveals all pending text immediately when reduced motion is requested", () => {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: true }));
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello world")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("hello world");
  });

  it("ignores non-assistant messages", () => {
    const userMsg: ChatMessage = {
      id: "user-1",
      turnId: "turn-1",
      role: "user",
      content: "hello",
      status: "done",
      tokenCount: null,
      reasoningContent: null,
      modelName: null,
      toolName: null,
      detail: null,
      durationSeconds: null,
      errorDetail: null
    };
    const messages = ref<ChatMessage[]>([userMsg]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(userMsg)).toBe("hello");
    expect(state.assistantDisplayStableContent(userMsg)).toBe("hello");
  });
});

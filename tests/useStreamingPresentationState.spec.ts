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
    vi.setSystemTime(new Date("2026-07-05T00:00:00.000Z"));
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("buffers the first short pending batch until the time threshold", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("");

    vi.advanceTimersByTime(119);
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("");

    vi.advanceTimersByTime(1);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("hello");
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("hello");
  });

  it("does not restart the first short fade animation while content is unchanged", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    vi.advanceTimersByTime(250);
    state.syncStreamingPresentationState();
    const firstFadeKey = state.assistantDisplayFadeKey(messages.value[0]!);
    vi.advanceTimersByTime(120);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("hello");
    expect(state.assistantDisplayFadeKey(messages.value[0]!)).toBe(firstFadeKey);
  });

  it("limits a large provider chunk to paced reveal batches", () => {
    const content = "hello this is a longer streamed assistant delta";
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe(content.slice(0, 12));
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content.slice(0, 12));

    vi.advanceTimersByTime(89);
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content.slice(0, 12));

    vi.advanceTimersByTime(1);
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content);
  });

  it("paces a 500-character delta instead of revealing it as one block", () => {
    const content = "x".repeat(500);
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(12);

    vi.advanceTimersByTime(90);
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(60);

    vi.advanceTimersByTime(90);
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(108);
  });

  it("paces long inline markdown that can be auto-closed safely", () => {
    const content = `**bold** ${"x".repeat(500)}`;
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(12);

    vi.advanceTimersByTime(90);
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toHaveLength(60);
  });

  it("keeps the previous faded batch visible while the next suffix is buffered", () => {
    const initial = "hello world!";
    const next = `${initial}!`;
    const messages = ref<ChatMessage[]>([createAssistantMessage(initial)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    messages.value = [createAssistantMessage(next)];
    vi.advanceTimersByTime(100);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe(initial);
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(initial);
  });

  it("flushes pending text when the time fallback threshold is reached", () => {
    const initial = "hello world!";
    const next = `${initial}!`;
    const messages = ref<ChatMessage[]>([createAssistantMessage(initial)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    messages.value = [createAssistantMessage(next)];
    state.syncStreamingPresentationState();
    vi.advanceTimersByTime(250);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe(initial);
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("!");
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(next);
  });

  it("uses the narrower batch threshold while an unclosed code fence is active", () => {
    const content = [
      "```ts",
      "const answer = 42;",
      "console.log(answer);"
    ].join("\n");
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("```ts\n");
    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
  });

  it("uses local threshold overrides for streaming reveal cadence", () => {
    window.localStorage.setItem("pony-agent.stream-render.first-batch-chars", "5");
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("hello");
    messages.value = [createAssistantMessage("hello!")];
    vi.advanceTimersByTime(100);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("hello");
  });

  it.each(["done", "error", "cancelled"] as const)(
    "reveals a hidden suffix immediately when the assistant becomes %s",
    (status) => {
    const content = "hello this is a longer streamed assistant delta";
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content.slice(0, 12));

    messages.value = [createAssistantMessage(content, status)];
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe(content);
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content);
    }
  );

  it("reveals pending text immediately when reduced motion is requested", () => {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({
      matches: true,
      media: "(prefers-reduced-motion: reduce)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn()
    }));
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("hello");
  });
});

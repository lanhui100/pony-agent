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
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("reveals the first short pending batch immediately through fade", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("hello");
  });

  it("does not restart the first short fade animation while content is unchanged", () => {
    const messages = ref<ChatMessage[]>([createAssistantMessage("hello")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    const firstFadeKey = state.assistantDisplayFadeKey(messages.value[0]!);
    vi.advanceTimersByTime(120);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("hello");
    expect(state.assistantDisplayFadeKey(messages.value[0]!)).toBe(firstFadeKey);
  });

  it("reveals the first pending batch through fade when the first threshold is reached", () => {
    const content = "hello this is a longer streamed assistant delta";
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe(content);
  });

  it("commits previously revealed text to stable content after a non-flush update", () => {
    const initial = "hello this is a longer streamed assistant delta";
    const next = `${initial}!`;
    const messages = ref<ChatMessage[]>([createAssistantMessage(initial)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    messages.value = [createAssistantMessage(next)];
    vi.advanceTimersByTime(100);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe(initial);
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("");
  });

  it("flushes pending text when the time fallback threshold is reached", () => {
    const initial = "hello this is a longer streamed assistant delta";
    const next = `${initial}!`;
    const messages = ref<ChatMessage[]>([createAssistantMessage(initial)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    state.syncStreamingPresentationState();
    messages.value = [createAssistantMessage(next)];
    vi.advanceTimersByTime(430);
    state.syncStreamingPresentationState();

    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe(initial);
    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe("!");
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

    expect(state.assistantDisplayFadeContent(messages.value[0]!)).toBe(content);
    expect(state.assistantDisplayStableContent(messages.value[0]!)).toBe("");
  });
});

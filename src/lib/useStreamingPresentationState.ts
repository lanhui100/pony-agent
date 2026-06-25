import { shallowReactive, type ComputedRef } from "vue";
import type { ChatMessage } from "@/types/runtime";

const STREAM_FADE_MIN_CHARS = 3;

export function useStreamingPresentationState(messages: ComputedRef<ChatMessage[]>) {
  const streamSnapshotTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamSnapshotReasoningByMessageId = shallowReactive<Record<string, string>>({});
  const streamFadeTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamFadeKeyByMessageId = shallowReactive<Record<string, number>>({});
  const streamReasoningFadeTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamReasoningFadeKeyByMessageId = shallowReactive<Record<string, number>>({});

  const PRESENTATION_MAPS = [
    streamSnapshotTextByMessageId,
    streamSnapshotReasoningByMessageId,
    streamFadeTextByMessageId,
    streamFadeKeyByMessageId,
    streamReasoningFadeTextByMessageId,
    streamReasoningFadeKeyByMessageId
  ];

  function syncPresentationMapValue<T extends string | number>(
    map: Record<string, T>,
    messageId: string,
    nextValue: T
  ) {
    if (map[messageId] !== nextValue) {
      map[messageId] = nextValue;
    }
  }

  function syncStreamingPresentationState() {
    const activeMessageIds = new Set<string>();
    const pendingAssistants: ChatMessage[] = [];

    for (const message of messages.value) {
      if (message.role !== "assistant") continue;
      activeMessageIds.add(message.id);
      if (message.status === "pending") {
        pendingAssistants.push(message);
      }
    }

    if (pendingAssistants.length === 0) {
      for (const map of PRESENTATION_MAPS) {
        for (const id of Object.keys(map)) {
          if (!activeMessageIds.has(id)) delete map[id];
        }
      }
      return;
    }

    for (const message of pendingAssistants) {
      const nextReasoning = message.reasoningContent ?? "";
      const nextText = message.content;
      const previousText = streamSnapshotTextByMessageId[message.id] ?? "";
      const previousReasoning = streamSnapshotReasoningByMessageId[message.id] ?? "";
      const hasOtherStreamingTextSnapshot = Object.keys(streamSnapshotTextByMessageId).some((id) => id !== message.id);
      const hasOtherStreamingReasoningSnapshot = Object.keys(streamSnapshotReasoningByMessageId).some((id) => id !== message.id);

      const appendedText = previousText.length > 0
        ? (nextText.length > previousText.length ? nextText.slice(previousText.length) : "")
        : (hasOtherStreamingTextSnapshot ? nextText : "");
      syncPresentationMapValue(streamFadeTextByMessageId, message.id, appendedText.length >= STREAM_FADE_MIN_CHARS ? appendedText : "");
      if (streamFadeTextByMessageId[message.id]) {
        streamFadeKeyByMessageId[message.id] = (streamFadeKeyByMessageId[message.id] ?? 0) + 1;
      }

      const appendedReasoning = previousReasoning.length > 0
        ? (nextReasoning.length > previousReasoning.length ? nextReasoning.slice(previousReasoning.length) : "")
        : (hasOtherStreamingReasoningSnapshot ? nextReasoning : "");
      syncPresentationMapValue(
        streamReasoningFadeTextByMessageId,
        message.id,
        appendedReasoning.length >= STREAM_FADE_MIN_CHARS ? appendedReasoning : ""
      );
      if (streamReasoningFadeTextByMessageId[message.id]) {
        streamReasoningFadeKeyByMessageId[message.id] = (streamReasoningFadeKeyByMessageId[message.id] ?? 0) + 1;
      }

      syncPresentationMapValue(streamSnapshotTextByMessageId, message.id, nextText);
      syncPresentationMapValue(streamSnapshotReasoningByMessageId, message.id, nextReasoning);
    }

    for (const map of PRESENTATION_MAPS) {
      for (const id of Object.keys(map)) {
        if (!activeMessageIds.has(id)) delete map[id];
      }
    }
  }

  function assistantDisplayContent(message: ChatMessage | null) {
    return message?.content ?? "";
  }

  function assistantDisplayStableContent(message: ChatMessage | null) {
    if (!message) return "";
    const displayText = assistantDisplayContent(message);
    const fadeText = streamFadeTextByMessageId[message.id] ?? "";
    return fadeText ? displayText.slice(0, Math.max(0, displayText.length - fadeText.length)) : displayText;
  }

  function assistantDisplayFadeContent(message: ChatMessage | null) {
    return message ? (streamFadeTextByMessageId[message.id] ?? "") : "";
  }

  function assistantDisplayFadeStyle(message: ChatMessage | null) {
    if (!message) return undefined;
    const key = streamFadeKeyByMessageId[message.id] ?? 0;
    return {
      animationName: key % 2 === 0 ? "assistant-stream-fade-in-a" : "assistant-stream-fade-in-b"
    };
  }

  function assistantDisplayedReasoning(message: ChatMessage | null) {
    return message?.reasoningContent ?? "";
  }

  function assistantDisplayedReasoningStable(message: ChatMessage | null) {
    if (!message) return "";
    const displayText = assistantDisplayedReasoning(message);
    const fadeText = streamReasoningFadeTextByMessageId[message.id] ?? "";
    return fadeText ? displayText.slice(0, Math.max(0, displayText.length - fadeText.length)) : displayText;
  }

  function assistantDisplayedReasoningFade(message: ChatMessage | null) {
    return message ? (streamReasoningFadeTextByMessageId[message.id] ?? "") : "";
  }

  function assistantDisplayedReasoningFadeStyle(message: ChatMessage | null) {
    if (!message) return undefined;
    const key = streamReasoningFadeKeyByMessageId[message.id] ?? 0;
    return {
      animationName: key % 2 === 0 ? "assistant-stream-fade-in-a" : "assistant-stream-fade-in-b"
    };
  }

  return {
    syncStreamingPresentationState,
    assistantDisplayContent,
    assistantDisplayStableContent,
    assistantDisplayFadeContent,
    assistantDisplayFadeStyle,
    assistantDisplayedReasoning,
    assistantDisplayedReasoningStable,
    assistantDisplayedReasoningFade,
    assistantDisplayedReasoningFadeStyle
  };
}

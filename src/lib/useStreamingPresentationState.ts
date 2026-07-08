import { shallowReactive, type ComputedRef, type Ref } from "vue";
import type { ChatMessage } from "@/types/runtime";
import { countUnclosedCodeFences } from "./markdown";

const STREAM_FADE_BATCH_CHARS = 80;
const STREAM_FADE_TIME_MS = 420;
const STREAM_FADE_FIRST_BATCH_CHARS = 24;
const STREAM_FADE_CODE_FENCE_CHARS = 18;
const STREAM_REASONING_FADE_CHARS = 3;
const STREAM_BATCH_CHARS_STORAGE_KEY = "pony-agent.stream-render.batch-chars";
const STREAM_BATCH_TIME_STORAGE_KEY = "pony-agent.stream-render.batch-ms";
const STREAM_FIRST_BATCH_CHARS_STORAGE_KEY = "pony-agent.stream-render.first-batch-chars";
const STREAM_CODE_FENCE_CHARS_STORAGE_KEY = "pony-agent.stream-render.code-fence-chars";

function readPositiveIntegerOverride(storageKey: string, fallback: number) {
  if (typeof window === "undefined") {
    return fallback;
  }

  const raw = window.localStorage.getItem(storageKey);
  if (!raw) {
    return fallback;
  }

  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : fallback;
}

function readStreamingRevealConfig() {
  return {
    batchChars: readPositiveIntegerOverride(STREAM_BATCH_CHARS_STORAGE_KEY, STREAM_FADE_BATCH_CHARS),
    timeMs: readPositiveIntegerOverride(STREAM_BATCH_TIME_STORAGE_KEY, STREAM_FADE_TIME_MS),
    firstBatchChars: readPositiveIntegerOverride(STREAM_FIRST_BATCH_CHARS_STORAGE_KEY, STREAM_FADE_FIRST_BATCH_CHARS),
    codeFenceChars: readPositiveIntegerOverride(STREAM_CODE_FENCE_CHARS_STORAGE_KEY, STREAM_FADE_CODE_FENCE_CHARS)
  };
}

function detectCodeFenceActive(content: string): boolean {
  return countUnclosedCodeFences(content) > 0;
}

export function useStreamingPresentationState(messages: ComputedRef<ChatMessage[]> | Ref<ChatMessage[]>) {
  const streamSnapshotTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamSnapshotReasoningByMessageId = shallowReactive<Record<string, string>>({});
  const streamFadeTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamFadeKeyByMessageId = shallowReactive<Record<string, number>>({});
  const streamFadeLastTimeByMessageId = shallowReactive<Record<string, number>>({});
  const streamReasoningFadeTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamReasoningFadeKeyByMessageId = shallowReactive<Record<string, number>>({});

  const PRESENTATION_MAPS = [
    streamSnapshotTextByMessageId,
    streamSnapshotReasoningByMessageId,
    streamFadeTextByMessageId,
    streamFadeKeyByMessageId,
    streamFadeLastTimeByMessageId,
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
      // Reasoning content (unchanged, simple delta model)
      const nextReasoning = message.reasoningContent ?? "";
      const previousReasoning = streamSnapshotReasoningByMessageId[message.id] ?? "";

      const appendedReasoning = previousReasoning.length > 0
        ? (nextReasoning.length > previousReasoning.length ? nextReasoning.slice(previousReasoning.length) : "")
        : "";
      syncPresentationMapValue(
        streamReasoningFadeTextByMessageId, message.id,
        appendedReasoning.length >= STREAM_REASONING_FADE_CHARS ? appendedReasoning : ""
      );
      if (streamReasoningFadeTextByMessageId[message.id]) {
        streamReasoningFadeKeyByMessageId[message.id] = (streamReasoningFadeKeyByMessageId[message.id] ?? 0) + 1;
      }
      syncPresentationMapValue(streamSnapshotReasoningByMessageId, message.id, nextReasoning);

      // Main text with batch fade: snapshot only advances on flush
      const nextText = message.content;
      const snapshotText = streamSnapshotTextByMessageId[message.id] ?? "";
      const isFirstSync = snapshotText.length === 0;
      const revealConfig = readStreamingRevealConfig();

      if (isFirstSync) {
        if (nextText.length >= revealConfig.firstBatchChars) {
          syncPresentationMapValue(streamFadeTextByMessageId, message.id, nextText);
          streamFadeKeyByMessageId[message.id] = (streamFadeKeyByMessageId[message.id] ?? 0) + 1;
          streamFadeLastTimeByMessageId[message.id] = Date.now();
          syncPresentationMapValue(streamSnapshotTextByMessageId, message.id, nextText);
        } else {
          const previousFadeText = streamFadeTextByMessageId[message.id] ?? "";
          syncPresentationMapValue(streamFadeTextByMessageId, message.id, nextText);
          if (previousFadeText !== nextText) {
            streamFadeKeyByMessageId[message.id] = (streamFadeKeyByMessageId[message.id] ?? 0) + 1;
          }
          streamFadeLastTimeByMessageId[message.id] = Date.now();
          // snapshot stays empty until the first real batch threshold or time flush is reached.
        }
      } else {
        const pendingChars = nextText.length - snapshotText.length;

        if (pendingChars <= 0) {
          syncPresentationMapValue(streamFadeTextByMessageId, message.id, "");
          continue;
        }

        const batchChars = detectCodeFenceActive(nextText)
          ? revealConfig.codeFenceChars
          : revealConfig.batchChars;
        const elapsed = Date.now() - (streamFadeLastTimeByMessageId[message.id] ?? 0);

        if (pendingChars >= batchChars || elapsed >= revealConfig.timeMs) {
          const fadeText = nextText.slice(snapshotText.length);
          syncPresentationMapValue(streamFadeTextByMessageId, message.id, fadeText);
          streamFadeKeyByMessageId[message.id] = (streamFadeKeyByMessageId[message.id] ?? 0) + 1;
          streamFadeLastTimeByMessageId[message.id] = Date.now();
          syncPresentationMapValue(streamSnapshotTextByMessageId, message.id, nextText);
        } else {
          syncPresentationMapValue(streamFadeTextByMessageId, message.id, "");
          // snapshot unchanged — content accumulates in stable
        }
      }
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
    if (fadeText) {
      return displayText.slice(0, Math.max(0, displayText.length - fadeText.length));
    }

    if (message.status === "pending") {
      return streamSnapshotTextByMessageId[message.id] ?? "";
    }

    return displayText;
  }

  function assistantDisplayFadeContent(message: ChatMessage | null) {
    return message ? (streamFadeTextByMessageId[message.id] ?? "") : "";
  }

  function assistantDisplayFadeStyle(message: ChatMessage | null) {
    if (!message) return undefined;
    return {
      animationName: "assistant-stream-fade-in"
    };
  }

  function assistantDisplayFadeKey(message: ChatMessage | null) {
    if (!message) return 0;
    return streamFadeKeyByMessageId[message.id] ?? 0;
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
    return {
      animationName: "assistant-stream-fade-in"
    };
  }

  function assistantDisplayedReasoningFadeKey(message: ChatMessage | null) {
    if (!message) return 0;
    return streamReasoningFadeKeyByMessageId[message.id] ?? 0;
  }

  return {
    syncStreamingPresentationState,
    assistantDisplayContent,
    assistantDisplayStableContent,
    assistantDisplayFadeContent,
    assistantDisplayFadeStyle,
    assistantDisplayFadeKey,
    assistantDisplayedReasoning,
    assistantDisplayedReasoningStable,
    assistantDisplayedReasoningFade,
    assistantDisplayedReasoningFadeStyle,
    assistantDisplayedReasoningFadeKey
  };
}

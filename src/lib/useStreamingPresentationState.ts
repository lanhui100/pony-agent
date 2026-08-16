import { shallowReactive, type ComputedRef, type Ref } from "vue";
import type { ChatMessage } from "@/types/runtime";

// ─── 逐字连续释放 ───────────────────────────────────────────
// 每次 tick（60ms）释放 4 个字符，避免长回复在展示层严重滞后。
const STREAM_RELEASE_CHARS_PER_TICK = 4;
const STREAM_RELEASE_CHARS_STORAGE_KEY = "pony-agent.stream-render.release-chars";

function prefersReducedMotion() {
  return typeof window !== "undefined"
    && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}

function readReleaseRate(): number {
  if (prefersReducedMotion()) return Number.MAX_SAFE_INTEGER;
  if (typeof window === "undefined") return STREAM_RELEASE_CHARS_PER_TICK;
  const raw = window.localStorage.getItem(STREAM_RELEASE_CHARS_STORAGE_KEY);
  if (!raw) return STREAM_RELEASE_CHARS_PER_TICK;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : STREAM_RELEASE_CHARS_PER_TICK;
}

// ─── Reasoning 淡入（保持原有简单增量模式） ─────────────────
const STREAM_REASONING_FADE_CHARS = 3;

// ─── 停滞收敛 ───────────────────────────────────────────────
// 若 pending 消息的 content 在约 2s（60ms tick × 34）内未再增长，
// 视为流已停滞（output_end 可能丢失/未到达），直接释放全部字符，
// 避免逐字渲染无限续跑拖垮主线程。
const STREAM_STALL_RELEASE_TICKS = 34;

export function useStreamingPresentationState(messages: ComputedRef<ChatMessage[]> | Ref<ChatMessage[]>) {
  // 逐字释放：每条正在流式回复的消息已展示到第几个字符
  const streamVisibleLengthByMessageId = shallowReactive<Record<string, number>>({});

  // Reasoning 跟踪（不变）
  const streamSnapshotReasoningByMessageId = shallowReactive<Record<string, string>>({});
  const streamReasoningFadeTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamReasoningFadeKeyByMessageId = shallowReactive<Record<string, number>>({});

  // 停滞检测：记录每条 pending 消息的 content 长度与连续未增长 tick 数
  const streamLastContentLengthByMessageId = shallowReactive<Record<string, number>>({});
  const streamStallTickCountByMessageId = shallowReactive<Record<string, number>>({});

  const PRESENTATION_MAPS = [
    streamVisibleLengthByMessageId,
    streamSnapshotReasoningByMessageId,
    streamReasoningFadeTextByMessageId,
    streamReasoningFadeKeyByMessageId,
    streamLastContentLengthByMessageId,
    streamStallTickCountByMessageId
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
      } else {
        // 非流式状态：释放全部字符，清理中间状态
        for (const map of PRESENTATION_MAPS) {
          delete map[message.id];
        }
      }
    }

    // 清理已消失消息的残留状态
    for (const map of PRESENTATION_MAPS) {
      for (const id of Object.keys(map)) {
        if (!activeMessageIds.has(id)) delete map[id];
      }
    }

    if (pendingAssistants.length === 0) return;

    for (const message of pendingAssistants) {
      // ── Reasoning ────────────────────────────────────
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

      // ── 主文本：逐字连续释放 ─────────────────────────
      const nextText = message.content;
      let currentLength = streamVisibleLengthByMessageId[message.id] ?? 0;

      // 边界安全：如果内容变短了（极少发生的 reset），回退
      if (currentLength > nextText.length) {
        currentLength = nextText.length;
      }

      if (currentLength < nextText.length) {
        // 停滞检测：content 未增长达到阈值 tick 数 → 直接释放全部字符
        const previousContentLength = streamLastContentLengthByMessageId[message.id] ?? -1;
        if (previousContentLength === nextText.length) {
          streamStallTickCountByMessageId[message.id] = (streamStallTickCountByMessageId[message.id] ?? 0) + 1;
        } else {
          streamStallTickCountByMessageId[message.id] = 0;
        }
        streamLastContentLengthByMessageId[message.id] = nextText.length;

        if ((streamStallTickCountByMessageId[message.id] ?? 0) >= STREAM_STALL_RELEASE_TICKS) {
          currentLength = nextText.length;
        } else {
          const releaseRate = readReleaseRate();
          currentLength = Math.min(currentLength + releaseRate, nextText.length);
        }
        streamVisibleLengthByMessageId[message.id] = currentLength;
      }
    }
  }

  /** 当前已释放的可见文本 */
  function assistantDisplayContent(message: ChatMessage | null) {
    if (!message) return "";
    // 非流式状态（done/error）：直接返回完整内容，避免残留的逐字游标截断文本
    if (message.status !== "pending") return message.content;
    const visibleLength = streamVisibleLengthByMessageId[message.id];
    if (visibleLength == null) return message.content;
    return message.content.slice(0, visibleLength);
  }

  /** 兼容旧接口：返回全部可见内容 */
  function assistantDisplayStableContent(message: ChatMessage | null) {
    return assistantDisplayContent(message);
  }

  // ── 以下三个函数在逐字模式中已不需要，保留空实现避免 break ──
  function assistantDisplayFadeContent(_message: ChatMessage | null) {
    return "";
  }
  function assistantDisplayFadeStyle(_message: ChatMessage | null) {
    return undefined;
  }
  function assistantDisplayFadeKey(_message: ChatMessage | null) {
    return 0;
  }

  // ── Reasoning 函数（不变） ────────────────────────────────
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
      animationName: "assistant-stream-fade-in",
      animationDuration: "350ms",
      animationTimingFunction: "ease-out",
      animationFillMode: "both"
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

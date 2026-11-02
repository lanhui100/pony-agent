import { shallowReactive, type ComputedRef, type Ref } from "vue";
import type { ChatMessage } from "@/types/runtime";
import {
  DEFAULT_REVEAL_BUDGET,
  INITIAL_REVEAL_STATE,
  stepReveal,
  visiblePrefix,
  type RevealBudgetConfig,
  type RevealState
} from "../../.agents/skills/streaming-motion/logic/budget";

// ─── 逐字连续释放（PDS-SM L4 reducer 驱动） ──────────────────
// 释放节奏的全部语义（每 tick 配额、积压加速、停滞收敛、降级）由
// `.agents/skills/streaming-motion/logic/budget.ts` 的 stepReveal 承担，
// 本文件只负责：按消息保存游标、读用户可配置速率、把结果投影成可见前缀。
// 决策与验收见 docs/decisions/0058-pds-budget-reducer-adoption.md。
const STREAM_RELEASE_CHARS_STORAGE_KEY = "pony-agent.stream-render.release-chars";

function prefersReducedMotion() {
  return typeof window !== "undefined"
    && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}

/** 用户可通过 localStorage 覆盖每 tick 释放的基础字符数（默认取 reducer 的默认值）。 */
function resolveRevealBudget(): RevealBudgetConfig {
  if (typeof window === "undefined") return DEFAULT_REVEAL_BUDGET;
  const raw = window.localStorage.getItem(STREAM_RELEASE_CHARS_STORAGE_KEY);
  if (!raw) return DEFAULT_REVEAL_BUDGET;
  const value = Number(raw);
  if (!Number.isFinite(value) || value <= 0) return DEFAULT_REVEAL_BUDGET;
  // 显式夹到 >= 1：旧实现在 "0.5" 这类速率上算出 0，文字会停滞约 2s 直到停滞兜底触发。
  // 那是缺陷而非特性，故不做等价保留（见 docs/decisions/0058 的「有意偏离」）。
  return { ...DEFAULT_REVEAL_BUDGET, charsPerTick: Math.max(1, Math.floor(value)) };
}

// ─── Reasoning 淡入（保持原有简单增量模式） ─────────────────
const STREAM_REASONING_FADE_CHARS = 3;

export function useStreamingPresentationState(messages: ComputedRef<ChatMessage[]> | Ref<ChatMessage[]>) {
  // 逐字释放游标：每条正在流式回复的消息的完整 reducer 状态
  const streamRevealStateByMessageId = shallowReactive<Record<string, RevealState>>({});

  // Reasoning 跟踪（不变）
  const streamSnapshotReasoningByMessageId = shallowReactive<Record<string, string>>({});
  const streamReasoningFadeTextByMessageId = shallowReactive<Record<string, string>>({});
  const streamReasoningFadeKeyByMessageId = shallowReactive<Record<string, number>>({});

  const PRESENTATION_MAPS = [
    streamRevealStateByMessageId,
    streamSnapshotReasoningByMessageId,
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

    const budget = resolveRevealBudget();
    const reduceMotion = prefersReducedMotion();

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

      // ── 主文本：单次 reducer 推进 ────────────────────
      // 只在"还没追上"时推进并持久化 —— 这正是重构前实现的写入时机：已追平
      // （含内容被截短）时旧实现既不推进也不回写游标，于是截断后再次增长会瞬间
      // 追平截断前的进度，而不是从新长度重新逐字打一遍。
      // 因此这里**不能让 reducer 的夹取结果回写宿主游标**：`visible >= incomingLength`
      // 时整段跳过，游标（高水位）与停滞计数都保持原样。
      // 副带收益：已追平时不产生任何响应式写入，避免多余失效。
      const incoming = message.content.length;
      const previousReveal = streamRevealStateByMessageId[message.id] ?? INITIAL_REVEAL_STATE;
      if (previousReveal.visible < incoming) {
        streamRevealStateByMessageId[message.id] = stepReveal(
          previousReveal,
          { incomingLength: incoming, reduceMotion },
          budget
        );
      }
    }
  }

  /** 当前已释放的可见文本 */
  function assistantDisplayContent(message: ChatMessage | null) {
    if (!message) return "";
    // 非流式状态（done/error）：直接返回完整内容，避免残留的逐字游标截断文本
    if (message.status !== "pending") return message.content;
    const revealState = streamRevealStateByMessageId[message.id];
    if (revealState == null) return message.content;
    return visiblePrefix(message.content, revealState);
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

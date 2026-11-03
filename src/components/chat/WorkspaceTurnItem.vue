<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowReactive } from "vue";
import type { ComponentPublicInstance } from "vue";
import { storeToRefs } from "pinia";
import {
  Activity,
  AlertTriangle,
  Check,
  ChevronDown,
  ClipboardList,
  Copy,
  FileSearch,
  FileText,
  Globe,
  History,
  List,
  LoaderCircle,
  MessageSquareMore,
  Pen,
  PenLine,
  Plug,
  RotateCcw,
  ScanSearch,
  Search,
  Terminal,
  UserRound,
  Wrench,
} from "lucide-vue-next";
import type { ChatMessage } from "@/types/runtime";
import { useRuntimeStore } from "@/stores/runtime";
import MarkdownRenderer, {
  checkStreamTailCandidate,
  isStreamTailReducedMotionActive,
  isStreamTailTextSingleLine,
  readStreamTailSwitchPreference
} from "@/components/MarkdownRenderer.vue";
import AskUserToolCallCard from "@/components/ask/AskUserToolCallCard.vue";
import { isAskToolName } from "@/lib/runtime/ask-tools";
import {
  PopoverClose,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger,
} from "reka-ui";

export type MergedToolCall = {
  id: string;
  toolName: string;
  canonicalToolName: string | null;
  displayNameZh: string | null;
  mergeKey: string;
  description: string;
  status: ChatMessage["status"];
  durationSeconds: number | null;
  count: number;
  /** PA-114: Ask 卡片精确绑定 pending ask 的调用 ID（后端 ToolActivity.id == call_id）。 */
  callId?: string | null;
  /** PA-114: 工具调用参数 JSON 字符串（ask_user 的 question/options 等）。 */
  argumentsText?: string | null;
};

// Wave-59 F1-1：`waiting` 分支已从联合类型删除（编译期强制清空三点等待动画的渲染路径；
// 「思考中」的唯一指示由底部状态条承载，见 HomeWorkspace 的 workspace-runtime-status-slot）。
export type AgentTurnEvent =
  | { kind: "reasoning"; key: string; order: number; assistant: ChatMessage; reasoningContent: string; streaming: boolean }
  | { kind: "tools"; key: string; order: number; tools: MergedToolCall[] }
  | { kind: "content"; key: string; order: number; assistant: ChatMessage; content: string; streaming: boolean }
  | { kind: "error"; key: string; order: number };

export type TurnBucket = {
  turnId: string;
  user: ChatMessage | null;
  assistant: ChatMessage | null;
  tools: ChatMessage[];
  mergedTools: MergedToolCall[];
};

export type CheckpointRollbackAction = "transcript_only" | "transcript_and_workspace";

const props = defineProps<{
  turn: TurnBucket;
  events: AgentTurnEvent[];
  shouldShowAgentArticle: boolean;
  rollbackInFlight: boolean;
  confirmRollback: (turnId: string, action: CheckpointRollbackAction) => void;
  setUserMessageRef: (element: Element | ComponentPublicInstance | null) => void;
  setAgentMessageRef: (element: Element | ComponentPublicInstance | null) => void;
  handleMarkdownRenderComplete: (payload: { contentLength: number; streaming: boolean }) => void;
  shouldUseMarkdownAssistantRendering: (message: ChatMessage | null, content: string, streaming?: boolean) => boolean;
  /**
   * 保留声明仅为维持调用方契约（HomeWorkspace 绑定 + 3 处直接挂载的本组件用例）。
   * Wave-59 F3-4 起内容区不再按「流式优化是否启用」分叉，本 prop 不参与渲染决策。
   */
  shouldUseOptimizedAssistantStreaming: (message: ChatMessage | null) => boolean;
  streamingReasoningStable: (reasoningContent: string, assistant: ChatMessage) => string;
  streamingReasoningFade: (reasoningContent: string, assistant: ChatMessage) => string;
  assistantDisplayedReasoningFadeStyle: (message: ChatMessage | null) => Record<string, string> | undefined;
  assistantDisplayedReasoningFadeKey: (message: ChatMessage | null) => number;
}>();

const runtimeStore = useRuntimeStore();
const { isSubmitting, sessionOperation, turnTraceHistory } = storeToRefs(runtimeStore);

const copiedErrorDetail = ref(false);
const copiedAssistantResponse = ref(false);
const copiedUserMessage = ref(false);

/** 选中即复制：光标选中消息文本后自动写入剪贴板，并在指针附近短暂提示 */
const autoCopyPill = ref<{ x: number; y: number } | null>(null);
let autoCopyPillTimer: ReturnType<typeof setTimeout> | null = null;

function showAutoCopyPill(event: MouseEvent) {
  autoCopyPill.value = { x: event.clientX, y: event.clientY };
  if (autoCopyPillTimer !== null) {
    window.clearTimeout(autoCopyPillTimer);
  }
  autoCopyPillTimer = window.setTimeout(() => {
    autoCopyPill.value = null;
    autoCopyPillTimer = null;
  }, 1600);
}

function handleSelectionAutoCopy(event: MouseEvent) {
  if (typeof window === "undefined" || typeof navigator === "undefined") return;
  const selection = window.getSelection();
  const container = event.currentTarget as HTMLElement | null;
  if (!selection || !container) return;
  if (selection.isCollapsed || selection.rangeCount === 0) return;

  // 仅当选中区整体落在消息内容容器内时自动复制，避免把旁侧文本一并带走
  const anchorEl =
    selection.anchorNode?.nodeType === Node.ELEMENT_NODE
      ? (selection.anchorNode as Element)
      : (selection.anchorNode?.parentElement ?? null);
  const focusEl =
    selection.focusNode?.nodeType === Node.ELEMENT_NODE
      ? (selection.focusNode as Element)
      : (selection.focusNode?.parentElement ?? null);
  if (!container.contains(anchorEl) || !container.contains(focusEl)) return;

  const text = selection.toString().trim();
  if (!text) return;
  if (!navigator.clipboard?.writeText) return;

  void navigator.clipboard
    .writeText(text)
    .then(() => {
      showAutoCopyPill(event);
    })
    .catch(() => {});
}

onBeforeUnmount(() => {
  if (autoCopyPillTimer !== null) {
    window.clearTimeout(autoCopyPillTimer);
    autoCopyPillTimer = null;
  }
});

function userShellClass() {
  return "rounded-[0.45rem] bg-stone-900 px-3 py-2 text-stone-50 shadow-[0_1px_0_rgba(28,25,23,0.03)] sm:px-4";
}

function actorLabelClass() {
  return "inline-flex items-center gap-2 text-[10px] uppercase tracking-[0.18em] text-stone-500";
}

function assistantTone(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return "text-stone-800";
}

function shouldOpenReasoningBlock(_message: ChatMessage | null) {
  return false;
}

function isAssistantStreaming(message: ChatMessage | null) {
  return message?.status === "pending";
}

/**
 * Wave-59 F3-4：正文走 markdown 渲染还是纯文本渲染（两条路径共用同一外层容器，
 * 且承载正文的都是 `.markdown-body`，故 streaming→完成态不替换承载节点）。
 */
function assistantUsesMarkdownRendering(event: Extract<AgentTurnEvent, { kind: "content" }>) {
  return props.shouldUseMarkdownAssistantRendering(event.assistant, event.content, event.streaming);
}

// ─── wave-pds-3 v8：纯文本快路径的尾巴（契约 v8） ─────────────────────────────
//
// 背景：`isSimpleTextContent` 命中时正文走「纯文本快路径」——由本组件自建 `.markdown-body`
// 直接插值 `event.content`，**不经过** MarkdownRenderer。v7 之前这条路径永远没有尾巴，
// 而"没有 markdown 标记的普通回复"恰恰最该有浮现效果。
//
// 实现要点（与 MarkdownRenderer 共用同一套纯函数，避免两种语义）：
// 1. 切分：取已释放文本里**最后一个空行（`\n\n`）之后**为尾巴；无空行则整段即尾巴。
// 2. **绝不吞字**：`stable` 与 `tail` 都是同一字符串的连续切片，拼接后恒等于原文；
//    下面还有一条运行时自检兜底（不等就退回"整段 stable、无尾巴"）。
// 3. 遮罩：尾巴实际单行才启用（多行由 `data-pds-tail-multiline` 让 CSS 关掉），与 MarkdownRenderer 同源。
// 4. 开关 off / reduced-motion / 非纯段落（资格门）= 不发尾巴，退回 wave-59 行为。
// 5. 宽度来自 ResizeObserver 缓存（**不在渲染路径读 clientWidth**，避免每 tick 强制布局）。
const plainTextTailSwitchOn = readStreamTailSwitchPreference();
const plainTextTailReduceMotionQuery = typeof window !== "undefined" && typeof window.matchMedia === "function"
  ? window.matchMedia("(prefers-reduced-motion: reduce)")
  : null;
/** 每条纯文本承载节点的内容宽度（px）；0/缺省 = 尚未测到（jsdom 等无布局环境）。 */
const plainTextTailWidths = shallowReactive<Record<string, number>>({});
const plainTextTailObservers = new Map<string, ResizeObserver>();
let plainTextTailFontSpec = "";
let plainTextTailMeasureText: ((text: string) => number) | null = null;

function isPlainTextTailActive(): boolean {
  return plainTextTailSwitchOn && !isStreamTailReducedMotionActive(plainTextTailReduceMotionQuery);
}

/** 最近一次挂载的纯文本承载节点（用于延迟解析测量器）。 */
let plainTextTailMeasureElement: HTMLElement | null = null;

/** 解析 canvas 测量器（代价高：getComputedStyle + canvas，只在首次/字号变化时调）。 */
function refreshPlainTextTailMeasurer(element: HTMLElement | null) {
  plainTextTailMeasureText = null;
  if (!element || typeof window === "undefined" || element.clientWidth <= 0) return;
  const context = document.createElement("canvas").getContext?.("2d") ?? null;
  if (!context) return;
  const style = window.getComputedStyle(element);
  plainTextTailFontSpec = `${style.fontStyle || "normal"} ${style.fontWeight || "400"} `
    + `${style.fontSize || "14px"} ${style.fontFamily || "sans-serif"}`;
  context.font = plainTextTailFontSpec;
  plainTextTailMeasureText = (text: string) => context.measureText(text).width;
}

/**
 * 延迟解析测量器：ref 回调跑在挂载期，此时元素还没布局（`clientWidth === 0`），
 * 直接解析会得到 null ⇒ 单行判定退化成"总是单行"，长段落尾巴会被误挂遮罩。
 * 故首次真正需要测量时再解析一次并缓存（只在 null 时付代价）。
 */
function ensurePlainTextTailMeasurer(): ((text: string) => number) | null {
  if (plainTextTailMeasureText) return plainTextTailMeasureText;
  refreshPlainTextTailMeasurer(plainTextTailMeasureElement);
  return plainTextTailMeasureText;
}

/**
 * 纯文本承载节点的 ref 回调：按 event.key 跟踪宽度（ResizeObserver 只在**宽度变化**时写响应式，
 * 因此不会与流式期的逐帧高度变化形成重渲染循环）。
 */
function setPlainTextTailBodyRef(key: string, element: unknown) {
  const previous = plainTextTailObservers.get(key);
  if (previous) {
    previous.disconnect();
    plainTextTailObservers.delete(key);
  }
  const target = element instanceof HTMLElement ? element : null;
  if (!target) return;
  plainTextTailMeasureElement = target;
  refreshPlainTextTailMeasurer(target);
  if (typeof ResizeObserver === "undefined") return;
  const observer = new ResizeObserver((entries) => {
    const entry = entries[0];
    if (!entry) return;
    const width = Number.isFinite(entry.contentRect?.width)
      ? Math.max(0, Math.floor(entry.contentRect.width))
      : 0;
    if (width > 0 && plainTextTailWidths[key] !== width) {
      plainTextTailWidths[key] = width;
      // 宽度变化可能改变换行结论；顺带刷新一次字体测量器（只在真正变化时付代价）。
      refreshPlainTextTailMeasurer(plainTextTailMeasureElement);
    }
  });
  observer.observe(target);
  plainTextTailObservers.set(key, observer);
}

onBeforeUnmount(() => {
  for (const observer of plainTextTailObservers.values()) observer.disconnect();
  plainTextTailObservers.clear();
  plainTextTailMeasureText = null;
});

// 资格判定与 markdown 路径**同源**（v9 render-then-verify）。渲染期不能 await，故采用
// 「按尾巴文本缓存的异步判定 + 未判定时乐观放行」：纯文本尾巴渲染的是**转义后的纯文本**
// （不含任何块级标签），因此乐观放行最坏只是某类 exotic 文本多显示一帧尾巴。
const plainTextTailEligibility = shallowReactive<Record<string, boolean>>({});
const plainTextTailEligibilityPending = new Set<string>();
const PLAIN_TEXT_TAIL_ELIGIBILITY_CACHE_LIMIT = 64;

function evaluatePlainTextTailEligibility(tail: string) {
  if (tail in plainTextTailEligibility || plainTextTailEligibilityPending.has(tail)) return;
  plainTextTailEligibilityPending.add(tail);
  void checkStreamTailCandidate(tail)
    .then((candidate) => {
      if (Object.keys(plainTextTailEligibility).length >= PLAIN_TEXT_TAIL_ELIGIBILITY_CACHE_LIMIT) {
        for (const key of Object.keys(plainTextTailEligibility)) delete plainTextTailEligibility[key];
      }
      plainTextTailEligibility[tail] = candidate.eligible;
    })
    .catch(() => {
      // 判定失败按"可发尾巴"处理：纯文本尾巴不含块级标签，不会带来语义风险。
      plainTextTailEligibility[tail] = true;
    })
    .finally(() => {
      plainTextTailEligibilityPending.delete(tail);
    });
}

function isPlainTextTailEligible(tail: string): boolean {
  const cached = plainTextTailEligibility[tail];
  if (cached !== undefined) return cached;
  evaluatePlainTextTailEligibility(tail);
  return true;
}

interface PlainTextTailParts {
  /** 尾巴之前（含尾部空行）的文本：与 `tail` 拼接后**恒等于**已释放文本。 */
  stable: string;
  /** 唯一尾巴文本（未闭合段）；空串 ⇒ 不渲染尾巴节点。 */
  tail: string;
  /** `tail` 的前段（尾巴除去热区）；与 `hot` 拼接恒等于 `tail`。 */
  tailBody: string;
  /** 热区：尾巴最后若干字符，每次重建都会重播一次短淡入，制造"逐字浮现"。 */
  hot: string;
  /** 尾巴是否实际单行（决定是否启用遮罩）。 */
  singleLine: boolean;
}

const NO_PLAIN_TEXT_TAIL = (content: string): PlainTextTailParts => ({ stable: content, tail: "", tailBody: "", hot: "", singleLine: false });

/** 热区宽度（字符）：只让最后 N 个字重播淡入。中文一个字符即有明显视觉量，取 10。 */
const PLAIN_TEXT_TAIL_HOT_CHARS = 10;

function splitPlainTextTail(key: string, content: string, streaming: boolean): PlainTextTailParts {
  if (!streaming || !isPlainTextTailActive()) return NO_PLAIN_TEXT_TAIL(content);

  const boundary = content.lastIndexOf("\n\n");
  const stable = boundary >= 0 ? content.slice(0, boundary + 2) : "";
  const tail = boundary >= 0 ? content.slice(boundary + 2) : content;
  if (!tail.trim()) return NO_PLAIN_TEXT_TAIL(content);
  // 契约 v7 §1 资格门：纯文本里若混入围栏/列表等构造，同样不发尾巴。
  if (!isPlainTextTailEligible(tail)) return NO_PLAIN_TEXT_TAIL(content);
  // 自检（v8 最要紧一条）：拼接必须与已释放文本逐字节相等，绝不吞字/重复。
  if (`${stable}${tail}` !== content) return NO_PLAIN_TEXT_TAIL(content);

  const hotLen = Math.min(PLAIN_TEXT_TAIL_HOT_CHARS, tail.length);
  const tailBody = hotLen > 0 ? tail.slice(0, tail.length - hotLen) : tail;
  const hot = hotLen > 0 ? tail.slice(tail.length - hotLen) : "";

  return {
    stable,
    tail,
    tailBody,
    hot,
    singleLine: isStreamTailTextSingleLine(tail, plainTextTailWidths[key] ?? 0, ensurePlainTextTailMeasurer())
  };
}

/** 按事件 key 预计算纯文本尾巴（模板只读结果，避免同一帧重复切分）。 */
const plainTextTailByEventKey = computed<Record<string, PlainTextTailParts>>(() => {
  const partsByKey: Record<string, PlainTextTailParts> = {};
  for (const event of props.events) {
    if (event.kind !== "content") continue;
    if (assistantUsesMarkdownRendering(event)) continue;
    partsByKey[event.key] = splitPlainTextTail(event.key, event.content, event.streaming);
  }
  return partsByKey;
});

function plainTextTailPartsFor(event: Extract<AgentTurnEvent, { kind: "content" }>): PlainTextTailParts {
  return plainTextTailByEventKey.value[event.key] ?? NO_PLAIN_TEXT_TAIL(event.content);
}

function latestTraceForTurn(turnId: string) {
  return [...turnTraceHistory.value].reverse().find((trace) => trace.turnId === turnId) ?? null;
}

function assistantErrorDetail(turn: TurnBucket): string {
  if (turn.assistant?.status !== "error") return "";

  const latestTrace = latestTraceForTurn(turn.turnId);
  const modelError = [...(latestTrace?.traceTimeline ?? [])]
    .reverse()
    .find((entry) => entry.kind === "call_model" && entry.error?.trim())
    ?.error?.trim();

  return modelError || latestTrace?.error?.trim() || turn.assistant.errorDetail?.trim() || "";
}

function copyErrorDetail(_turnId: string, text: string) {
  copiedErrorDetail.value = true;

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text);
  }

  if (typeof window !== "undefined") {
    window.setTimeout(() => {
      copiedErrorDetail.value = false;
    }, 1200);
  }
}

function copyAssistantResponse(_turnId: string, content: string) {
  copiedAssistantResponse.value = true;

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(content);
  }

  if (typeof window !== "undefined") {
    window.setTimeout(() => {
      copiedAssistantResponse.value = false;
    }, 1200);
  }
}

function copyUserMessage(_turnId: string, content: string) {
  copiedUserMessage.value = true;

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(content);
  }

  if (typeof window !== "undefined") {
    window.setTimeout(() => {
      copiedUserMessage.value = false;
    }, 1200);
  }
}

const toolIconByCanonicalName: Record<string, any> = {
  Run: Terminal,
  Ask: MessageSquareMore,
  Read: FileText,
  Search: Search,
  List: List,
  Glob: FileSearch,
  WebFetch: Globe,
  WebSearch: ScanSearch,
  MCPResource: Plug,
  ToolSearch: Wrench,
  Write: Pen,
  Edit: PenLine,
  Plan: ClipboardList,
};
</script>

<template>
  <section class="space-y-3" :data-turn-id="turn.turnId">
    <article
      v-if="turn.user"
      :ref="(element) => setUserMessageRef(element)"
      class="conversation-user-message ml-auto w-fit max-w-[68.8%] sm:max-w-[54.4%]"
    >
      <div class="flex flex-col items-end">
        <div :class="actorLabelClass()" class="mb-1">
          <span>User</span>
          <UserRound class="h-3.5 w-3.5" />
        </div>
        <div :class="userShellClass()">
          <div
            class="text-left whitespace-pre-wrap text-sm leading-6 select-text"
            @mouseup="handleSelectionAutoCopy"
          >
            {{ turn.user.content }}
          </div>
        </div>
        <div
          class="message-action-bar relative self-start mt-1.5 flex items-center gap-1.5"
          data-testid="workspace-user-checkpoint-actions"
        >
          <PopoverRoot>
            <PopoverTrigger as-child>
              <button
                class="checkpoint-icon-button"
                type="button"
                :disabled="isSubmitting || !!sessionOperation || rollbackInFlight"
                :title="isSubmitting ? '运行中暂不可撤回' : '仅撤回对话'"
              >
                <History class="h-3.5 w-3.5" />
                <span class="sr-only">仅撤回对话</span>
              </button>
            </PopoverTrigger>
            <PopoverPortal>
              <PopoverContent side="top" align="center" :side-offset="6" class="z-50 rounded-[0.3rem] border border-stone-200/70 bg-white/97 px-2.5 py-1.5 shadow-md backdrop-blur">
                <div class="flex items-center gap-1 text-nowrap">
                  <span class="text-[11px] leading-none text-stone-400 select-none">确认仅撤回对话？</span>
                  <PopoverClose as-child>
                    <button type="button" class="inline-flex items-center justify-center w-5 h-5 rounded-[0.25rem] text-rose-500 hover:bg-rose-100/80 hover:text-rose-600 transition cursor-pointer cursor-pointer" @click="confirmRollback(turn.turnId, 'transcript_only')">
                      <Check class="h-3 w-3" />
                    </button>
                  </PopoverClose>
                </div>
              </PopoverContent>
            </PopoverPortal>
          </PopoverRoot>
          <PopoverRoot>
            <PopoverTrigger as-child>
              <button
                class="checkpoint-icon-button"
                type="button"
                :disabled="isSubmitting || !!sessionOperation || rollbackInFlight"
                :title="isSubmitting ? '运行中暂不可撤回' : '撤回对话和修改'"
              >
                <RotateCcw class="h-3.5 w-3.5" />
                <span class="sr-only">撤回对话和修改</span>
              </button>
            </PopoverTrigger>
            <PopoverPortal>
              <PopoverContent side="top" align="center" :side-offset="6" class="z-50 rounded-[0.3rem] border border-stone-200/70 bg-white/97 px-2.5 py-1.5 shadow-md backdrop-blur">
                <div class="flex items-center gap-1 text-nowrap">
                  <span class="text-[11px] leading-none text-stone-400 select-none">确认撤回对话和文件？</span>
                  <PopoverClose as-child>
                    <button type="button" class="inline-flex items-center justify-center w-5 h-5 rounded-[0.25rem] text-rose-500 hover:bg-rose-100/80 hover:text-rose-600 transition cursor-pointer cursor-pointer" @click="confirmRollback(turn.turnId, 'transcript_and_workspace')">
                      <Check class="h-3 w-3" />
                    </button>
                  </PopoverClose>
                </div>
              </PopoverContent>
            </PopoverPortal>
          </PopoverRoot>
          <button
            class="checkpoint-icon-button"
            type="button"
            :data-testid="`workspace-user-copy-${turn.turnId}`"
            :title="copiedUserMessage ? '已复制' : '复制消息'"
            @click="copyUserMessage(turn.turnId, turn.user?.content ?? '')"
          >
            <component :is="copiedUserMessage ? Check : Copy" class="h-3.5 w-3.5" />
            <span class="sr-only">复制消息</span>
          </button>
        </div>
      </div>
    </article>

    <article
      v-if="shouldShowAgentArticle"
      :ref="(element) => setAgentMessageRef(element)"
      class="conversation-agent-shell flex w-full flex-col gap-2 px-0 py-1"
    >
      <template v-for="event in events" :key="event.key">
        <details
          v-if="event.kind === 'reasoning'"
          :open="shouldOpenReasoningBlock(event.assistant)"
          v-motion
          :initial="{ opacity: 0, y: 4 }"
          :animate="{ opacity: 1, y: 0 }"
          :transition="{ duration: 0.16, ease: 'easeOut' }"
          class="conversation-disclosure conversation-reasoning-panel group p-0 transition-item-enter"
          style="contain: layout;"
        >
          <summary class="conversation-disclosure-summary">
            <div class="flex min-w-0 items-center gap-2">
              <Activity class="h-3 w-3 shrink-0 text-stone-400" />
              <span>思考过程</span>
            </div>
            <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
          </summary>
          <div class="conversation-reasoning-body mt-1.5 ml-1 border-l-2 border-stone-200/80 pl-3 whitespace-pre-wrap break-words text-[12.5px] leading-relaxed text-stone-500/90">
            <template v-if="event.reasoningContent">
              <span class="reasoning-italic">{{ event.streaming ? streamingReasoningStable(event.reasoningContent, event.assistant) : event.reasoningContent }}</span>
              <span
                v-if="event.streaming && streamingReasoningFade(event.reasoningContent, event.assistant)"
                :key="`rfade-${event.assistant.id}-${assistantDisplayedReasoningFadeKey(event.assistant)}`"
                class="assistant-streaming-fade reasoning-italic"
                :style="assistantDisplayedReasoningFadeStyle(event.assistant)"
              >
                {{ streamingReasoningFade(event.reasoningContent, event.assistant) }}
              </span>
            </template>
          </div>
        </details>

        <div
          v-else-if="event.kind === 'tools'"
          v-motion
          :initial="{ opacity: 0, y: 4 }"
          :animate="{ opacity: 1, y: 0 }"
          :transition="{ duration: 0.16, ease: 'easeOut' }"
          class="conversation-tool-panel space-y-0.5 transition-item-enter"
        >
          <template v-for="tool in event.tools" :key="tool.id">
            <!--
              PA-114: Ask 工具行渲染专属交互卡片，其余行保持通用渲染（F1-4）。
              wave-pds-3 §1 / §3.3：每条工具行都必须带 `data-pds-tool-row` 锚点并挂入场消融
              （`filter: blur(2px)→0`，220ms）。两种行体都靠属性/类透传到根节点，不改内部结构。
              wave-pds-4：行级入场补齐与「思考过程」块**完全相同**的 motion 层
              （`opacity 0→1, y 4→0, 160ms easeOut`）；此前该块有 motion+Css 两层，
              工具行只有 Css 一层，造成两者手感不一致。
            -->
            <AskUserToolCallCard
              v-if="isAskToolName(tool.toolName)"
              :tool="tool"
              v-motion
              :initial="{ opacity: 0, y: 4 }"
              :animate="{ opacity: 1, y: 0 }"
              :transition="{ duration: 0.16, ease: 'easeOut' }"
              data-pds-tool-row
              class="pds-tool-row-enter"
            />
            <div
              v-else
              v-motion
              :initial="{ opacity: 0, y: 4 }"
              :animate="{ opacity: 1, y: 0 }"
              :transition="{ duration: 0.16, ease: 'easeOut' }"
              class="pds-tool-row-enter flex flex-col py-0.5 text-[12px] leading-5"
              data-pds-tool-row
            >
              <div class="flex items-center gap-2">
                <component :is="toolIconByCanonicalName[tool.canonicalToolName ?? ''] ?? Wrench" class="h-3 w-3 shrink-0 text-stone-400" />
                <span
                  v-if="tool.displayNameZh || tool.canonicalToolName || tool.toolName || tool.description"
                  class="conversation-tool-name shrink-0 text-stone-400"
                >
                  {{ tool.displayNameZh || tool.canonicalToolName || tool.toolName || tool.description }}
                </span>
                <span
                  v-if="tool.description && tool.description !== (tool.displayNameZh || tool.canonicalToolName || tool.toolName)"
                  class="conversation-tool-detail min-w-0 truncate text-stone-400"
                  :title="tool.description"
                >
                  {{ tool.description }}
                </span>
                <span class="conversation-tool-status flex shrink-0 items-center gap-1 leading-none">
                  <span class="text-[11px] text-stone-400" :class="tool.durationSeconds != null ? 'visible' : 'invisible'">{{ tool.durationSeconds != null ? (tool.durationSeconds).toFixed(1) + 's' : '0.0s' }}</span>
                  <LoaderCircle v-if="tool.status === 'pending'" class="h-3 w-3 animate-spin text-stone-400" />
                  <Check v-else-if="tool.status === 'done'" class="h-3 w-3 text-stone-400" />
                  <AlertTriangle v-else-if="tool.status === 'error'" class="h-3 w-3 shrink-0 text-rose-400" aria-label="工具调用失败" :aria-hidden="false" />
                </span>
              </div>
            </div>
          </template>
        </div>

        <div
          v-else-if="event.kind === 'content'"
          class="assistant-response-panel"
        >
          <!-- 统一 shell：流式/完成共享同一外容器，避免 v-if/v-else 导致的 DOM 子树替换 -->
          <div
            class="assistant-plain-text text-sm select-text"
            :class="assistantTone(event.assistant)"
            :data-streaming="event.streaming ? 'true' : undefined"
            @mouseup="handleSelectionAutoCopy"
          >
            <!--
              Wave-59 F3-4：内容区只有这一个容器（不再按 streaming 分叉成两棵子树）。
              两条渲染路径都落在同一容器上，且正文承载节点都是 `.markdown-body`：
              - markdown 路径：由 MarkdownRenderer 的常驻 `.markdown-body` 承载（F3-2/F3-3 身份不变）；
              - 纯文本路径：本容器内自建 `.markdown-body`（逐字符 span 已按 D-59-2 移除）。
              纯文本→完成态沿用纯文本路径由 HomeWorkspace.shouldUseMarkdownAssistantRendering 锁定，
              因此完成帧不会替换承载节点（RC5）。
            -->
            <div
              class="assistant-streaming-content"
              :data-testid="event.streaming ? 'assistant-streaming-flow' : undefined"
            >
              <MarkdownRenderer
                v-if="assistantUsesMarkdownRendering(event)"
                :content="event.content"
                :streaming="event.streaming"
                :force-markdown-streaming="event.streaming"
                :wrapper-class="[
                  'assistant-markdown',
                  event.streaming ? 'assistant-streaming-markdown' : ''
                ].filter(Boolean).join(' ')"
                :tone-class="assistantTone(event.assistant)"
                @render-complete="handleMarkdownRenderComplete"
              />
              <!-- 纯文本快路径（v8）：尾巴 = 最后一个空行之后的未闭合段；stable/tail 为连续切片，拼接恒等于原文。整行书写，避免 `whitespace-pre-wrap` 把模板缩进当成正文。 -->
              <div v-else class="markdown-body whitespace-pre-wrap" :ref="(element) => setPlainTextTailBodyRef(event.key, element)">{{ plainTextTailPartsFor(event).stable }}<span v-if="plainTextTailPartsFor(event).tail" data-stream-tail :data-tail-len="plainTextTailPartsFor(event).tail.length" :data-tail-hot="plainTextTailPartsFor(event).hot ? 'true' : undefined" :data-pds-tail-multiline="plainTextTailPartsFor(event).singleLine ? undefined : 'true'">{{ plainTextTailPartsFor(event).tailBody }}<span v-if="plainTextTailPartsFor(event).hot" data-pds-tail-hot>{{ plainTextTailPartsFor(event).hot }}</span></span></div>
            </div>
          </div>
        </div>

        <!-- Error detail panel (raw error for debugging) -->
        <details
          v-else-if="event.kind === 'error'"
          class="conversation-disclosure conversation-error-panel group"
        >
          <summary class="conversation-disclosure-summary text-rose-700">
            <div class="flex min-w-0 items-center gap-2">
              <AlertTriangle class="h-3.5 w-3.5 shrink-0 text-rose-500" />
              <span class="text-rose-700">错误详情</span>
            </div>
            <div class="ml-auto flex items-center gap-1">
              <button
                class="invisible group-hover:visible inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-rose-50 hover:text-rose-600"
                type="button"
                :data-testid="`workspace-error-copy-${turn.turnId}`"
                @click.stop="copyErrorDetail(turn.turnId, assistantErrorDetail(turn))"
              >
                <component
                  :is="copiedErrorDetail ? Check : Copy"
                  class="h-3 w-3"
                />
              </button>
              <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
            </div>
          </summary>
          <div
            class="whitespace-pre-wrap break-words px-3 py-2 text-[11px] leading-4 text-rose-900"
            data-testid="workspace-error-detail"
          >
            {{ assistantErrorDetail(turn) }}
          </div>
        </details>
      </template>

      <div
        v-if="turn.assistant && !isAssistantStreaming(turn.assistant)"
        class="agent-action-bar ml-auto flex flex-wrap items-center justify-end gap-2"
        data-testid="workspace-agent-actions"
      >
        <button
          class="checkpoint-icon-button"
          type="button"
          :title="copiedAssistantResponse ? '已复制' : '复制回复'"
          @click="copyAssistantResponse(turn.turnId, turn.assistant?.content ?? '')"
        >
          <component :is="copiedAssistantResponse ? Check : Copy" class="h-3.5 w-3.5" />
          <span class="sr-only">复制回复</span>
        </button>
      </div>
    </article>

    <Teleport to="body">
      <Transition name="auto-copy-pill-fade">
        <div
          v-if="autoCopyPill"
          class="auto-copy-pill-position pointer-events-none fixed z-[200] flex items-center gap-1 rounded-full bg-stone-800/92 px-2.5 py-1 text-[11px] leading-none text-stone-50 shadow-lg backdrop-blur-sm"
          :style="{ left: `${autoCopyPill.x}px`, top: `${autoCopyPill.y}px` }"
          role="status"
          data-testid="auto-copy-feedback"
        >
          <Check class="h-3 w-3" />
          已复制
        </div>
      </Transition>
    </Teleport>
  </section>
</template>

<style scoped>
:deep(.markdown-body) {
  min-width: 0;
  overflow-wrap: anywhere;
  word-break: break-word;
  line-height: 1.7;
  color: #3d342d;
}

:deep(.assistant-markdown > :first-child),
:deep(.assistant-markdown .markdown-body > :first-child) {
  margin-top: 0 !important;
}

:deep(.assistant-markdown > :last-child),
:deep(.assistant-markdown .markdown-body > :last-child) {
  margin-bottom: 0 !important;
}

:deep(.assistant-markdown p) {
  margin: 0 0 1.2em;
  color: inherit;
}

:deep(.assistant-markdown p:empty) {
  display: none !important;
  margin: 0 !important;
}

:deep(.assistant-markdown p:last-child),
:deep(.assistant-markdown pre:last-child),
:deep(.assistant-markdown .table-scroll-wrapper:last-child),
:deep(.assistant-markdown blockquote:last-child),
:deep(.assistant-markdown ul:last-child),
:deep(.assistant-markdown ol:last-child) {
  margin-bottom: 0 !important;
}

:deep(.assistant-markdown h1),
:deep(.assistant-markdown h2),
:deep(.assistant-markdown h3),
:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  margin: 1rem 0 0.45rem;
  font-size: inherit;
  line-height: 1.5;
  letter-spacing: 0;
  color: #241b14;
}

:deep(.assistant-markdown h1),
:deep(.assistant-markdown h2) {
  font-weight: 600;
}

:deep(.assistant-markdown h3),
:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  font-weight: 520;
  color: #3c3028;
}

:deep(.assistant-markdown ul),
:deep(.assistant-markdown ol) {
  margin: 0.5rem 0 0.95rem 1.35rem;
  padding: 0;
}

:deep(.assistant-markdown li + li) {
  margin-top: 0.38rem;
}

:deep(.assistant-markdown li > p) {
  margin: 0.35rem 0;
}

:deep(.assistant-markdown pre) {
  margin: 0.8rem 0;
  overflow-x: auto;
  border-radius: 0.45rem;
  background: #faf5eb;
  padding: 0.8rem 0.9rem;
  font-size: 0.82rem;
  line-height: 1.55;
  color: #2f261d;
}

:deep(.assistant-markdown code) {
  border-radius: 0.3rem;
  background: #faf5eb;
  padding: 0.08rem 0.34rem;
  font-size: 0.82em;
  color: #5b4330;
}

:deep(.assistant-markdown pre code) {
  background: transparent;
  padding: 0;
  border-radius: 0;
  display: block;
  color: #2f261d;
  font-size: 1em;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
}

:deep(.assistant-markdown .table-scroll-wrapper) {
  overflow-x: auto;
  margin: 0.8rem 0;
}

:deep(.assistant-markdown .table-scroll-wrapper table) {
  width: 100%;
  table-layout: auto;
  border-collapse: collapse;
  border-spacing: 0;
  background: transparent;
  border-radius: 0;
}

:deep(.assistant-markdown .table-scroll-wrapper thead th) {
  background: transparent;
  font-weight: 520;
  color: #56463a;
  border-bottom: 2px solid rgba(112, 76, 40, 0.98);
}

:deep(.assistant-markdown .table-scroll-wrapper th),
:deep(.assistant-markdown .table-scroll-wrapper td) {
  padding: 0.2rem 0.8rem;
  vertical-align: top;
  text-align: left;
  white-space: normal;
  word-break: break-word;
  overflow-wrap: anywhere;
  max-width: 320px;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody td) {
  border-bottom: 0;
}

:deep(.assistant-markdown .table-scroll-wrapper tbody tr:last-child td) {
  border-bottom: 1px solid rgba(232, 210, 175, 0.98);
  background-image: none;
  background-position: initial;
  background-size: auto;
  background-repeat: repeat;
}

:deep(.assistant-markdown a) {
  color: #8b5e34;
  text-decoration: underline;
  text-underline-offset: 2px;
}

:deep(.assistant-markdown blockquote) {
  margin: 0.8rem 0;
  padding: 0.2rem 0 0.2rem 0.8rem;
  color: #71665c;
  background: transparent;
  border-left: 2px solid rgba(139, 94, 52, 0.28);
  border-radius: 0;
  font-style: normal;
}

:deep(.assistant-markdown blockquote p:last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown hr) {
  margin: 1.15rem 0;
  height: 0.5px;
  border: 0;
  background: linear-gradient(90deg, transparent, rgba(198, 174, 147, 0.18), transparent);
}

:deep(.assistant-markdown img) {
  display: block;
  max-width: 100%;
  height: auto;
  border-radius: 0.55rem;
}

:deep(.assistant-markdown strong) {
  color: #1f1712;
  font-weight: 520;
}

:deep(.assistant-markdown del) {
  color: #8a7c6c;
}

:deep(.assistant-markdown input[type="checkbox"]) {
  margin: 0 0.35rem 0 0;
  transform: translateY(1px);
  accent-color: #8b5e34;
}

.assistant-plain-text {
  overflow-wrap: anywhere;
  word-break: break-word;
  line-height: 1.7;
  color: #3d342d;
}

/* 工具状态与时长容器：预留足够宽度避免 duration 出现时布局偏移 */
.conversation-tool-status {
  min-width: 5.5rem;
  text-align: right;
  justify-content: flex-end;
}

/* 面板入场与过渡：硬件加速的平滑淡入微动效 */
.conversation-tool-row,
.conversation-tool-panel,
.conversation-reasoning-panel,
.assistant-response-panel {
  will-change: opacity, transform;
}

/* wave-pds-3 §3.3：工具面板/思考面板入场 200ms → 220ms，并补 filter 消融（blur(2px)→0）。
   入场节点是一次性节点（不是每 tick 重建），允许用 filter：它是"落定"信号，不是逐字浮现。 */
.transition-item-enter {
  animation: item-smooth-appear 220ms cubic-bezier(0.23, 1, 0.32, 1);
}

/* wave-pds-3 §3.3：工具行（契约 §1 的 `data-pds-tool-row`）补同类入场消融；
   面板本身已有入场动画，行级再补一次是为了「新增一行」时只让该行显影，整块不闪。 */
.pds-tool-row-enter {
  animation: item-smooth-appear 220ms cubic-bezier(0.23, 1, 0.32, 1);
}

@keyframes item-smooth-appear {
  from {
    opacity: 0;
    filter: blur(2px);
    transform: translateY(4px);
  }
  to {
    opacity: 1;
    filter: blur(0);
    transform: translateY(0);
  }
}

/* 非流式 content 入场使用极简动画，避免 `animation-fill-mode: both` 在流式→完成过渡时
   将已渲染的可见元素重置为 opacity:0 导致闪烁。初始渲染靠 streaming char 逐字淡入完成，
   完成后不再需要额外入场动画。 */
.assistant-response-panel.motion-entrance {
  animation: panel-fade-in 0.18s ease-out;
}

@keyframes panel-fade-in {
  from {
    opacity: 0;
    transform: translateY(3px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.assistant-streaming-content {
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.7;
  transition: color 140ms ease;
  contain: layout style;
}

.assistant-streaming-markdown {
  display: block;
  white-space: normal;
  contain: layout style;
}

.assistant-streaming-content .assistant-streaming-markdown:empty {
  display: none;
}

.assistant-streaming-fade {
  display: inline;
  white-space: pre-wrap;
  will-change: opacity, transform;
}

/* wave-pds-3 §3.2：reasoning 尾巴 350ms 纯透明度 → 140ms + translateY(2px)→0，ease-out。
   注意（单一真值）：动画名由 `useStreamingPresentationState.assistantDisplayedReasoningFadeStyle`
   以**内联样式**给出，而 `<style scoped>` 里的 @keyframes 会被 Vue 重命名（加 scope hash），
   内联的全局名解析不到它。因此 `assistant-stream-fade-in` 的 keyframes 唯一真值放在
   全局 `src/styles.css`（亦为契约 §3.2 的 140ms/位移判据所在）。此处不再重复定义。 */

@media (prefers-reduced-motion: reduce) {
  .conversation-tool-panel,
  .conversation-reasoning-panel,
  .assistant-response-panel,
  .transition-item-enter,
  .pds-tool-row-enter {
    animation: none !important;
    opacity: 1 !important;
    filter: none !important;
    transform: none !important;
    transition: none !important;
  }

  .assistant-streaming-fade {
    animation: none !important;
    opacity: 1 !important;
    transform: none !important;
  }
}

.conversation-disclosure {
  color: rgb(120 113 108);
}

.conversation-disclosure-summary {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 0.5rem;
  cursor: pointer;
  list-style: none;
  font-size: 12px;
  line-height: 1.4;
  color: rgb(120 113 108);
}

.conversation-disclosure-summary::-webkit-details-marker {
  display: none;
}

.conversation-disclosure-chevron {
  transition: transform 180ms ease;
}

.conversation-disclosure[open] .conversation-disclosure-chevron {
  transform: rotate(180deg);
}

.checkpoint-icon-button {
  display: inline-flex;
  height: 1.35rem;
  width: 1.35rem;
  align-items: center;
  justify-content: center;
  border-radius: 0.25rem;
  border: none;
  background: transparent;
  color: rgb(168 162 158);
  cursor: pointer;
  transition:
    color 0.15s ease,
    background-color 0.15s ease,
    transform 0.1s ease;
}

.checkpoint-icon-button:hover {
  color: rgb(87 83 78);
  background: rgba(0, 0, 0, 0.04);
}

.checkpoint-icon-button:active {
  transform: scale(0.82);
  color: rgb(68 64 60);
}

.checkpoint-icon-button:disabled {
  cursor: not-allowed;
  opacity: 0.3;
}

.checkpoint-icon-button:disabled:hover {
  background: transparent;
  color: rgb(168 162 158);
}

.checkpoint-icon-button:disabled:active {
  transform: none;
}

/* Hover-reveal for action bars — opacity preserves layout */
.conversation-user-message .message-action-bar {
  opacity: 0;
  transition: opacity 0.2s ease;
}

.conversation-user-message:hover .message-action-bar {
  opacity: 1;
}

.conversation-agent-shell .agent-action-bar {
  opacity: 0;
  transition: opacity 0.2s ease;
}

.conversation-agent-shell:hover .agent-action-bar {
  opacity: 1;
}

.reasoning-italic {
  font-style: normal;
}

.auto-copy-pill-position {
  transform: translate(-50%, -160%);
}

.auto-copy-pill-fade-enter-active,
.auto-copy-pill-fade-leave-active {
  transition:
    opacity 160ms ease,
    transform 160ms ease;
}

.auto-copy-pill-fade-enter-from,
.auto-copy-pill-fade-leave-to {
  opacity: 0;
  transform: translate(-50%, -120%) scale(0.92);
}
</style>

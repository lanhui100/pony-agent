<script setup lang="ts">
import { ref } from "vue";
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
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
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
};

export type AgentTurnEvent =
  | { kind: "reasoning"; key: string; order: number; assistant: ChatMessage; reasoningContent: string; streaming: boolean }
  | { kind: "waiting"; key: string; order: number }
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
  shouldUseMarkdownAssistantRendering: (message: ChatMessage | null, content: string) => boolean;
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

// 整块释放后的字符错落延时：每个字符相对前一个字符延迟一步开始淡入，
// 形成"波浪式"逐字浮现效果；超过窗口后封顶，避免长文本总动画时长失控。
const STREAM_CHAR_STAGGER_STEP_MS = 12;
const STREAM_CHAR_STAGGER_CAP_MS = 480;

function charStaggerDelayMs(index: number) {
  return Math.min(index * STREAM_CHAR_STAGGER_STEP_MS, STREAM_CHAR_STAGGER_CAP_MS);
}

/** 将已按 model-hop 裁剪的可见文本拆为逐字数组，用于逐字淡入渲染 */
function streamingVisibleChars(text: string) {
  if (!text) return [];
  return text.split("");
}

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
          <div class="text-left whitespace-pre-wrap text-sm leading-6">
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
          :initial="{ opacity: 0, y: 6 }"
          :animate="{ opacity: 1, y: 0 }"
          :transition="{ duration: 0.2, ease: 'easeOut', delay: 0.32 }"
          class="conversation-disclosure conversation-reasoning-panel group p-0"
          style="contain: layout;"
        >
          <summary class="conversation-disclosure-summary">
            <div class="flex min-w-0 items-center gap-2">
              <Activity class="h-3 w-3 shrink-0 text-stone-400" />
              <span>思考过程</span>
            </div>
            <ChevronDown class="conversation-disclosure-chevron h-3.5 w-3.5 shrink-0 text-stone-400" />
          </summary>
          <div class="mt-1 pl-5 whitespace-pre-wrap break-words text-[13px] leading-[1.4] text-stone-400">
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

        <Transition v-else-if="event.kind === 'waiting'" name="assistant-waiting-signal">
          <div
            class="assistant-waiting-panel"
            role="status"
            aria-label="等待回复开始"
            data-testid="assistant-awaiting-first-signal"
          >
            <span class="assistant-waiting-dots" aria-hidden="true">
              <span
                v-for="index in 3"
                :key="index"
                class="assistant-waiting-dot"
                :style="{ animationDelay: `${(index - 1) * 150}ms` }"
              ></span>
            </span>
          </div>
        </Transition>

        <div
          v-else-if="event.kind === 'tools'"
          class="conversation-tool-panel space-y-0.5"
        >
          <div
            v-for="tool in event.tools"
            :key="tool.id"
            class="flex flex-col py-0.5 text-[12px] leading-5"
          >
            <div class="flex items-center gap-2">
              <component :is="toolIconByCanonicalName[tool.canonicalToolName ?? ''] ?? Wrench" class="h-3 w-3 shrink-0 text-stone-400" />
              <span
                v-if="tool.displayNameZh || tool.canonicalToolName || tool.toolName || tool.description"
                class="conversation-tool-name min-w-0 truncate"
                :class="tool.status === 'error' ? 'text-rose-600' : 'text-stone-400'"
              >
                {{ tool.displayNameZh || tool.canonicalToolName || tool.toolName || tool.description }}
              </span>
              <span
                v-if="tool.description && tool.description !== (tool.displayNameZh || tool.canonicalToolName || tool.toolName)"
                class="conversation-tool-detail min-w-0 truncate text-stone-400"
              >
                {{ tool.description }}
              </span>
              <span v-if="tool.count > 1" class="shrink-0 text-[11px] text-stone-300">({{ tool.count }}x)</span>
              <span class="conversation-tool-status flex shrink-0 items-center gap-1 leading-none">
                <span class="text-[11px] text-stone-400" :class="tool.durationSeconds != null ? 'visible' : 'invisible'">{{ tool.durationSeconds != null ? (tool.durationSeconds).toFixed(1) + 's' : '0.0s' }}</span>
                <LoaderCircle v-if="tool.status === 'pending'" class="h-3 w-3 animate-spin text-stone-400" />
                <Check v-else-if="tool.status === 'done'" class="h-3 w-3 text-stone-400" />
                <AlertTriangle v-else-if="tool.status === 'error'" class="h-3 w-3 shrink-0 text-rose-400" aria-label="工具调用失败" :aria-hidden="false" />
              </span>
            </div>
          </div>
        </div>

        <div
          v-else-if="event.kind === 'content'"
          class="assistant-response-panel"
        >
          <!-- 统一 shell：流式/完成共享同一外容器，避免 v-if/v-else 导致的 DOM 子树替换 -->
          <div
            class="assistant-plain-text text-sm"
            :class="assistantTone(event.assistant)"
            :data-streaming="event.streaming ? 'true' : undefined"
          >
            <div
              v-if="shouldUseMarkdownAssistantRendering(event.assistant, event.content)"
              :class="event.streaming ? 'assistant-streaming-content' : undefined"
              :data-testid="event.streaming ? 'assistant-streaming-flow' : undefined"
            >
              <MarkdownRenderer
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
            </div>
            <!-- 纯文本路径：流式逐字与完成态共用同一容器，完成时不切换 DOM 子树 -->
            <div
              v-else
              class="assistant-streaming-content"
              :data-testid="event.streaming ? 'assistant-streaming-flow' : undefined"
            >
              <template v-if="shouldUseOptimizedAssistantStreaming(event.assistant)">
                <span
                  v-for="(char, i) in streamingVisibleChars(event.content)"
                  :key="i"
                  class="assistant-streaming-char"
                  :style="{ animationDelay: `${charStaggerDelayMs(i)}ms` }"
                >{{ char }}</span>
              </template>
              <template v-else>{{ event.content }}</template>
            </div>
          </div>
        </div>

        <!-- Error detail panel (raw error for debugging) -->
        <details
          v-else-if="event.kind === 'error'"
          class="conversation-disclosure conversation-error-panel mt-3 group"
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
        class="agent-action-bar ml-auto mt-3 flex flex-wrap items-center justify-end gap-2"
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

:deep(.assistant-markdown > :first-child) {
  margin-top: 0;
}

:deep(.assistant-markdown > :last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown p) {
  margin: 0 0 1.2em;
  color: inherit;
}

:deep(.assistant-markdown p:last-child) {
  margin-bottom: 0;
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

/* v-motion 面板：预声明 transform 层以减少首次动画跳变 */
.conversation-tool-row,
.conversation-tool-panel,
.conversation-reasoning-panel,
.assistant-response-panel {
  will-change: opacity, transform;
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
}

.assistant-streaming-markdown {
  display: block;
  white-space: normal;
}

.assistant-streaming-content .assistant-streaming-markdown:empty {
  display: none;
}

.assistant-streaming-fade {
  display: inline;
  white-space: pre-wrap;
}

.assistant-streaming-char {
  display: inline;
  white-space: pre-wrap;
  animation-name: assistant-stream-char-fade-in;
  animation-duration: 180ms;
  animation-timing-function: ease-out;
  animation-fill-mode: both;
}

@keyframes assistant-stream-char-fade-in {
  from {
    opacity: 0;
    transform: translateY(-0.04em);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes assistant-stream-fade-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}

.assistant-waiting-panel {
  display: inline-flex;
  min-height: 1.7rem;
  align-items: center;
  padding: 0.1rem 0;
}

.assistant-waiting-dots {
  display: inline-flex;
  align-items: center;
  gap: 0.34rem;
}

.assistant-waiting-dot {
  width: 0.46rem;
  height: 0.46rem;
  border-radius: 9999px;
  background: rgba(87, 83, 78, 0.58);
  animation: assistant-waiting-dot-bounce 900ms cubic-bezier(0.2, 0.7, 0.35, 1) infinite both;
}

.assistant-waiting-signal-enter-active,
.assistant-waiting-signal-leave-active {
  transition:
    opacity 220ms ease,
    transform 220ms ease;
}

.assistant-waiting-signal-enter-from,
.assistant-waiting-signal-leave-to {
  opacity: 0;
  transform: translateY(0.25rem);
}

@keyframes assistant-waiting-dot-bounce {
  0%,
  70%,
  100% {
    opacity: 0.42;
    transform: translateY(0) scale(0.86);
  }

  35% {
    opacity: 1;
    transform: translateY(-0.34rem) scale(1.08);
  }
}

@media (prefers-reduced-motion: reduce) {
  .conversation-tool-panel,
  .conversation-reasoning-panel,
  .assistant-response-panel {
    animation: none !important;
    opacity: 1 !important;
    transform: none !important;
    transition: none !important;
  }

  .assistant-streaming-fade,
  .assistant-streaming-char,
  .assistant-waiting-dot {
    animation: none !important;
    opacity: 1 !important;
    transform: none !important;
  }

  .assistant-waiting-signal-enter-active,
  .assistant-waiting-signal-leave-active {
    transition: opacity 120ms ease !important;
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
  font-style: italic !important;
}
</style>

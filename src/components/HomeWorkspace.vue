<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  AlertTriangle,
  ArrowUp,
  Bot,
  Check,
  ChevronDown,
  LoaderCircle,
  UserRound,
  Wrench
} from "lucide-vue-next";
import type { ProviderReasoningEffort } from "@/types/provider";
import type { ChatMessage } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import Button from "@/components/ui/Button.vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import Switch from "@/components/ui/Switch.vue";

type TurnBucket = {
  turnId: string;
  user: ChatMessage | null;
  assistant: ChatMessage | null;
  tools: ChatMessage[];
};

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const { draftMessage, error, isSubmitting, messages, phase, sessionError, sessionOperation } = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const providerMenuOpen = ref(false);
const hoveredProviderId = ref<string | null>(null);
const reasoningMenuOpen = ref(false);
const showReasoningContent = ref(false);
const providerMenuRef = ref<HTMLElement | null>(null);
const reasoningMenuRef = ref<HTMLElement | null>(null);
const timelineScrollAreaRef = ref<{ scrollToBottom: (behavior?: ScrollBehavior) => void } | null>(null);
const bottomAnchorRef = ref<HTMLElement | null>(null);
const scrollFrameId = ref<number | null>(null);
let lastMessageSignature = "";
const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";

const currentModelSupportsReasoning = computed(
  () => currentModel.value?.capabilities?.supportsReasoning ?? false
);

const providerLabel = computed(() => {
  const providerName = currentProvider.value?.name?.trim();
  const modelName = currentModel.value?.name?.trim();

  if (providerName && modelName) {
    return `${providerName}/${modelName}`;
  }

  if (providerName) {
    return providerName;
  }

  return "选择 provider/model";
});

const reasoningLabel = computed(() => {
  if (!currentModel.value) {
    return "思考 --";
  }

  if (!currentModelSupportsReasoning.value) {
    return "思考 不支持";
  }

  return providerStore.currentReasoningEffort
    ? `思考 ${providerStore.currentReasoningEffort}`
    : "思考 默认";
});

const reasoningOptions: Array<{ label: string; value: ProviderReasoningEffort | null }> = [
  { label: "默认", value: null },
  { label: "minimal", value: "minimal" },
  { label: "low", value: "low" },
  { label: "medium", value: "medium" },
  { label: "high", value: "high" }
];

const sessionBanner = computed<{ tone: "info" | "danger"; text: string } | null>(() => {
  if (sessionOperation.value === "initializing") {
    return {
      tone: "info",
      text: "正在加载最近对话…"
    };
  }

  if (sessionOperation.value === "switching") {
    return {
      tone: "info",
      text: "正在切换对话…"
    };
  }

  if (sessionOperation.value === "deleting") {
    return {
      tone: "info",
      text: "正在删除对话并刷新会话状态…"
    };
  }

  if (sessionError.value?.trim()) {
    return {
      tone: "danger",
      text: sessionError.value.trim()
    };
  }

  return null;
});

const runtimeErrorBanner = computed(() => {
  if (phase.value !== "failed") {
    return "";
  }

  return error.value?.trim() || "当前回合执行失败。";
});

const turns = computed<TurnBucket[]>(() => {
  const buckets = new Map<string, TurnBucket>();

  for (const message of messages.value) {
    const bucket = buckets.get(message.turnId) ?? {
      turnId: message.turnId,
      user: null,
      assistant: null,
      tools: []
    };

    if (message.role === "user") {
      bucket.user = message;
    } else if (message.role === "assistant") {
      bucket.assistant = message;
    } else if (message.toolName || message.detail || message.content) {
      bucket.tools.push(message);
    }

    buckets.set(message.turnId, bucket);
  }

  return Array.from(buckets.values());
});

function formatAssistantModelLabel(modelName?: string | null) {
  return modelName?.trim() || "";
}

function formatTokenBadge(tokenCount?: number | null) {
  return `T:${tokenCount != null ? tokenCount : "--"}`;
}

function formatToolDuration(durationSeconds?: number | null) {
  if (durationSeconds == null) {
    return "--";
  }

  return `${Math.round(durationSeconds)}s`;
}

function assistantHeaderModel(message: ChatMessage | null) {
  if (!message) {
    return "";
  }

  return formatAssistantModelLabel(message.modelName);
}

function assistantReasoning(message: ChatMessage | null) {
  return message?.reasoningContent?.trim() || "";
}

function shouldShowReasoningBlock(message: ChatMessage | null) {
  if (!message || !showReasoningContent.value) {
    return false;
  }

  return message.status === "pending" || assistantReasoning(message).length > 0;
}

function reasoningPlaceholder(message: ChatMessage | null) {
  if (!message || message.status !== "pending") {
    return "";
  }

  return "正在思考...";
}

function assistantHasVisibleContent(message: ChatMessage | null) {
  return Boolean(message?.content?.trim());
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

  if (message.status === "pending") {
    return "text-stone-400";
  }

  if (message.status === "error") {
    return "text-rose-800";
  }

  return "text-stone-800";
}

function toolStatusIcon(message: ChatMessage) {
  if (message.status === "error") {
    return "error";
  }

  if (message.status === "done") {
    return "done";
  }

  return "pending";
}

function handleComposerKeydown(event: KeyboardEvent) {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    if (!isSubmitting.value) {
      runtimeStore.submitTurn();
    }
  }
}

function toggleProviderMenu() {
  providerMenuOpen.value = !providerMenuOpen.value;

  if (providerMenuOpen.value) {
    hoveredProviderId.value = currentProvider.value?.id ?? providerStore.providers[0]?.id ?? null;
    reasoningMenuOpen.value = false;
    return;
  }

  hoveredProviderId.value = null;
}

function toggleReasoningMenu() {
  if (!currentModelSupportsReasoning.value) {
    return;
  }

  reasoningMenuOpen.value = !reasoningMenuOpen.value;
  if (reasoningMenuOpen.value) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
  }
}

function selectModel(providerId: string, modelId: string) {
  providerStore.selectModel(providerId, modelId);
  providerMenuOpen.value = false;
  hoveredProviderId.value = providerId;
}

function selectReasoningEffort(value: ProviderReasoningEffort | null) {
  providerStore.setCurrentReasoningEffort(value);
  reasoningMenuOpen.value = false;
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  if (providerMenuRef.value && target && !providerMenuRef.value.contains(target)) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
  }

  if (reasoningMenuRef.value && target && !reasoningMenuRef.value.contains(target)) {
    reasoningMenuOpen.value = false;
  }
}

function queueScrollToLatestTurn(behavior: ScrollBehavior = "smooth") {
  if (scrollFrameId.value != null) {
    return;
  }

  scrollFrameId.value = window.requestAnimationFrame(async () => {
    scrollFrameId.value = null;
    await nextTick();
    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });

    if (bottomAnchorRef.value) {
      bottomAnchorRef.value.scrollIntoView({
        block: "end",
        behavior
      });
      return;
    }

    timelineScrollAreaRef.value?.scrollToBottom(behavior);
  });
}

onMounted(() => {
  if (typeof window !== "undefined") {
    showReasoningContent.value = window.localStorage.getItem(SHOW_REASONING_STORAGE_KEY) === "true";
  }
  window.addEventListener("click", handleClickOutside);
  queueScrollToLatestTurn("auto");
});

onBeforeUnmount(() => {
  window.removeEventListener("click", handleClickOutside);
  if (scrollFrameId.value != null) {
    window.cancelAnimationFrame(scrollFrameId.value);
  }
});

watch(
  () =>
    messages.value
      .map(
        (message) =>
          `${message.id}:${message.content.length}:${message.reasoningContent?.length ?? ""}:${message.status ?? ""}:${message.tokenCount ?? ""}`
      )
      .join("|"),
  (signature) => {
    const behavior: ScrollBehavior =
      signature.startsWith(lastMessageSignature) && lastMessageSignature.length > 0
        ? "auto"
        : "smooth";

    lastMessageSignature = signature;
    queueScrollToLatestTurn(behavior);
  },
  { flush: "post" }
);

watch(showReasoningContent, (value) => {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(SHOW_REASONING_STORAGE_KEY, value ? "true" : "false");
  }
});
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-t-[0.6rem]">
    <ScrollArea
      ref="timelineScrollAreaRef"
      class="min-h-0 flex-1 rounded-t-[0.6rem]"
      viewport-class="px-4 py-4 sm:px-5"
    >
      <div class="mx-auto w-full max-w-[58rem]" data-testid="workspace-content-column">
      <div v-if="sessionBanner || runtimeErrorBanner" class="mb-4 space-y-2">
        <div
          v-if="sessionBanner"
          :class="
            sessionBanner.tone === 'danger'
              ? 'border-rose-200/80 bg-rose-50 text-rose-800'
              : 'border-stone-200/80 bg-white/92 text-stone-700'
          "
          class="flex items-start gap-2 rounded-[0.55rem] border px-3 py-2 text-[12px] leading-5"
        >
          <LoaderCircle
            v-if="sessionBanner.tone === 'info'"
            class="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-stone-400"
          />
          <AlertTriangle v-else class="mt-0.5 h-3.5 w-3.5 shrink-0 text-rose-500" />
          <span>{{ sessionBanner.text }}</span>
        </div>
        <div
          v-if="runtimeErrorBanner"
          class="flex items-start gap-2 rounded-[0.55rem] border border-rose-200/80 bg-rose-50 px-3 py-2 text-[12px] leading-5 text-rose-800"
        >
          <AlertTriangle class="mt-0.5 h-3.5 w-3.5 shrink-0 text-rose-500" />
          <span>{{ runtimeErrorBanner }}</span>
        </div>
      </div>

      <TransitionGroup name="turn-flow" tag="div" class="space-y-5">
        <section v-for="turn in turns" :key="turn.turnId" class="space-y-3">
          <article v-if="turn.user" class="ml-auto w-fit max-w-[86%] sm:max-w-[68%]">
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
            </div>
          </article>

          <article v-if="turn.assistant || turn.tools.length" class="w-full px-0 py-1">
            <div class="flex items-center justify-between gap-3">
              <div :class="actorLabelClass()" class="min-w-0">
                <Bot class="h-3.5 w-3.5" />
                <span>Agent</span>
              </div>
              <div class="flex items-center gap-2 text-right normal-case tracking-normal">
                <span v-if="assistantHeaderModel(turn.assistant)" class="truncate">
                  <span class="inline-flex rounded-full border border-[#e6d7c3] bg-[#f6efe3] px-2 py-0.5 text-[10px] text-[#8b6b47]">
                    {{ assistantHeaderModel(turn.assistant) }}
                  </span>
                </span>
              </div>
            </div>
            <div class="mt-2 h-px w-full bg-stone-200/70"></div>

            <div v-if="turn.tools.length" class="mt-2 space-y-2">
              <div v-for="tool in turn.tools" :key="tool.id" class="text-[12px] leading-5 text-stone-500">
                <details class="group">
                  <summary class="list-none cursor-pointer">
                    <div class="flex items-center justify-between gap-3">
                      <div class="flex min-w-0 items-center gap-2">
                        <Wrench class="h-3.5 w-3.5 shrink-0 text-stone-400" />
                        <span class="truncate">{{ tool.toolName || "Tool" }}</span>
                        <LoaderCircle
                          v-if="toolStatusIcon(tool) === 'pending'"
                          class="h-3.5 w-3.5 shrink-0 animate-spin text-stone-400"
                        />
                        <Check v-else-if="toolStatusIcon(tool) === 'done'" class="h-3.5 w-3.5 shrink-0 text-stone-500" />
                        <span v-else class="text-[13px] leading-none text-rose-500">!</span>
                      </div>
                      <div class="flex shrink-0 items-center gap-2 text-[11px] text-stone-400">
                        <span
                          v-if="tool.tokenCount != null"
                          class="inline-flex rounded-full border border-stone-200 bg-white/70 px-2 py-0.5 text-[10px] normal-case tracking-normal text-stone-500"
                        >
                          {{ formatTokenBadge(tool.tokenCount) }}
                        </span>
                        <span v-if="tool.durationSeconds != null">{{ formatToolDuration(tool.durationSeconds) }}</span>
                        <ChevronDown class="h-3.5 w-3.5 transition group-open:rotate-180" />
                      </div>
                    </div>
                  </summary>
                  <div class="mt-1 whitespace-pre-wrap border-l border-stone-200 pl-3 text-[11px] leading-[1.3] text-stone-500">
                    {{ tool.detail || "暂无额外详情。" }}
                  </div>
                </details>
              </div>
            </div>

            <div
              v-if="turn.assistant && shouldShowReasoningBlock(turn.assistant)"
              class="mt-2"
            >
              <MarkdownRenderer
                v-if="assistantReasoning(turn.assistant)"
                :content="assistantReasoning(turn.assistant)"
                wrapper-class="assistant-markdown assistant-reasoning-markdown text-[13px]"
              />
              <p
                v-else-if="reasoningPlaceholder(turn.assistant)"
                class="assistant-reasoning"
              >
                {{ reasoningPlaceholder(turn.assistant) }}
              </p>
            </div>
            <Transition name="stream-fade" mode="out-in">
              <div
                v-if="turn.assistant && assistantHasVisibleContent(turn.assistant)"
                :key="`${turn.turnId}:${turn.assistant.content.length}:${turn.assistant.status}`"
                class="mt-2"
              >
                <MarkdownRenderer
                  :content="turn.assistant.content"
                  wrapper-class="assistant-markdown text-sm"
                  :tone-class="assistantTone(turn.assistant)"
                />
              </div>
            </Transition>
          </article>
        </section>
      </TransitionGroup>
      <div ref="bottomAnchorRef" class="h-px w-full" aria-hidden="true"></div>
      </div>
    </ScrollArea>

    <div class="px-4 py-3 sm:px-5">
      <div
        class="mx-auto w-full max-w-[58rem] rounded-[0.6rem] bg-white/76 px-4 py-3"
        data-testid="workspace-composer-shell"
      >
      <textarea
        :value="draftMessage"
        :disabled="Boolean(sessionOperation)"
        class="min-h-[82px] w-full resize-none bg-transparent px-0 py-0 text-sm leading-6 text-stone-900 outline-none placeholder:text-stone-400"
        placeholder="输入消息，按 Enter 发送，Shift+Enter 换行。"
        @input="runtimeStore.setDraftMessage(($event.target as HTMLTextAreaElement).value)"
        @keydown="handleComposerKeydown"
      />

      <div class="mt-2.5 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 pt-2">
        <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5">
          <div ref="providerMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              @click.stop="toggleProviderMenu"
            >
              <span class="truncate">{{ providerLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="providerMenuOpen"
              class="absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[15rem] rounded-[0.45rem] border border-stone-200/80 bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
            >
              <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">提供商</div>
              <div class="mx-2 border-t border-stone-200/90"></div>
              <div class="relative py-0.5">
                <button
                  v-for="provider in providerStore.providers"
                  :key="provider.id"
                  class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-700 hover:bg-[#f8f4ed]"
                  type="button"
                  @mouseenter="hoveredProviderId = provider.id"
                  @focus="hoveredProviderId = provider.id"
                >
                  <span class="truncate">{{ provider.name }}</span>
                  <div class="flex items-center gap-2">
                    <Check v-if="currentProvider?.id === provider.id" class="h-3.5 w-3.5 text-stone-700" />
                    <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                  </div>
                </button>
                <div
                  v-if="hoveredProviderId"
                  class="absolute left-full top-0 ml-1 min-w-[15rem] rounded-[0.45rem] border border-stone-200/80 bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
                >
              <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">模型</div>
                  <div class="mx-2 border-t border-stone-200/90"></div>
                  <button
                    v-for="model in providerStore.providers.find((provider) => provider.id === hoveredProviderId)?.models ?? []"
                    :key="model.id"
                    class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-600 hover:bg-[#fbf7f1]"
                    type="button"
                    @click="selectModel(hoveredProviderId, model.id)"
                  >
                    <span class="truncate">{{ model.name }}</span>
                    <Check
                      v-if="currentProvider?.id === hoveredProviderId && currentModel?.id === model.id"
                      class="h-3.5 w-3.5 text-stone-700"
                    />
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div ref="reasoningMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              :disabled="!currentModelSupportsReasoning"
              :title="currentModelSupportsReasoning ? '选择思考强度' : '当前模型不支持思考强度'"
              @click.stop="toggleReasoningMenu"
            >
              <span class="truncate">{{ reasoningLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="currentModelSupportsReasoning && reasoningMenuOpen"
              class="absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[11rem] rounded-[0.45rem] border border-stone-200/80 bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
            >
              <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">思考强度</div>
              <div class="mx-2 border-t border-stone-200/90"></div>
              <button
                v-for="option in reasoningOptions"
                :key="option.label"
                class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-700 hover:bg-[#f8f4ed]"
                type="button"
                @click="selectReasoningEffort(option.value)"
              >
                <span>{{ option.label }}</span>
                <Check
                  v-if="(providerStore.currentReasoningEffort ?? null) === option.value"
                  class="h-3.5 w-3.5 text-stone-700"
                />
              </button>
            </div>
          </div>

          <label class="composer-switch-row" title="切换是否显示模型思考过程">
            <Switch v-model="showReasoningContent" />
            <span class="composer-switch-label">显示思考</span>
          </label>
        </div>

        <Button
          class="h-8 w-8 rounded-full p-0"
          size="sm"
          :disabled="isSubmitting || Boolean(sessionOperation)"
          @click="runtimeStore.submitTurn()"
        >
          <ArrowUp class="h-3.5 w-3.5" />
        </Button>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
:deep(.markdown-body) {
  min-width: 0;
  overflow-wrap: anywhere;
  word-break: break-word;
  line-height: 1.65;
  color: #3d342d;
}

:deep(.assistant-markdown > :first-child) {
  margin-top: 0;
}

:deep(.assistant-markdown > :last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown p) {
  margin: 0 0 0.9rem;
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
  margin: 1.15rem 0 0.65rem;
  font-weight: 700;
  line-height: 1.22;
  letter-spacing: -0.015em;
  color: #241b14;
}

:deep(.assistant-markdown h1) {
  font-size: 1.4rem;
}

:deep(.assistant-markdown h2) {
  font-size: 1.22rem;
}

:deep(.assistant-markdown h3) {
  font-size: 1.08rem;
}

:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  font-size: 0.98rem;
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
  margin: 1rem 0;
  overflow-x: auto;
  border-radius: 0.9rem;
  background: linear-gradient(180deg, rgba(247, 242, 234, 0.96), rgba(241, 233, 222, 0.92));
  padding: 1rem 1.05rem;
  font-size: 0.82rem;
  line-height: 1.55;
  color: #2f261d;
}

:deep(.assistant-markdown code) {
  border-radius: 0.42rem;
  background: rgba(239, 230, 218, 0.92);
  padding: 0.08rem 0.34rem;
  font-size: 0.82em;
  color: #5b4330;
}

:deep(.assistant-markdown pre code) {
  background: transparent;
  padding: 0;
  color: inherit;
}

:deep(.assistant-markdown table) {
  display: block;
  width: 100%;
  overflow-x: auto;
  margin: 1rem 0;
  border-collapse: separate;
  border-spacing: 0;
  border-radius: 0.9rem;
  background: rgba(255, 251, 244, 0.92);
}

:deep(.assistant-markdown thead th) {
  background: rgba(244, 234, 221, 0.88);
  font-weight: 600;
  color: #56463a;
}

:deep(.assistant-markdown th),
:deep(.assistant-markdown td) {
  padding: 0.62rem 0.8rem;
  vertical-align: top;
  text-align: left;
  white-space: normal;
}

:deep(.assistant-markdown tbody tr:nth-child(even)) {
  background: rgba(246, 240, 231, 0.7);
}

:deep(.assistant-markdown a) {
  color: #8b5e34;
  text-decoration: underline;
  text-underline-offset: 2px;
}

:deep(.assistant-markdown blockquote) {
  margin: 1rem 0;
  padding: 0.8rem 1rem;
  color: #71665c;
  background: linear-gradient(135deg, rgba(245, 239, 231, 0.92), rgba(250, 246, 240, 0.86));
  border-radius: 0.95rem;
  font-style: italic;
}

:deep(.assistant-markdown blockquote p:last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown hr) {
  margin: 1.15rem 0;
  height: 1px;
  border: 0;
  background: linear-gradient(90deg, transparent, rgba(198, 174, 147, 0.9), transparent);
}

:deep(.assistant-markdown img) {
  display: block;
  max-width: 100%;
  height: auto;
  border-radius: 0.55rem;
}

:deep(.assistant-markdown strong) {
  color: #1f1712;
  font-weight: 700;
}

:deep(.assistant-markdown del) {
  color: #8a7c6c;
}

:deep(.assistant-markdown input[type="checkbox"]) {
  margin: 0 0.35rem 0 0;
  transform: translateY(1px);
  accent-color: #8b5e34;
}

.turn-flow-enter-active,
.turn-flow-leave-active {
  transition:
    opacity 220ms ease,
    transform 220ms ease;
}

.turn-flow-enter-from,
.turn-flow-leave-to {
  opacity: 0;
  transform: translateY(0.45rem);
}

.stream-fade-enter-active,
.stream-fade-leave-active {
  transition: opacity 180ms ease;
}

.stream-fade-enter-from,
.stream-fade-leave-to {
  opacity: 0;
}

.assistant-reasoning {
  margin: 0;
  color: #8d857a;
  font-size: 0.82rem;
  line-height: 1.7;
  font-style: italic;
}

:deep(.assistant-reasoning-markdown) {
  color: #8d857a;
  font-style: italic;
  line-height: 1.72;
}

:deep(.assistant-reasoning-markdown h1),
:deep(.assistant-reasoning-markdown h2),
:deep(.assistant-reasoning-markdown h3),
:deep(.assistant-reasoning-markdown h4),
:deep(.assistant-reasoning-markdown h5),
:deep(.assistant-reasoning-markdown h6),
:deep(.assistant-reasoning-markdown strong) {
  color: #746d64;
}

:deep(.assistant-reasoning-markdown pre),
:deep(.assistant-reasoning-markdown table),
:deep(.assistant-reasoning-markdown blockquote) {
  background: rgba(245, 240, 233, 0.72);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.65);
}

.composer-select-trigger {
  display: inline-flex;
  max-width: 12rem;
  align-items: center;
  gap: 0.25rem;
  background: transparent;
  padding: 0;
  font-size: 11px;
  line-height: 1.1;
  color: rgb(87 83 78);
  outline: none;
}

.composer-select-trigger:disabled {
  opacity: 0.45;
}

.composer-select-trigger:focus-visible {
  border-radius: 0.2rem;
  box-shadow: 0 0 0 2px rgba(252, 211, 77, 0.35);
}

.composer-toggle-trigger {
  display: none;
}

.composer-toggle-trigger:focus-visible {
  border-radius: 0.2rem;
  box-shadow: 0 0 0 2px rgba(252, 211, 77, 0.35);
}

.composer-switch-row {
  display: inline-flex;
  min-height: 1.5rem;
  align-items: center;
  gap: 0.5rem;
  color: rgb(87 83 78);
}

.composer-switch-label {
  font-size: 11px;
  line-height: 1.1;
}
</style>

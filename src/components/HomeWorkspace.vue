<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { ArrowUp, Bot, Check, ChevronDown, LoaderCircle, UserRound, Wrench } from "lucide-vue-next";
import type { ProviderReasoningEffort } from "@/types/provider";
import type { ChatMessage } from "@/types/runtime";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import Button from "@/components/ui/Button.vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import ScrollArea from "@/components/ui/ScrollArea.vue";

type TurnBucket = {
  turnId: string;
  user: ChatMessage | null;
  assistant: ChatMessage | null;
  tools: ChatMessage[];
};

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const { draftMessage, isSubmitting, messages } = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const providerMenuOpen = ref(false);
const modelMenuOpen = ref(false);
const reasoningMenuOpen = ref(false);
const providerMenuRef = ref<HTMLElement | null>(null);
const modelMenuRef = ref<HTMLElement | null>(null);
const reasoningMenuRef = ref<HTMLElement | null>(null);
const timelineScrollAreaRef = ref<{ scrollToBottom: (behavior?: ScrollBehavior) => void } | null>(null);
const bottomAnchorRef = ref<HTMLElement | null>(null);
const scrollFrameId = ref<number | null>(null);
let lastMessageSignature = "";

const providerLabel = computed(() => currentProvider.value?.name || "选择提供商");
const modelLabel = computed(() => currentModel.value?.name || "选择模型");
const reasoningLabel = computed(() => {
  if (!currentModel.value?.capabilities.supportsReasoning) {
    return "";
  }

  return providerStore.currentReasoningEffort ? `思考 ${providerStore.currentReasoningEffort}` : "思考 默认";
});

const reasoningOptions: Array<{ label: string; value: ProviderReasoningEffort | null }> = [
  { label: "默认", value: null },
  { label: "minimal", value: "minimal" },
  { label: "low", value: "low" },
  { label: "medium", value: "medium" },
  { label: "high", value: "high" }
];

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

function formatInlineToken(kind: "IN" | "OUT", tokenCount?: number | null) {
  return `${kind}:${tokenCount != null ? tokenCount : "--"}`;
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
    modelMenuOpen.value = false;
    reasoningMenuOpen.value = false;
  }
}

function toggleModelMenu() {
  modelMenuOpen.value = !modelMenuOpen.value;
  if (modelMenuOpen.value) {
    providerMenuOpen.value = false;
    reasoningMenuOpen.value = false;
  }
}

function toggleReasoningMenu() {
  if (!currentModel.value?.capabilities.supportsReasoning) {
    return;
  }

  reasoningMenuOpen.value = !reasoningMenuOpen.value;
  if (reasoningMenuOpen.value) {
    providerMenuOpen.value = false;
    modelMenuOpen.value = false;
  }
}

function selectProvider(providerId: string) {
  providerStore.selectProvider(providerId);
  providerMenuOpen.value = false;
  modelMenuOpen.value = false;
  reasoningMenuOpen.value = false;
}

function selectModel(modelId: string) {
  if (!currentProvider.value) {
    return;
  }

  providerStore.selectModel(currentProvider.value.id, modelId);
  modelMenuOpen.value = false;
}

function selectReasoningEffort(value: ProviderReasoningEffort | null) {
  providerStore.setCurrentReasoningEffort(value);
  reasoningMenuOpen.value = false;
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  if (providerMenuRef.value && target && !providerMenuRef.value.contains(target)) {
    providerMenuOpen.value = false;
  }

  if (modelMenuRef.value && target && !modelMenuRef.value.contains(target)) {
    modelMenuOpen.value = false;
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
  () => messages.value.map((message) => `${message.id}:${message.content.length}:${message.status ?? ""}:${message.tokenCount ?? ""}`).join("|"),
  (signature) => {
    const behavior: ScrollBehavior =
      signature.startsWith(lastMessageSignature) && lastMessageSignature.length > 0
        ? "auto"
        : "smooth";

    lastMessageSignature = signature;
    queueScrollToLatestTurn(behavior);
  }
,
  { flush: "post" }
);
</script>

<template>
  <section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.6rem] bg-[#fdfbf7]/88">
    <ScrollArea ref="timelineScrollAreaRef" class="min-h-0 flex-1" viewport-class="px-4 py-4 sm:px-5">
      <TransitionGroup name="turn-flow" tag="div" class="space-y-5">
        <section v-for="turn in turns" :key="turn.turnId" class="space-y-3">
          <article v-if="turn.user" class="ml-auto w-fit max-w-[86%] sm:max-w-[68%]">
            <div class="flex flex-col items-end">
              <div :class="actorLabelClass()" class="mb-1">
                <span>User</span>
                <UserRound class="h-3.5 w-3.5" />
              </div>
              <div :class="userShellClass()">
                <div class="whitespace-pre-wrap text-sm leading-6 text-left">
                  {{ turn.user.content }}
                </div>
              </div>
              <div
                v-if="turn.user.tokenCount != null"
                class="mt-1 w-full text-left text-[10px] normal-case tracking-normal text-stone-400"
              >
                {{ formatInlineToken("IN", turn.user.tokenCount) }}
              </div>
            </div>
          </article>

          <article v-if="turn.assistant || turn.tools.length" class="max-w-[86%] px-0 py-1 sm:max-w-[78%]">
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

            <Transition name="stream-fade" mode="out-in">
              <div v-if="turn.assistant" :key="`${turn.turnId}:${turn.assistant.content.length}:${turn.assistant.status}`" class="mt-2">
                <MarkdownRenderer class="assistant-markdown text-sm" :content="turn.assistant.content" :tone-class="assistantTone(turn.assistant)" />
              </div>
            </Transition>
            <div
              v-if="turn.assistant && turn.assistant.tokenCount != null"
              class="mt-1 w-full text-right text-[10px] normal-case tracking-normal text-stone-400"
            >
              {{ formatInlineToken("OUT", turn.assistant.tokenCount) }}
            </div>
          </article>
        </section>
      </TransitionGroup>
      <div ref="bottomAnchorRef" class="h-px w-full" aria-hidden="true"></div>
    </ScrollArea>

    <div class="border-t border-stone-200/70 bg-white/76 px-4 py-3 sm:px-5">
      <textarea
        :value="draftMessage"
        class="min-h-[82px] w-full resize-none bg-transparent px-0 py-0 text-sm leading-6 text-stone-900 outline-none placeholder:text-stone-400"
        placeholder="输入消息，按 Enter 发送，Shift+Enter 换行。"
        @input="runtimeStore.setDraftMessage(($event.target as HTMLTextAreaElement).value)"
        @keydown="handleComposerKeydown"
      />

      <div class="mt-3 flex flex-wrap items-end justify-between gap-3 border-t border-stone-100 pt-2.5">
        <div class="flex flex-wrap items-end gap-3">
          <div ref="providerMenuRef" class="relative">
            <button
              class="inline-flex min-w-[6.2rem] items-center gap-1 bg-transparent p-0 text-[11px] leading-4 text-stone-600 outline-none"
              type="button"
              @click.stop="toggleProviderMenu"
            >
              <span class="truncate text-[11px] leading-4">{{ providerLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="providerMenuOpen"
              class="absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[12rem] rounded-[0.45rem] border border-stone-200/80 bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
            >
              <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">提供方</div>
              <div class="mx-2 border-t border-stone-200/90"></div>
              <button
                v-for="provider in providerStore.providers"
                :key="provider.id"
                class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-700 hover:bg-[#f8f4ed]"
                type="button"
                @click="selectProvider(provider.id)"
              >
                <span>{{ provider.name }}</span>
                <Check v-if="currentProvider?.id === provider.id" class="h-3.5 w-3.5 text-stone-700" />
              </button>
            </div>
          </div>

          <div ref="modelMenuRef" class="relative">
            <button
              :disabled="!currentProvider"
              class="inline-flex min-w-[7.4rem] items-center gap-1 bg-transparent p-0 text-[11px] leading-4 text-stone-600 outline-none"
              type="button"
              @click.stop="toggleModelMenu"
            >
              <span class="truncate text-[11px] leading-4">{{ modelLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="modelMenuOpen"
              class="absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[13rem] rounded-[0.45rem] border border-stone-200/80 bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
            >
              <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">模型</div>
              <div class="mx-2 border-t border-stone-200/90"></div>
              <template v-if="currentProvider">
                <button
                  v-for="model in currentProvider.models"
                  :key="model.id"
                  class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-700 hover:bg-[#f8f4ed]"
                  type="button"
                  @click="selectModel(model.id)"
                >
                  <span>{{ model.name }}</span>
                  <Check v-if="currentModel?.id === model.id" class="h-3.5 w-3.5 text-stone-700" />
                </button>
              </template>
              <div v-else class="px-3 py-2 text-stone-500">
                暂无可用模型
              </div>
            </div>
          </div>

          <div v-if="currentModel?.capabilities.supportsReasoning" ref="reasoningMenuRef" class="relative">
            <button
              class="inline-flex min-w-[7.4rem] items-center gap-1 bg-transparent p-0 text-[11px] leading-4 text-stone-600 outline-none"
              type="button"
              @click.stop="toggleReasoningMenu"
            >
              <span class="truncate text-[11px] leading-4">{{ reasoningLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="reasoningMenuOpen"
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
        </div>

        <Button
          class="h-8 w-8 rounded-full p-0"
          size="sm"
          :disabled="isSubmitting"
          @click="runtimeStore.submitTurn()"
        >
          <ArrowUp class="h-3.5 w-3.5" />
        </Button>
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
}

:deep(.assistant-markdown p) {
  margin: 0 0 0.85rem;
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
  margin: 1rem 0 0.55rem;
  font-weight: 700;
  line-height: 1.25;
  color: #2f261d;
}

:deep(.assistant-markdown h1) {
  font-size: 1.28rem;
}

:deep(.assistant-markdown h2) {
  font-size: 1.16rem;
}

:deep(.assistant-markdown h3) {
  font-size: 1.07rem;
}

:deep(.assistant-markdown h4),
:deep(.assistant-markdown h5),
:deep(.assistant-markdown h6) {
  font-size: 0.98rem;
}

:deep(.assistant-markdown ul),
:deep(.assistant-markdown ol) {
  margin: 0.45rem 0 0.85rem 1.35rem;
  padding: 0;
}

:deep(.assistant-markdown li + li) {
  margin-top: 0.3rem;
}

:deep(.assistant-markdown li > p) {
  margin: 0.35rem 0;
}

:deep(.assistant-markdown pre) {
  margin: 0.9rem 0;
  overflow-x: auto;
  border: 1px solid #e3d7c6;
  border-radius: 0.75rem;
  background: #f7f2ea;
  padding: 0.95rem 1rem;
  font-size: 0.82rem;
  line-height: 1.45;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.55);
}

:deep(.assistant-markdown code) {
  border-radius: 0.32rem;
  background: #f3eee6;
  padding: 0.08rem 0.32rem;
  font-size: 0.82em;
}

:deep(.assistant-markdown pre code) {
  background: transparent;
  padding: 0;
}

:deep(.assistant-markdown table) {
  display: block;
  width: 100%;
  overflow-x: auto;
  margin: 0.95rem 0;
  border-collapse: separate;
  border-spacing: 0;
  border: 1px solid #e2d6c6;
  border-radius: 0.75rem;
  background: #fffdf8;
}

:deep(.assistant-markdown thead th) {
  background: #f5ede1;
  font-weight: 600;
}

:deep(.assistant-markdown th),
:deep(.assistant-markdown td) {
  padding: 0.55rem 0.7rem;
  border-right: 1px solid #e9ded1;
  border-bottom: 1px solid #e9ded1;
  vertical-align: top;
  text-align: left;
  white-space: normal;
}

:deep(.assistant-markdown tr:last-child td),
:deep(.assistant-markdown tr:last-child th) {
  border-bottom: 0;
}

:deep(.assistant-markdown th:last-child),
:deep(.assistant-markdown td:last-child) {
  border-right: 0;
}

:deep(.assistant-markdown tbody tr:nth-child(even)) {
  background: rgba(245, 237, 225, 0.42);
}

:deep(.assistant-markdown a) {
  color: #8b5e34;
  text-decoration: underline;
  text-underline-offset: 2px;
}

:deep(.assistant-markdown blockquote) {
  margin: 0.9rem 0;
  border-left: 2px solid #d8c7b2;
  padding: 0.1rem 0 0.1rem 0.9rem;
  color: #6b6257;
  background: rgba(243, 238, 230, 0.55);
  border-radius: 0 0.5rem 0.5rem 0;
}

:deep(.assistant-markdown hr) {
  margin: 1rem 0;
  border: 0;
  border-top: 1px solid #e4d6c2;
}

:deep(.assistant-markdown img) {
  display: block;
  max-width: 100%;
  height: auto;
  border-radius: 0.55rem;
}

:deep(.assistant-markdown strong) {
  color: #1f1712;
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
</style>

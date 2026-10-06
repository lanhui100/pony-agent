<script setup lang="ts">
import { computed, ref } from "vue";
import { LoaderCircle, MessageSquareWarning } from "lucide-vue-next";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

/**
 * PA-114: 聊天/轨迹内 Ask 工具调用专属卡片。
 *
 * 结构化工具调用投影：WorkspaceTurnItem 的 merged 行只保证
 * toolName/canonicalToolName/displayNameZh/description，`callId`/`runId`/`arguments`
 * 来自 trace 投影时可能缺失，故全部可选。
 */
export type AskUserToolCall = {
  id?: string;
  toolName: string;
  canonicalToolName?: string | null;
  displayNameZh?: string | null;
  description?: string;
  callId?: string | null;
  runId?: string | null;
  arguments?: Record<string, unknown> | null;
  status?: string;
};

const props = defineProps<{ tool: AskUserToolCall }>();

const askStore = useAskStore();
const typedAnswer = ref("");

/** 按 callId/runId 匹配当前 pending ask（契约 F1-3）。两个标识都存在时必须全部相等，
 * 避免默认 runId 巧合造成误配（红相 waiting 用例：callId 不同即不算匹配）。 */
const matchingPending = computed<PendingAsk | null>(() => {
  const callId = props.tool.callId ?? null;
  const runId = props.tool.runId ?? null;
  return (
    askStore.pendingAsks.find((ask) => {
      if (callId != null && runId != null) {
        return ask.callId === callId && ask.runId === runId;
      }
      const callMatches = callId == null || ask.callId === callId;
      const runMatches = runId == null || ask.runId === runId;
      return callMatches && runMatches;
    }) ?? null
  );
});

/** 问题文本：存在匹配 pending ask 时以其 prompt 为准（红相 F1-2 用 pending prompt 覆盖
 * 工具参数）；否则按 tool 参数 question → text → prompt，最终回退 description。 */
const questionText = computed(() => {
  const pendingPrompt = matchingPending.value?.prompt?.trim();
  if (pendingPrompt) {
    return pendingPrompt;
  }
  const args = props.tool.arguments ?? null;
  if (args) {
    for (const key of ["question", "text", "prompt"] as const) {
      const value = args[key];
      if (typeof value === "string" && value.trim()) {
        return value;
      }
    }
  }
  return props.tool.description ?? "";
});

const optionValues = computed<string[]>(() => {
  const raw = matchingPending.value?.options;
  if (!Array.isArray(raw)) {
    return [];
  }
  return raw
    .map((item) =>
      typeof item === "string" ? item : typeof item === "number" ? String(item) : null
    )
    .filter((item): item is string => item != null);
});

const busy = computed(() => {
  const pending = matchingPending.value;
  if (!pending) {
    return false;
  }
  return (
    askStore.answeringRequestId === pending.requestId ||
    askStore.cancellingRequestId === pending.requestId
  );
});

async function answerWithOption(value: string) {
  const pending = matchingPending.value;
  if (!pending || busy.value) {
    return;
  }
  await askStore.answer(pending, value);
}

async function answerTyped() {
  const pending = matchingPending.value;
  if (!pending || busy.value) {
    return;
  }
  const value = typedAnswer.value.trim();
  await askStore.answer(pending, value || null);
}

async function cancelAsk() {
  const pending = matchingPending.value;
  if (!pending || busy.value) {
    return;
  }
  await askStore.cancel(pending);
}
</script>

<template>
  <div class="flex flex-col py-0.5 text-[12px] leading-5">
    <div class="flex items-center gap-2">
      <MessageSquareWarning class="h-3 w-3 shrink-0 text-amber-500" />
      <span
        v-if="tool.displayNameZh || tool.canonicalToolName || tool.toolName"
        class="conversation-tool-name shrink-0 text-stone-400"
      >
        {{ tool.displayNameZh || tool.canonicalToolName || tool.toolName }}
      </span>
      <LoaderCircle
        v-if="busy"
        class="h-3 w-3 animate-spin text-amber-500"
        aria-label="正在提交回答"
      />
    </div>

    <p
      v-if="questionText"
      class="mt-1 whitespace-pre-wrap break-words text-[12px] leading-[1.5] text-stone-700"
      data-testid="ask-user-question"
    >
      {{ questionText }}
    </p>

    <template v-if="matchingPending">
      <div
        v-if="optionValues.length > 0"
        class="mt-1.5 flex flex-wrap gap-1.5"
      >
        <button
          v-for="option in optionValues"
          :key="option"
          type="button"
          data-testid="ask-user-option"
          :disabled="busy"
          class="rounded-full border border-amber-300/60 bg-white/80 px-2.5 py-1 text-[11px] font-medium text-stone-700 transition hover:bg-amber-100 disabled:pointer-events-none disabled:opacity-50"
          @click="answerWithOption(option)"
        >
          {{ option }}
        </button>
      </div>

      <div class="mt-1.5 flex items-center gap-2">
        <input
          v-model="typedAnswer"
          type="text"
          data-testid="ask-user-typed-input"
          :disabled="busy"
          :placeholder="questionText || '输入回答…'"
          class="h-8 min-w-0 flex-1 rounded-[0.3rem] border border-stone-200 bg-white/80 px-2 text-[12px] text-stone-700 outline-none transition placeholder:text-stone-300 focus:border-amber-300 disabled:pointer-events-none disabled:opacity-50"
        />
        <button
          type="button"
          data-testid="ask-user-answer"
          :disabled="busy || !typedAnswer.trim()"
          class="inline-flex h-8 items-center gap-1 rounded-[0.3rem] border border-amber-300/70 bg-amber-50 px-2.5 text-[11px] font-medium text-amber-800 transition hover:bg-amber-100 disabled:pointer-events-none disabled:opacity-50"
          @click="answerTyped"
        >
          <LoaderCircle v-if="busy && askStore.answeringRequestId === matchingPending.requestId" class="h-3 w-3 animate-spin" />
          回答
        </button>
        <button
          type="button"
          data-testid="ask-user-cancel"
          :disabled="busy"
          class="inline-flex h-8 items-center gap-1 rounded-[0.3rem] px-2 text-[11px] text-stone-500 transition hover:bg-stone-100 disabled:pointer-events-none disabled:opacity-50"
          @click="cancelAsk"
        >
          <LoaderCircle v-if="busy && askStore.cancellingRequestId === matchingPending.requestId" class="h-3 w-3 animate-spin" />
          取消
        </button>
      </div>
    </template>

    <div
      v-else
      class="mt-1 flex items-center gap-1.5 text-stone-400"
      data-testid="ask-user-waiting"
    >
      <LoaderCircle class="h-3 w-3 animate-spin" />
      <span>等待用户回答…</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { AlertTriangle, Check, LoaderCircle, MessageSquareWarning } from "lucide-vue-next";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

/**
 * PA-114: 聊天/轨迹内 Ask 工具调用专属卡片。
 *
 * 结构化工具调用投影：WorkspaceTurnItem 的 merged 行只保证
 * toolName/canonicalToolName/displayNameZh/description/status；`callId`/`runId`/
 * `arguments`/`argumentsText` 来自 trace 投影时可能缺失，故全部可选。
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
  argumentsText?: string | null;
  status?: string;
};

const props = defineProps<{ tool: AskUserToolCall }>();

const askStore = useAskStore();
const typedAnswer = ref("");

/** 解析工具调用参数 JSON 字符串（后端 arguments_text），失败返回 null。 */
function parseArgumentsText(text: string | null | undefined): Record<string, unknown> | null {
  if (!text) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    return parsed && typeof parsed === "object" ? (parsed as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/**
 * 按 callId 精确绑定 pending ask（第二轮修复）：callId 缺失时不得通配——绝不命中
 * pendingAsks[0]；`tool.runId` 非空时要求 `ask.runId === tool.runId`。
 */
const matchingPending = computed<PendingAsk | null>(() => {
  const callId = props.tool.callId ?? null;
  const runId = props.tool.runId ?? null;
  if (callId == null || callId.trim() === "") {
    return null;
  }
  return (
    askStore.pendingAsks.find((ask) => {
      if (ask.callId !== callId) {
        return false;
      }
      if (runId != null && runId.trim() !== "" && ask.runId !== runId) {
        return false;
      }
      return true;
    }) ?? null
  );
});

/** 问题文本：匹配 pending ask 的 prompt → tool.arguments 对象 → argumentsText JSON →
 * 最终回退 description（红相 F1-2 全部用例 + 真实投影 arguments_text 双通道）。 */
const questionText = computed(() => {
  const pendingPrompt = matchingPending.value?.prompt?.trim();
  if (pendingPrompt) {
    return pendingPrompt;
  }
  const argumentSources = [
    props.tool.arguments ?? null,
    parseArgumentsText(props.tool.argumentsText ?? null)
  ];
  for (const args of argumentSources) {
    if (!args) {
      continue;
    }
    for (const key of ["question", "text", "prompt"] as const) {
      const value = args[key];
      if (typeof value === "string" && value.trim()) {
        return value;
      }
    }
  }
  return props.tool.description ?? "";
});

/**
 * 无匹配 pending 时的终态分支：done → 静态"已回答/已完成"；error → 失败态；
 * 其余（undefined/pending/running）→ ask-user-waiting 待命态（保留 spinner）。
 */
const terminalState = computed<"waiting" | "done" | "error" | null>(() => {
  if (matchingPending.value) {
    return null;
  }
  const status = props.tool.status;
  if (status === "done") {
    return "done";
  }
  if (status === "error") {
    return "error";
  }
  return "waiting";
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

/** 绑定 ask 变化（换 callId/新 pending）时清空脏输入，避免旧回答误提交（T4）。 */
watch(
  () => matchingPending.value?.requestId ?? null,
  () => {
    typedAnswer.value = "";
  }
);

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

    <template v-else>
      <div
        v-if="terminalState === 'waiting'"
        class="mt-1 flex items-center gap-1.5 text-stone-400"
        data-testid="ask-user-waiting"
      >
        <LoaderCircle class="h-3 w-3 animate-spin" />
        <span>等待用户回答…</span>
      </div>
      <div
        v-else-if="terminalState === 'done'"
        class="mt-1 flex items-center gap-1.5 text-stone-400"
        data-testid="ask-user-done"
      >
        <Check class="h-3 w-3" />
        <span>已回答 / 已完成</span>
      </div>
      <div
        v-else
        class="mt-1 flex items-center gap-1.5 text-rose-500"
        data-testid="ask-user-error"
      >
        <AlertTriangle class="h-3 w-3 shrink-0" />
        <span>提问失败</span>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive } from "vue";
import { storeToRefs } from "pinia";
import { AlertTriangle, LoaderCircle, MessageSquareWarning } from "lucide-vue-next";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";

const askStore = useAskStore();
const { pendingAsks, answeringRequestId, cancellingRequestId, error, loading } =
  storeToRefs(askStore);
const drafts = reactive<Record<string, string>>({});

const visibleAsks = computed(() => pendingAsks.value);

/**
 * ADR 0074: there is no countdown and no "expired" state. Whether a card may be answered is
 * decided by the host lifecycle alone (it only lists `Pending` requests), so the card never
 * pre-judges a submission against the local wall clock — a rejected CAS turns into the shared
 * Chinese notice and the card disappears on the next list refresh.
 */
function askIsBusy(requestId: string) {
  return (
    answeringRequestId.value === requestId || cancellingRequestId.value === requestId
  );
}

/** Only an in-flight IPC locks the controls; a card is never locked by elapsed time. */
function askIsLocked(ask: PendingAsk) {
  return askIsBusy(ask.requestId);
}

function answerOptions(ask: PendingAsk): string[] {
  const options = ask.options;
  if (!Array.isArray(options)) {
    return [];
  }
  return options
    .map((option) => {
      if (typeof option === "string") {
        return option;
      }
      if (typeof option === "number") {
        return String(option);
      }
      return null;
    })
    .filter((value): value is string => value != null);
}

async function answerWithOption(ask: PendingAsk, value: string) {
  drafts[ask.requestId] = value;
  await askStore.answer(ask, value);
}

async function answerTyped(ask: PendingAsk) {
  const value = drafts[ask.requestId]?.trim();
  await askStore.answer(ask, value || null);
}

async function ignoreAsk(ask: PendingAsk) {
  await askStore.ignore(ask);
}

/**
 * When the question was raised (contract §6: the card shows creation time instead of a deadline).
 *
 * `createdAtMs` is added to the frozen `PendingAsk` type by the protocol freeze task; read it
 * defensively so this component never blocks that change and simply renders nothing when absent.
 */
function askCreatedAtLabel(ask: PendingAsk): string {
  const createdAtMs = (ask as { createdAtMs?: number | null }).createdAtMs;
  if (typeof createdAtMs !== "number" || createdAtMs <= 0) {
    return "";
  }
  const date = new Date(createdAtMs);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `提问于 ${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function askContextLabel(ask: PendingAsk) {
  const parts: string[] = [];
  if (ask.sessionId) {
    parts.push(`会话 ${ask.sessionId.slice(0, 12)}`);
  }
  if (ask.runId) {
    parts.push(`运行 ${ask.runId.slice(0, 12)}`);
  }
  return parts.join(" · ");
}

onMounted(() => {
  void askStore.refresh();
  askStore.startPolling();
  void askStore.startWatchingEvents();
});

onBeforeUnmount(() => {
  askStore.stopPolling();
  askStore.stopWatchingEvents();
});
</script>

<template>
  <div v-if="visibleAsks.length > 0" class="mx-auto w-full max-w-[46.4rem] px-4 pb-3 sm:px-5" data-testid="ask-panel">
    <div class="space-y-2">
      <div
        v-for="ask in visibleAsks"
        :key="ask.requestId"
        class="rounded-[0.6rem] border border-amber-200/70 bg-[#fdf6e7] px-4 py-3 shadow-[0_2px_10px_rgba(60,40,20,0.05)]"
        data-testid="ask-card"
      >
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.16em] text-amber-800/80">
            <MessageSquareWarning class="h-3.5 w-3.5" />
            <span>待确认请求</span>
            <span class="rounded-full bg-amber-200/60 px-1.5 py-px font-mono text-[9px] text-amber-900">
              v{{ ask.version }}
            </span>
          </div>
          <div class="flex items-center gap-2 text-[10px] leading-4 text-stone-500">
            <span v-if="askContextLabel(ask)" class="truncate">{{ askContextLabel(ask) }}</span>
            <span v-if="askCreatedAtLabel(ask)" class="shrink-0 tabular-nums" data-testid="ask-created-at">
              {{ askCreatedAtLabel(ask) }}
            </span>
          </div>
        </div>

        <p class="mt-2 text-[13px] leading-[1.6] text-stone-800" data-testid="ask-prompt">
          {{ ask.prompt || "请回答以下问题。" }}
        </p>

        <div v-if="answerOptions(ask).length > 0" class="mt-2 flex flex-wrap gap-1.5" data-testid="ask-options">
          <button
            v-for="option in answerOptions(ask)"
            :key="option"
            type="button"
            class="rounded-full border border-amber-300/60 bg-white/80 px-2.5 py-1 text-[11px] font-medium text-stone-700 transition hover:bg-amber-100 disabled:pointer-events-none disabled:opacity-50"
            :disabled="askIsLocked(ask)"
            @click="answerWithOption(ask, option)"
          >
            {{ option }}
          </button>
        </div>

        <div class="mt-2 flex items-center gap-2" data-testid="ask-answer-row">
          <Input
            v-model="drafts[ask.requestId]"
            class="h-9 flex-1 bg-white/80 text-[12px]"
            :placeholder="ask.prompt || '输入回答…'"
            :disabled="askIsLocked(ask)"
            data-testid="ask-answer-input"
          />
          <Button
            size="sm"
            :disabled="askIsLocked(ask)"
            data-testid="ask-answer-submit"
            @click="answerTyped(ask)"
          >
            <LoaderCircle v-if="answeringRequestId === ask.requestId" class="mr-1 h-3 w-3 animate-spin" />
            回答
          </Button>
          <Button
            size="sm"
            variant="ghost"
            :disabled="askIsLocked(ask)"
            data-testid="ask-ignore-submit"
            @click="ignoreAsk(ask)"
          >
            <LoaderCircle v-if="cancellingRequestId === ask.requestId" class="mr-1 h-3 w-3 animate-spin" />
            忽略此提问
          </Button>
        </div>
      </div>

      <div v-if="error" class="flex items-start gap-1.5 text-[11px] leading-4 text-rose-700" data-testid="ask-error">
        <AlertTriangle class="mt-0.5 h-3 w-3 shrink-0 text-rose-500" />
        <span>{{ error }}</span>
      </div>

      <div v-if="loading && visibleAsks.length === 0" class="px-1 text-[10px] leading-5 text-stone-400">
        正在刷新待确认请求…
      </div>
    </div>
  </div>
</template>

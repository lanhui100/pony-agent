<script setup lang="ts">
import { computed } from "vue";
import { storeToRefs } from "pinia";
import {
  Target,
  Pause,
  Play,
  CheckCircle2,
  AlertCircle,
  LoaderCircle,
} from "lucide-vue-next";
import { useGoalStore } from "@/stores/goal";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";

const props = withDefaults(
  defineProps<{
    sessionId?: string;
  }>(),
  {
    sessionId: "default",
  }
);

const goalStore = useGoalStore();
const { currentGoal, updating, isBlocked, isActive, isPaused, isCompleted } =
  storeToRefs(goalStore);

const phaseVariant = computed(() => {
  switch (currentGoal.value?.phase) {
    case "active":
      return "success";
    case "paused":
      return "warning";
    case "blocked":
      return "danger";
    case "completed":
      return "secondary";
    default:
      return "secondary";
  }
});

const phaseLabel = computed(() => {
  switch (currentGoal.value?.phase) {
    case "active":
      return "活跃中";
    case "paused":
      return "已暂停";
    case "blocked":
      return "已阻断";
    case "completed":
      return "已达成";
    default:
      return currentGoal.value?.phase || "无目标";
  }
});

async function handlePause() {
  await goalStore.pause(props.sessionId);
}

async function handleResume() {
  await goalStore.resume(props.sessionId);
}

async function handleComplete() {
  await goalStore.complete(props.sessionId);
}
</script>

<template>
  <div
    v-if="currentGoal"
    class="flex flex-col gap-1.5 border-b border-stone-200/80 bg-stone-50/90 px-4 py-2.5 backdrop-blur dark:border-stone-800 dark:bg-stone-900/90"
    data-testid="goal-bar"
  >
    <div class="flex items-center justify-between gap-3">
      <!-- 目标主体信息 -->
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <Target class="h-4 w-4 shrink-0 text-amber-500 dark:text-amber-400" />
        <span
          class="truncate text-xs font-semibold text-stone-800 dark:text-stone-200"
          :title="currentGoal.objective"
        >
          {{ currentGoal.objective }}
        </span>
        <Badge :variant="phaseVariant" class="shrink-0 text-[10px] px-1.5 py-0.5">
          {{ phaseLabel }}
        </Badge>
        <span class="shrink-0 text-[11px] text-stone-500 dark:text-stone-400">
          轮次: {{ currentGoal.roundsStarted }} / {{ currentGoal.maxGoalRounds }}
        </span>
      </div>

      <!-- 控制动作按钮组 -->
      <div class="flex shrink-0 items-center gap-1.5">
        <Button
          v-if="isActive"
          size="sm"
          variant="outline"
          class="h-6 gap-1 px-2 text-[11px]"
          :disabled="updating"
          @click="handlePause"
        >
          <LoaderCircle v-if="updating" class="h-3 w-3 animate-spin" />
          <Pause v-else class="h-3 w-3 text-stone-500" />
          暂停
        </Button>

        <Button
          v-if="isPaused || isBlocked"
          size="sm"
          variant="outline"
          class="h-6 gap-1 px-2 text-[11px]"
          :disabled="updating"
          @click="handleResume"
        >
          <LoaderCircle v-if="updating" class="h-3 w-3 animate-spin" />
          <Play v-else class="h-3 w-3 text-emerald-600 dark:text-emerald-400" />
          继续
        </Button>

        <Button
          v-if="!isCompleted"
          size="sm"
          variant="outline"
          class="h-6 gap-1 px-2 text-[11px]"
          :disabled="updating"
          @click="handleComplete"
        >
          <CheckCircle2 class="h-3 w-3 text-stone-500" />
          达成
        </Button>
      </div>
    </div>

    <!-- 阻断信息提示条 -->
    <div
      v-if="isBlocked && currentGoal.blocked_reason"
      class="flex items-center gap-1.5 rounded bg-red-50 px-2 py-1 text-[11px] text-red-700 dark:bg-red-950/40 dark:text-red-300"
      data-testid="goal-blocked-banner"
    >
      <AlertCircle class="h-3.5 w-3.5 shrink-0 text-red-500" />
      <span class="truncate">阻断原因：{{ currentGoal.blocked_reason }}</span>
    </div>
  </div>
</template>

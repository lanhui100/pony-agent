<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  Circle,
  ChevronRight,
  ListChecks,
  LoaderCircle,
  Play,
  ShieldCheck
} from "lucide-vue-next";
import { usePlanStore } from "@/stores/plan";
import Button from "@/components/ui/Button.vue";
import Badge from "@/components/ui/Badge.vue";

const props = defineProps<{
  sessionId: string;
  open: boolean;
}>();

const emit = defineEmits<{ toggle: [] }>();

const planStore = usePlanStore();
const { mode, todos, loading, todosLoading, error, todoProgress } = storeToRefs(planStore);

const reviewing = ref(false);

/**
 * 折叠头角标：**只显示待办任务的 x/x 进度**。
 *
 * 旧实现是 `plans.length + todos.length`——把「计划份数」与「待办条数」两个量纲相加，
 * 数字没有任何意义（ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）。Plan 已收敛
 * 为会话级协作模式（不是任务清单），任务进度只来自 Todo。
 */
const taskBadge = computed(() => todoProgress.value.total > 0 ? todoProgress.value.completed + "/" + todoProgress.value.total : "");

const modeLabel = computed(() => {
  if (mode.value.planApproved) return "已批准";
  if (mode.value.active) return "规划中 · 写/执行锁定";
  return "未激活";
});

const modeVariant = computed<"secondary" | "warning" | "success">(() => {
  if (mode.value.planApproved) return "success";
  if (mode.value.active) return "warning";
  return "secondary";
});

const planLines = computed(() => {
  const body = (mode.value.plan ?? "").trim();
  return body ? body.split("\n") : [];
});

const todoSummary = computed(() => {
  const { completed, inProgress, pending, total } = todoProgress.value;
  if (total === 0) return "暂无任务";
  const parts = [completed + "/" + total + " 已完成"];
  if (inProgress > 0) parts.push(inProgress + " 进行中");
  if (pending > 0) parts.push(pending + " 待处理");
  return parts.join(" · ");
});

async function refresh() {
  if (!props.sessionId) return;
  await Promise.allSettled([
    planStore.fetchMode(props.sessionId),
    planStore.listTodos(props.sessionId)
  ]);
}

async function toggleMode() {
  if (!props.sessionId) return;
  await planStore.setMode(props.sessionId, !mode.value.active);
}

async function review(approved: boolean) {
  if (!props.sessionId || reviewing.value) return;
  reviewing.value = true;
  try {
    await planStore.reviewPlan(props.sessionId, approved);
  } finally {
    reviewing.value = false;
  }
}

onMounted(refresh);
watch(() => props.sessionId, refresh);
watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) void refresh();
  }
);
</script>

<template>
  <section class="collapsible-shell border-b border-stone-200/60 pb-4" :data-open="open">
    <button
      class="flex w-full items-center justify-between gap-3 text-left"
      type="button"
      data-testid="plan-panel-toggle"
      @click="emit('toggle')"
    >
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <ListChecks class="h-3.5 w-3.5" />
        <span>计划与任务</span>
      </div>
      <div class="flex items-center gap-2">
        <!-- 任务进度 x/x（独立计数，不与 Plan 混加） -->
        <span
          v-if="taskBadge"
          class="text-[10px] leading-[1.2] text-stone-400"
          data-testid="plan-panel-task-progress"
        >
          {{ taskBadge }}
        </span>
        <ChevronRight
          class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200"
          :class="{ 'rotate-90': open }"
        />
      </div>
    </button>

    <div class="collapsible-body">
      <div class="collapsible-content mt-2 space-y-2">
        <div v-if="error" class="flex items-start gap-1.5 text-[11px] leading-4 text-rose-700">
          <AlertTriangle class="mt-0.5 h-3 w-3 shrink-0 text-rose-500" />
          <span>{{ error }}</span>
        </div>

        <!-- Plan 模式：状态 + 方案正文（不再是步骤清单） -->
        <div
          class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
          data-testid="plan-mode-card"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="flex min-w-0 items-center gap-1.5">
              <ShieldCheck class="h-3 w-3 shrink-0 text-stone-500" />
              <span class="truncate text-[12px] font-medium text-stone-800">Plan 模式</span>
              <Badge :variant="modeVariant">{{ modeLabel }}</Badge>
            </div>
            <Button
              size="sm"
              variant="outline"
              class="h-5 shrink-0 px-1.5 text-[10px]"
              data-testid="plan-mode-toggle"
              @click="toggleMode"
            >
              {{ mode.active ? "退出" : "进入" }}
            </Button>
          </div>

          <div
            v-if="planLines.length > 0"
            class="mt-2 space-y-0.5 text-[11px] leading-4 text-stone-600"
            data-testid="plan-mode-body"
          >
            <div v-for="(line, index) in planLines" :key="index" class="whitespace-pre-wrap break-words">
              {{ line }}
            </div>
          </div>
          <div v-else class="mt-1 px-0.5 text-[10px] leading-5 text-stone-400">
            模型尚未提交方案（ExitPlanMode 后出现在这里）
          </div>

          <!-- 方案审批：批准执行 / 继续规划（复用同一 Ask 交互通道） -->
          <div
            v-if="mode.active && planLines.length > 0 && !mode.planApproved"
            class="mt-2 flex items-center gap-1.5"
            data-testid="plan-review-actions"
          >
            <Button
              size="sm"
              variant="outline"
              class="h-5 px-1.5 text-[10px] text-emerald-700 border-emerald-300 hover:bg-emerald-50"
              :disabled="reviewing"
              data-testid="plan-approve-execution"
              @click="review(true)"
            >
              <LoaderCircle v-if="reviewing" class="mr-1 h-2.5 w-2.5 animate-spin" />
              <Play v-else class="mr-1 h-2.5 w-2.5 fill-current" />
              批准执行
            </Button>
            <Button
              size="sm"
              variant="ghost"
              class="h-5 px-1.5 text-[10px]"
              :disabled="reviewing"
              data-testid="plan-keep-planning"
              @click="review(false)"
            >
              继续规划
            </Button>
          </div>
        </div>

        <!-- 任务（Todo）：独立 x/x 进度 -->
        <div class="rounded-[0.55rem] border border-stone-200/80 px-3 py-2">
          <div class="flex items-center justify-between gap-2">
            <span class="text-[12px] font-medium text-stone-800">任务</span>
            <span class="text-[10px] leading-[1.2] text-stone-400" data-testid="plan-todo-progress">
              {{ todoSummary }}
            </span>
          </div>

          <div
            v-if="loading && todos.length === 0"
            class="mt-1 flex items-center gap-1.5 text-[10px] leading-5 text-stone-400"
          >
            <LoaderCircle class="h-3 w-3 animate-spin" />
            正在加载…
          </div>
          <div
            v-else-if="todos.length === 0 && !todosLoading"
            class="mt-1 text-[10px] leading-5 text-stone-400"
          >
            暂无任务
          </div>
          <ul v-else class="mt-1 space-y-0.5" data-testid="plan-todos">
            <li
              v-for="(todo, index) in todos"
              :key="index"
              class="flex items-start gap-1.5 text-[11px] leading-4"
            >
              <CheckCircle2
                v-if="todo.status === 'completed'"
                class="mt-0.5 h-3 w-3 shrink-0 text-emerald-600"
              />
              <Circle v-else-if="todo.status === 'pending'" class="mt-0.5 h-3 w-3 shrink-0 text-stone-300" />
              <Check v-else class="mt-0.5 h-3 w-3 shrink-0 text-amber-600" />
              <span
                class="min-w-0 break-words"
                :class="todo.status === 'completed' ? 'text-stone-400 line-through' : 'text-stone-600'"
              >
                {{ todo.content }}
              </span>
            </li>
          </ul>
        </div>
      </div>
    </div>
  </section>
</template>

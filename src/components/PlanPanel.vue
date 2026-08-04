<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { storeToRefs } from "pinia";
import { AlertTriangle, Check, CheckCircle2, ChevronRight, Circle, ListChecks, LoaderCircle } from "lucide-vue-next";
import { usePlanStore } from "@/stores/plan";
import type { Plan, PlanStep } from "@/types/ask-plan";
import Button from "@/components/ui/Button.vue";
import Badge from "@/components/ui/Badge.vue";

const props = defineProps<{
  sessionId: string;
  open: boolean;
}>();

const emit = defineEmits<{ toggle: [] }>();

const planStore = usePlanStore();
const { plans, selectedPlan, loading, error, completingStepKey } = storeToRefs(planStore);

function lifecycleLabel(lifecycle: string) {
  switch (lifecycle) {
    case "draft":
      return "草稿";
    case "executing":
      return "执行中";
    case "completed":
      return "已完成";
    case "aborted":
      return "已中止";
    default:
      return lifecycle;
  }
}

function lifecycleVariant(lifecycle: string): "secondary" | "warning" | "success" | "danger" {
  switch (lifecycle) {
    case "draft":
      return "secondary";
    case "executing":
      return "warning";
    case "completed":
      return "success";
    case "aborted":
      return "danger";
    default:
      return "secondary";
  }
}

function stepStatusLabel(status: string) {
  switch (status) {
    case "pending":
      return "待办";
    case "inProgress":
      return "进行中";
    case "completed":
      return "完成";
    case "failed":
      return "失败";
    default:
      return status;
  }
}

function stepStatusIcon(status: string) {
  if (status === "completed") {
    return CheckCircle2;
  }
  return Circle;
}

function stepBusy(plan: Plan, step: PlanStep) {
  return planStore.isCompletingStep(plan.planId, step.stepId);
}

async function refreshPlans() {
  if (props.sessionId?.trim()) {
    await planStore.list(props.sessionId);
  }
}

async function completeStep(plan: Plan, step: PlanStep) {
  await planStore.completeStep(props.sessionId, plan.planId, plan.revision, step.stepId);
}

onMounted(() => {
  void refreshPlans();
});

watch(
  () => props.sessionId,
  () => {
    void refreshPlans();
  }
);

const completedCount = computed(() =>
  selectedPlan.value
    ? selectedPlan.value.steps.filter((step) => step.status === "completed").length
    : 0
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
        <span>计划</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="text-[10px] leading-[1.2] text-stone-400">{{ plans.length }}</span>
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

        <div v-if="loading && plans.length === 0" class="flex items-center gap-1.5 px-1 text-[10px] leading-5 text-stone-400">
          <LoaderCircle class="h-3 w-3 animate-spin" />
          正在加载计划…
        </div>

        <div v-if="plans.length === 0 && !loading" class="px-1 text-[10px] leading-5 text-stone-400">
          暂无计划
        </div>

        <div
          v-for="plan in plans"
          :key="plan.planId"
          class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
          :data-testid="`plan-card-${plan.planId}`"
        >
          <div class="flex items-center justify-between gap-2">
            <button
              type="button"
              class="min-w-0 flex-1 truncate text-left text-[12px] font-medium text-stone-800"
              data-testid="plan-select"
              @click="planStore.select(plan.planId)"
            >
              {{ plan.summary || plan.kind || plan.planId }}
            </button>
            <Badge :variant="lifecycleVariant(plan.lifecycle)">
              {{ lifecycleLabel(plan.lifecycle) }}
            </Badge>
          </div>

          <div v-if="plan === selectedPlan" class="mt-2 space-y-1" data-testid="plan-steps">
            <div
              v-for="step in plan.steps"
              :key="step.stepId"
              class="flex items-center gap-1.5 rounded-[0.4rem] bg-white/70 px-2 py-1"
              :data-testid="`plan-step-${step.stepId}`"
            >
              <component
                :is="stepStatusIcon(step.status)"
                class="h-3 w-3 shrink-0"
                :class="step.status === 'completed' ? 'text-emerald-600' : 'text-stone-300'"
              />
              <div class="min-w-0 flex-1">
                <div class="truncate text-[11px] leading-4 text-stone-700">{{ step.name }}</div>
                <div v-if="step.summary" class="truncate text-[9px] leading-3 text-stone-400">
                  {{ step.summary }}
                </div>
              </div>
              <span class="shrink-0 text-[9px] uppercase tracking-[0.1em] text-stone-400">
                {{ stepStatusLabel(step.status) }}
              </span>
              <Button
                v-if="step.status !== 'completed' && step.status !== 'failed'"
                size="sm"
                variant="ghost"
                class="h-6 shrink-0 px-2 text-[10px]"
                :disabled="stepBusy(plan, step) || completingStepKey != null"
                data-testid="plan-step-complete"
                @click="completeStep(plan, step)"
              >
                <LoaderCircle v-if="stepBusy(plan, step)" class="mr-1 h-3 w-3 animate-spin" />
                <Check v-else class="mr-1 h-3 w-3" />
                完成
              </Button>
            </div>

            <div v-if="plan.steps.length === 0" class="px-1 text-[10px] leading-5 text-stone-400">
              暂无步骤
            </div>

            <div class="flex items-center justify-between px-1 pt-1 text-[9px] uppercase tracking-[0.12em] text-stone-400">
              <span>revision {{ plan.revision }}</span>
              <span>{{ completedCount }} / {{ plan.steps.length }} 完成</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.collapsible-shell > .collapsible-body {
  display: grid;
  grid-template-rows: 0fr;
  min-height: 0;
  opacity: 0;
  overflow: hidden;
  transition:
    grid-template-rows 260ms cubic-bezier(0.2, 0.72, 0.18, 1),
    opacity 180ms ease;
}

.collapsible-shell[data-open="true"] > .collapsible-body {
  grid-template-rows: 1fr;
  min-height: 0;
  opacity: 1;
}

.collapsible-content {
  min-height: 0;
  overflow: hidden;
}
</style>

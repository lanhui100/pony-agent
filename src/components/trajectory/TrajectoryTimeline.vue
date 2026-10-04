<script setup lang="ts">
import { computed } from "vue";
import type { TrajectoryTimelineModel, TrajectoryTimelineSpan } from "@/lib/runtime/trajectory-core";
import Tooltip from "@/components/ui/Tooltip.vue";

const props = defineProps<{
  model: TrajectoryTimelineModel;
  selectedSpanId?: string | null;
}>();

const emit = defineEmits<{
  "select-span": [span: TrajectoryTimelineSpan];
}>();

const laneLabels = ["Input / Context", "Model / Message", "Tool / Action"];

const totalDuration = computed(() => {
  return Math.max(1, props.model.totalEnd - props.model.totalStart);
});

function getSpanLeftPercent(span: TrajectoryTimelineSpan): number {
  if (totalDuration.value <= 0) return 0;
  return ((span.start - props.model.totalStart) / totalDuration.value) * 100;
}

function getSpanWidthPercent(span: TrajectoryTimelineSpan): number {
  if (totalDuration.value <= 0) return 0;
  const rawWidth = ((span.end - span.start) / totalDuration.value) * 100;
  return Math.max(0.6, rawWidth);
}

function getTurnBoundaryLeftPercent(time: number): number {
  if (totalDuration.value <= 0) return 0;
  return ((time - props.model.totalStart) / totalDuration.value) * 100;
}

function getSpanColorClasses(span: TrajectoryTimelineSpan): string {
  if (span.isError) {
    return "bg-rose-500 hover:bg-rose-600 text-white border-rose-600";
  }
  switch (span.kind) {
    case "user":
      return "bg-sky-500/80 hover:bg-sky-500 text-white border-sky-600";
    case "context":
    case "system":
      return "bg-stone-400 hover:bg-stone-500 text-stone-900 border-stone-400";
    case "message":
    case "compacted":
      return "bg-emerald-500/80 hover:bg-emerald-500 text-white border-emerald-600";
    case "tool":
    case "subtool":
      return "bg-amber-500/80 hover:bg-amber-500 text-white border-amber-600";
    default:
      return "bg-stone-400 hover:bg-stone-500 text-white border-stone-500";
  }
}
</script>

<template>
  <div
    class="relative select-none border-b border-stone-200/80 bg-stone-50/70 p-2 text-xs dark:border-stone-800 dark:bg-stone-950/40"
    role="region"
    aria-label="轨迹时间轴"
    data-testid="trajectory-timeline"
  >
    <!-- Turn Boundaries Vertical Rules -->
    <div class="pointer-events-none absolute inset-0 z-0 overflow-hidden">
      <div
        v-for="boundary in model.turnBoundaries"
        :key="boundary.turnId"
        class="absolute top-0 bottom-0 border-l border-dashed border-stone-300 dark:border-stone-700"
        :style="{ left: `${getTurnBoundaryLeftPercent(boundary.time)}%` }"
      >
        <span
          class="sticky top-0 ml-1 inline-block rounded bg-stone-200/80 px-1 py-0.5 text-[9px] font-semibold text-stone-600 backdrop-blur-xs dark:bg-stone-800 dark:text-stone-300"
        >
          {{ boundary.title }}
        </span>
      </div>
    </div>

    <!-- 3 Swimlanes -->
    <div class="relative z-10 space-y-1.5 pt-4">
      <div
        v-for="(label, laneIndex) in laneLabels"
        :key="laneIndex"
        class="relative flex h-6 items-center rounded bg-stone-200/40 px-1 dark:bg-stone-900/60"
      >
        <!-- Lane Label -->
        <span
          class="pointer-events-none absolute left-1.5 z-20 text-[10px] font-medium tracking-tight text-stone-400 select-none opacity-60"
        >
          {{ label }}
        </span>

        <!-- Spans in this lane -->
        <div class="relative h-full w-full">
          <template v-for="span in model.spans" :key="span.id">
            <Tooltip
              v-if="span.lane === laneIndex"
              :text="`${span.label} (${span.durationMs > 0 ? span.durationMs + 'ms' : 'seq'})`"
            >
              <button
                type="button"
                class="absolute top-1 bottom-1 flex items-center justify-center truncate rounded-[3px] border px-1 text-[10px] font-medium leading-none transition shadow-2xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-400"
                :class="[
                  getSpanColorClasses(span),
                  selectedSpanId === span.id
                    ? 'ring-2 ring-amber-500 ring-offset-1 z-30 font-bold scale-[1.02]'
                    : 'z-10'
                ]"
                :style="{
                  left: `${getSpanLeftPercent(span)}%`,
                  width: `${getSpanWidthPercent(span)}%`
                }"
                :aria-label="span.label"
                :data-testid="`trajectory-span-${span.id}`"
                @click="emit('select-span', span)"
              >
                <span class="truncate">{{ span.label }}</span>
              </button>
            </Tooltip>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

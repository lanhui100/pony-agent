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
      return "bg-[#8b5e34] hover:bg-[#724c29] text-[#faf6ef] border-[#724c29]";
    case "context":
    case "system":
      return "bg-stone-300 hover:bg-stone-400 text-stone-800 border-stone-400";
    case "message":
    case "compacted":
      return "bg-[#c89d66] hover:bg-[#b88c55] text-stone-900 border-[#b88c55]";
    case "tool":
    case "subtool":
      return "bg-[#e2b882] hover:bg-[#d4a469] text-stone-900 border-[#d4a469]";
    default:
      return "bg-stone-300 hover:bg-stone-400 text-stone-800 border-stone-400";
  }
}

function getSpanCoreInitial(span: TrajectoryTimelineSpan): string {
  // 根据事件类型或标签提取核心名称首字母：如 Model -> M, Tool -> T
  switch (span.kind) {
    case "message":
    case "compacted":
      return "M";
    case "tool":
    case "subtool":
      return "T";
    case "user":
      return "U";
    case "context":
      return "C";
    case "system":
      return "S";
    default: {
      const trimmed = span.label?.trim() || "";
      return trimmed ? trimmed.charAt(0).toUpperCase() : "?";
    }
  }
}
</script>

<template>
  <div
    class="relative select-none border-b border-stone-200/70 bg-[#faf6ef] p-2 text-xs"
    role="region"
    aria-label="轨迹时间轴"
    data-testid="trajectory-timeline"
  >
    <!-- Turn Boundaries Vertical Rules -->
    <div class="pointer-events-none absolute inset-0 z-0 overflow-hidden">
      <div
        v-for="boundary in model.turnBoundaries"
        :key="boundary.turnId"
        class="absolute top-0 bottom-0 border-l border-dashed border-stone-300/80"
        :style="{ left: `${getTurnBoundaryLeftPercent(boundary.time)}%` }"
      >
        <span
          class="sticky top-0 ml-1 inline-block rounded bg-[#f6f0e8] px-1 py-0.5 text-[9px] font-semibold text-stone-600 shadow-xs"
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
        class="relative flex h-6 items-center rounded-[0.35rem] bg-[#f6f0e8] px-1"
      >
        <!-- Lane Label -->
        <span
          class="pointer-events-none absolute left-1.5 z-20 text-[10px] font-medium tracking-tight text-stone-400 select-none opacity-70"
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
                class="absolute top-1 bottom-1 flex items-center justify-center truncate rounded-[3px] border px-1 text-[10px] font-medium leading-none transition shadow-2xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-400 cursor-pointer"
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
                <span>{{ getSpanCoreInitial(span) }}</span>
              </button>
            </Tooltip>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { ChevronRight, ListChecks } from "lucide-vue-next";
import { useTeamStore } from "@/stores/team";
import {
  shouldUseTeamPreview,
  sortTeamTasks,
  TEAM_PREVIEW_TASKS
} from "@/components/team/team-preview-fixtures";
import type { TeamTaskItem } from "@/types/team";

defineProps<{ open: boolean }>();
const emit = defineEmits<{ toggle: [] }>();

const teamStore = useTeamStore();

// 浏览器 dev 预览兜底：store 为空且无 Tauri 运行时时，用同一套排序规则排预览数据，
// 保证排序切换开关在预览态下同样真实生效。
const visibleTasks = computed(() =>
  teamStore.tasks.length > 0
    ? teamStore.sortedTasks
    : shouldUseTeamPreview()
      ? sortTeamTasks(TEAM_PREVIEW_TASKS, teamStore.sortMode)
      : []
);

const completedCount = computed(
  () => visibleTasks.value.filter((task) => task.status === "completed").length
);

function taskDotClass(status: TeamTaskItem["status"]) {
  switch (status) {
    case "completed":
      return "bg-emerald-500";
    case "in_progress":
      return "bg-amber-500";
    default:
      return "bg-stone-300";
  }
}

function taskStatusLabel(status: TeamTaskItem["status"]) {
  switch (status) {
    case "completed":
      return "已完成";
    case "in_progress":
      return "进行中";
    default:
      return "待开始";
  }
}
</script>

<template>
  <section class="collapsible-shell border-b border-stone-200/60 pb-4" :data-open="open">
    <button
      class="flex w-full items-center justify-between gap-3 text-left"
      type="button"
      data-testid="team-task-board-toggle"
      @click="emit('toggle')"
    >
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <ListChecks class="h-3.5 w-3.5" />
        <span>任务看板</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="text-[10px] leading-[1.2] text-stone-400">
          {{ completedCount }}/{{ visibleTasks.length }}
        </span>
        <ChevronRight
          class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200"
          :class="{ 'rotate-90': open }"
        />
      </div>
    </button>

    <div class="collapsible-body">
      <div class="collapsible-content mt-2 space-y-2">
        <div
          class="flex items-center gap-1 text-[9px] uppercase tracking-[0.1em]"
          data-testid="team-sort-switch"
        >
          <button
            type="button"
            class="transition-colors duration-200"
            :class="teamStore.sortMode === 'created' ? 'text-stone-700' : 'text-stone-400 hover:text-stone-600'"
            :aria-pressed="teamStore.sortMode === 'created'"
            data-testid="team-sort-created"
            @click="teamStore.setSortMode('created')"
          >
            生成次序
          </button>
          <span class="text-stone-300">/</span>
          <button
            type="button"
            class="transition-colors duration-200"
            :class="teamStore.sortMode === 'progress' ? 'text-stone-700' : 'text-stone-400 hover:text-stone-600'"
            :aria-pressed="teamStore.sortMode === 'progress'"
            data-testid="team-sort-progress"
            @click="teamStore.setSortMode('progress')"
          >
            按进展
          </button>
        </div>

        <div
          v-if="visibleTasks.length === 0"
          class="px-1 text-[10px] leading-5 text-stone-400"
          data-testid="team-task-empty"
        >
          暂无任务
        </div>

        <div v-else class="flex flex-col gap-1.5" data-testid="team-task-list">
          <div
            v-for="task in visibleTasks"
            :key="task.task_id"
            class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
            data-testid="team-task-item"
            :data-task-id="task.task_id"
            :data-status="task.status"
          >
            <div class="flex items-center gap-1.5">
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full transition-colors duration-200"
                :class="taskDotClass(task.status)"
                role="img"
                :aria-label="taskStatusLabel(task.status)"
              />
              <span
                class="min-w-0 flex-1 truncate text-[12px] leading-4"
                :class="task.status === 'completed' ? 'text-stone-400 line-through' : 'text-stone-700'"
              >
                {{ task.subject }}
              </span>
            </div>

            <div
              v-if="task.description"
              class="mt-0.5 truncate text-[9px] leading-3 text-stone-400"
              data-testid="team-task-description"
            >
              {{ task.description }}
            </div>

            <div
              class="mt-1 flex items-center justify-between text-[9px] uppercase tracking-[0.12em] text-stone-400"
            >
              <span>负责 {{ task.owner || "未分配" }}</span>
              <span>{{ taskStatusLabel(task.status) }}</span>
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

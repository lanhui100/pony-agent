<template>
  <div class="team-panel mx-auto w-full max-w-[46.4rem] px-4 pt-2 pb-1 transition-all">
    <div class="rounded-xl border border-stone-200/80 bg-stone-50/60 p-3 shadow-xs backdrop-blur-xs">
      <div class="flex items-center justify-between cursor-pointer select-none" @click="expanded = !expanded">
        <div class="flex items-center gap-2.5">
          <div class="flex h-6 w-6 items-center justify-center rounded-lg bg-stone-200/60 text-stone-700">
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"></path>
              <circle cx="9" cy="7" r="4"></circle>
              <path d="M23 21v-2a4 4 0 0 0-3-3.87"></path>
              <path d="M16 3.13a4 4 0 0 1 0 7.75"></path>
            </svg>
          </div>
          <span class="text-xs font-semibold text-stone-800">Agent Team 协同大盘</span>
          <span class="inline-flex items-center rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700 border border-emerald-200/60">
            <span class="mr-1.5 h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse"></span>
            {{ displayMembers.length }} 名队员在册
          </span>
          <span v-if="displayTasks.length > 0" class="text-[11px] text-stone-500">
            · {{ completedTasksCount }}/{{ displayTasks.length }} 任务完成
          </span>
        </div>
        <div class="flex items-center gap-2 text-stone-400 hover:text-stone-600 text-xs">
          <span>{{ expanded ? "收起" : "展开" }}</span>
          <svg
            class="h-3.5 w-3.5 transform transition-transform duration-200"
            :class="{ 'rotate-180': expanded }"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <polyline points="6 9 12 15 18 9"></polyline>
          </svg>
        </div>
      </div>

      <!-- 展开详情面板 -->
      <div v-show="expanded" class="mt-3.5 flex flex-col gap-3 pt-3 border-t border-stone-200/60">
        <!-- 成员花名册 (Roster) -->
        <div>
          <div class="text-[11px] font-medium text-stone-500 mb-2">活跃花名册 (Roster)</div>
          <div class="grid grid-cols-2 sm:grid-cols-3 gap-2">
            <div
              v-for="member in displayMembers"
              :key="member.target"
              class="flex items-center justify-between rounded-lg border border-stone-200/70 bg-white px-2.5 py-1.5 shadow-2xs"
            >
              <div class="flex items-center gap-2 min-w-0">
                <span
                  class="h-2 w-2 shrink-0 rounded-full"
                  :class="{
                    'bg-emerald-500 ring-2 ring-emerald-200': member.status === 'running',
                    'bg-sky-500': member.status === 'active',
                    'bg-stone-300': member.status === 'inactive',
                    'bg-amber-400': member.status === 'provisioning',
                  }"
                />
                <span class="truncate text-xs font-medium text-stone-800">{{ member.name }}</span>
              </div>
              <span class="shrink-0 text-[10px] text-stone-400 uppercase tracking-wider font-mono">{{ member.role }}</span>
            </div>
          </div>
        </div>

        <!-- 任务看板 (Task Board) -->
        <div>
          <div class="text-[11px] font-medium text-stone-500 mb-2">任务依赖与写域看板 (Task Board)</div>
          <div class="grid grid-cols-1 sm:grid-cols-2 gap-2">
            <div
              v-for="task in displayTasks"
              :key="task.task_id"
              class="flex flex-col justify-between rounded-lg border border-stone-200/70 bg-white p-2.5 shadow-2xs gap-1.5"
            >
              <div class="flex items-start justify-between gap-2">
                <span class="text-xs font-medium text-stone-800 line-clamp-1">{{ task.subject }}</span>
                <span
                  class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium font-mono"
                  :class="{
                    'bg-emerald-50 text-emerald-700 border border-emerald-200/60': task.status === 'completed',
                    'bg-sky-50 text-sky-700 border border-sky-200/60': task.status === 'in_progress',
                    'bg-amber-50 text-amber-700 border border-amber-200/60': task.status === 'pending',
                  }"
                >
                  {{ task.status }}
                </span>
              </div>
              <p class="text-[11px] text-stone-500 line-clamp-2 leading-relaxed">{{ task.description }}</p>
              <div class="flex items-center justify-between text-[10px] text-stone-400 pt-1 border-t border-stone-100 font-mono">
                <span>负责: {{ task.owner || "未分配" }}</span>
                <span>Rev {{ task.revision }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useTeamStore } from "@/stores/team";
import type { TeammateMember, TeamTaskItem } from "@/types/team";

const teamStore = useTeamStore();
const expanded = ref(true);

// 默认兜底预览数据（在纯浏览器非 Tauri 开发环境下展示完整高保真布局）
const fallbackMembers: TeammateMember[] = [
  { target: "lead", name: "Team Lead", role: "lead", status: "running", description: "Orchestrator" },
  { target: "executor-1", name: "Executor", role: "backend", status: "active", description: "Implementation" },
  { target: "test-agent", name: "Test Agent", role: "qa", status: "active", description: "Adversarial & Acceptance" },
];

const fallbackTasks: TeamTaskItem[] = [
  {
    task_id: "task-01",
    subject: "全双工队友消息总线与异步事件驱动",
    description: "驱动 HostTurnEngine 在后台工作线程执行真实 LLM turn 并回复",
    status: "completed",
    owner: "executor-1",
    revision: 3,
    write_scopes: ["crates/pony-agent-core/src/agent"],
    blocked_by: [],
  },
  {
    task_id: "task-02",
    subject: "前端协同大盘高保真展示与交互",
    description: "完成 Roster 花名册与 Task Board 看板响应式渲染及状态流转",
    status: "in_progress",
    owner: "lead",
    revision: 2,
    write_scopes: ["src/components"],
    blocked_by: ["task-01"],
  },
];

const displayMembers = computed(() => {
  return teamStore.members.length > 0 ? teamStore.members : fallbackMembers;
});

const displayTasks = computed(() => {
  return teamStore.tasks.length > 0 ? teamStore.tasks : fallbackTasks;
});

const completedTasksCount = computed(() => {
  return displayTasks.value.filter((t) => t.status === "completed").length;
});

onMounted(() => {
  teamStore.fetchTeamOverview();
});
</script>

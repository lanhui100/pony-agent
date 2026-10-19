<script setup lang="ts">
import { computed, onMounted } from "vue";
import { ChevronRight, Users } from "lucide-vue-next";
import { useTeamStore } from "@/stores/team";
import { shouldUseTeamPreview, TEAM_PREVIEW_MEMBERS } from "@/components/team/team-preview-fixtures";
import type { TeammateStatus } from "@/types/team";

defineProps<{ open: boolean }>();
const emit = defineEmits<{ toggle: [] }>();

const teamStore = useTeamStore();

// 浏览器 dev 预览兜底：仅当 store 无数据且无 Tauri 运行时时展示预览夹具。
const members = computed(() =>
  teamStore.members.length > 0
    ? teamStore.members
    : shouldUseTeamPreview()
      ? TEAM_PREVIEW_MEMBERS
      : []
);

function memberDotClass(status: TeammateStatus) {
  switch (status) {
    case "running":
      return "bg-emerald-500";
    case "active":
      return "bg-emerald-400";
    case "provisioning":
      return "bg-amber-500";
    case "failed":
      return "bg-rose-400";
    default:
      return "bg-stone-300";
  }
}

onMounted(() => {
  void teamStore.fetchTeamOverview();
});
</script>

<template>
  <section class="collapsible-shell border-b border-stone-200/60 pb-4" :data-open="open">
    <button
      class="flex w-full items-center justify-between gap-3 text-left"
      type="button"
      data-testid="team-roster-toggle"
      @click="emit('toggle')"
    >
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <Users class="h-3.5 w-3.5" />
        <span>队友</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="text-[10px] leading-[1.2] text-stone-400">{{ members.length }}</span>
        <ChevronRight
          class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200"
          :class="{ 'rotate-90': open }"
        />
      </div>
    </button>

    <div class="collapsible-body">
      <div class="collapsible-content mt-2 space-y-2">
        <div
          v-if="members.length === 0"
          class="px-1 text-[10px] leading-5 text-stone-400"
          data-testid="team-roster-empty"
        >
          暂无队友
        </div>

        <div
          v-else
          class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
          data-testid="team-roster-list"
        >
          <ul class="space-y-1">
            <li
              v-for="member in members"
              :key="member.target"
              class="flex items-center gap-1.5 px-1 py-0.5"
              data-testid="team-roster-item"
              :data-status="member.status"
            >
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full transition-colors duration-200"
                :class="memberDotClass(member.status)"
                role="img"
                :aria-label="member.status"
              />
              <span class="min-w-0 flex-1 truncate text-[11px] leading-4 text-stone-700">
                {{ member.name }}
              </span>
              <span class="shrink-0 text-[9px] uppercase tracking-[0.1em] text-stone-400">
                {{ member.role }}
              </span>
            </li>
          </ul>
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

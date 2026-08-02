<script setup lang="ts">
import { ChevronRight, Wrench } from "lucide-vue-next";
import type { AvailableTool } from "@/types/runtime";

defineProps<{
  tools: AvailableTool[];
  open: boolean;
}>();

const emit = defineEmits<{ toggle: [] }>();

function availableToolDisplayLabel(tool: AvailableTool) {
  return tool.displayMetadata.displayNameZh?.trim()
    || tool.canonicalToolName?.trim()
    || tool.name;
}

function availableToolApprovalLabel(tool: AvailableTool) {
  const mode = tool.permissionFacts.approvalMode?.trim();
  if (mode) {
    return mode;
  }

  return tool.permissionFacts.requiresApproval ? "required" : "none";
}

function availableToolPermissionScopeLabel(tool: AvailableTool) {
  return tool.permissionFacts.permissionScope?.trim() || "--";
}

function availableToolPermissionSourceLabel(tool: AvailableTool) {
  return tool.permissionFacts.decisionSource?.trim() || "--";
}
</script>

<template>
  <section class="collapsible-shell mt-auto border-b border-stone-200/60 pb-4" :data-open="open">
    <button class="flex w-full items-center justify-between gap-3 text-left" type="button" data-testid="tools-panel-toggle" @click="emit('toggle')">
      <div class="flex items-center gap-2 text-[11px] uppercase tracking-[0.18em] text-stone-500">
        <Wrench class="h-3.5 w-3.5" />
        <span>Tools</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="text-[10px] leading-[1.2] text-stone-400">{{ tools.length }}</span>
        <ChevronRight class="h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200" :class="{ 'rotate-90': open }" />
      </div>
    </button>

    <div class="collapsible-body">
      <section class="collapsible-content mt-2 space-y-2">
        <div
          v-for="tool in tools"
          :key="tool.name"
          class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="min-w-0 text-[12px] font-medium text-stone-800">
              {{ availableToolDisplayLabel(tool) }}
            </div>
            <span class="shrink-0 text-[10px] uppercase tracking-[0.14em] text-stone-400">
              {{ tool.kind }}
            </span>
          </div>
          <p v-if="tool.description" class="mt-1 text-[10px] leading-[1.3] text-stone-500">
            {{ tool.description }}
          </p>
          <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[10px] leading-[1.25] text-stone-500">
            <span>权限: {{ availableToolPermissionScopeLabel(tool) }}</span>
            <span>审批: {{ availableToolApprovalLabel(tool) }}</span>
            <span>来源: {{ availableToolPermissionSourceLabel(tool) }}</span>
          </div>
        </div>
        <div v-if="tools.length === 0" class="px-1 text-[10px] leading-5 text-stone-400">
          暂无可用工具
        </div>
      </section>
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

<script setup lang="ts">
import { computed } from "vue";
import { storeToRefs } from "pinia";
import { Wrench } from "lucide-vue-next";
import ScrollArea from "@/components/ui/ScrollArea.vue";
import { useRuntimeStore } from "@/stores/runtime";
import type { AvailableTool } from "@/types/runtime";

/**
 * PA-096：配置页"工具" tab——可用工具目录静态列表。
 * 文案规则自原 HomeToolsPanel 移植（中文名优先、审批缺失回退 requiresApproval）。
 */

const runtimeStore = useRuntimeStore();
const { availableTools } = storeToRefs(runtimeStore);

const toolCount = computed(() => availableTools.value.length);

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
  <div class="flex h-full min-h-0 flex-col" data-testid="config-tools-section">
    <div class="flex items-center justify-between gap-3 px-1 pb-2">
      <div class="flex items-center gap-2 text-[12px] font-medium text-stone-500">
        <Wrench class="h-3.5 w-3.5" />
        <span>当前运行时可用的工具目录（只读）</span>
      </div>
      <span class="text-[11px] text-stone-400">{{ toolCount }} 项</span>
    </div>

    <ScrollArea class="min-h-0 flex-1" viewport-class="pr-1">
      <div class="grid gap-2 pb-1 sm:grid-cols-2 xl:grid-cols-3" data-testid="config-tools-list">
        <section
          v-for="tool in availableTools"
          :key="tool.name"
          class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2.5"
          :data-testid="`config-tool-${tool.name}`"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="min-w-0 text-[12px] font-medium text-stone-800">
              {{ availableToolDisplayLabel(tool) }}
            </div>
            <span class="shrink-0 text-[10px] uppercase tracking-[0.14em] text-stone-400">
              {{ tool.kind }}
            </span>
          </div>
          <p v-if="tool.description" class="mt-1 text-[10px] leading-[1.4] text-stone-500">
            {{ tool.description }}
          </p>
          <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[10px] leading-[1.35] text-stone-500">
            <span>权限: {{ availableToolPermissionScopeLabel(tool) }}</span>
            <span>审批: {{ availableToolApprovalLabel(tool) }}</span>
            <span>来源: {{ availableToolPermissionSourceLabel(tool) }}</span>
          </div>
        </section>

        <p
          v-if="availableTools.length === 0"
          class="col-span-full px-1 py-6 text-center text-[12px] leading-5 text-stone-400"
          data-testid="config-tools-empty"
        >
          暂无可用工具
        </p>
      </div>
    </ScrollArea>
  </div>
</template>

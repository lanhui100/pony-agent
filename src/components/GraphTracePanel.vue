<script setup lang="ts">
import { storeToRefs } from "pinia";
import { GitBranch, Wrench } from "lucide-vue-next";
import { useRuntimeStore } from "@/stores/runtime";
import Badge from "@/components/ui/Badge.vue";
import Card from "@/components/ui/Card.vue";
import Separator from "@/components/ui/Separator.vue";

const runtimeStore = useRuntimeStore();
const { toolActivities, traceSteps } = storeToRefs(runtimeStore);
</script>

<template>
  <Card class="p-4">
    <div class="flex items-start justify-between gap-3">
      <div class="space-y-1">
        <div class="text-sm font-semibold text-slate-900">3. 执行轨迹与工具状态</div>
        <p class="text-sm leading-6 text-slate-600">
          这里显示的不是“理论流程图”，而是这一轮执行返回给前端的结构化轨迹数据。
        </p>
      </div>
      <GitBranch class="mt-1 h-4 w-4 shrink-0 text-slate-400" />
    </div>

    <div class="mt-4 space-y-2">
      <div
        v-for="step in traceSteps"
        :key="step.id"
        class="flex items-center justify-between rounded-xl border px-3 py-2 text-sm"
        :class="{
          'border-emerald-200 bg-emerald-50/80 text-emerald-900': step.state === 'completed',
          'border-sky-200 bg-sky-50/80 text-sky-900': step.state === 'active',
          'border-slate-200 bg-slate-50 text-slate-600': step.state === 'pending'
        }"
      >
        <span class="font-medium">{{ step.label }}</span>
        <Badge :variant="step.state === 'completed' ? 'success' : step.state === 'active' ? 'default' : 'secondary'">
          {{ step.state === "completed" ? "已完成" : step.state === "active" ? "进行中" : "未进入" }}
        </Badge>
      </div>
    </div>

    <Separator class="my-4" />

    <div class="mb-2 flex items-center gap-2 text-sm font-semibold text-slate-900">
      <Wrench class="h-4 w-4 text-slate-400" />
      工具层预留状态
    </div>

    <div class="space-y-2">
      <div v-for="tool in toolActivities" :key="tool.id" class="rounded-xl border border-slate-200 bg-slate-50/70 px-3 py-3 text-sm">
        <div class="flex items-center justify-between gap-3">
          <strong>{{ tool.name }}</strong>
          <Badge variant="secondary">{{ tool.status }}</Badge>
        </div>
        <p class="mt-2 leading-6 text-slate-600">{{ tool.summary }}</p>
      </div>
    </div>
  </Card>
</template>

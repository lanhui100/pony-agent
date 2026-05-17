<script setup lang="ts">
import { storeToRefs } from "pinia";
import { onMounted } from "vue";
import { Activity, Cpu, TriangleAlert, Workflow } from "lucide-vue-next";
import { useRuntimeStore } from "@/stores/runtime";
import Badge from "@/components/ui/Badge.vue";
import Card from "@/components/ui/Card.vue";

const runtimeStore = useRuntimeStore();
const {
  error,
  fallbackReason,
  health,
  phaseLabel,
  providerMode,
  providerModel,
  providerName,
  providerProtocol,
  providerRequestedName,
  sessionSummary
} = storeToRefs(runtimeStore);

onMounted(() => {
  void runtimeStore.fetchHealth();
});
</script>

<template>
  <Card class="p-4">
    <div class="flex items-start justify-between gap-3">
      <div class="space-y-1">
        <div class="text-sm font-semibold text-slate-900">2. 当前运行状态</div>
        <p class="text-sm leading-6 text-slate-600">
          这里专门用来回答一个关键问题：这一轮到底有没有走到真实模型，如果没有，为什么会回退到 mock。
        </p>
      </div>
      <Cpu class="mt-1 h-4 w-4 shrink-0 text-slate-400" />
    </div>

    <div
      v-if="health"
      class="mt-4 rounded-xl border border-emerald-100 bg-emerald-50/80 p-4 text-sm leading-7 text-emerald-950"
    >
      <div><strong>{{ health.appName }}</strong> {{ health.appVersion }}</div>
      <div>运行时内核：{{ health.runtime }}</div>
      <div>图编排引擎：{{ health.graphEngine }}</div>
      <div>当前阶段：{{ phaseLabel }}</div>
    </div>
    <div
      v-else-if="error"
      class="mt-4 rounded-xl border border-rose-100 bg-rose-50/80 p-4 text-sm leading-7 text-rose-900"
    >
      {{ error }}
    </div>
    <div v-else class="mt-4 rounded-xl border border-slate-200 bg-slate-50 p-4 text-sm leading-7 text-slate-600">
      正在连接 Rust 后端...
    </div>

    <div class="mt-4 space-y-2 rounded-xl bg-slate-50/80 px-3 py-3 text-[13px] leading-6 text-slate-700">
      <div><span class="font-medium text-slate-900">目标提供商：</span>{{ providerRequestedName || "尚未执行本轮" }}</div>
      <div><span class="font-medium text-slate-900">当前提供商：</span>{{ providerName || "尚未执行本轮" }}</div>
      <div><span class="font-medium text-slate-900">运行模式：</span>{{ providerMode || "尚未执行本轮" }}</div>
      <div><span class="font-medium text-slate-900">协议：</span>{{ providerProtocol || "尚未执行本轮" }}</div>
      <div><span class="font-medium text-slate-900">模型：</span>{{ providerModel || "尚未执行本轮" }}</div>
    </div>

    <div
      v-if="fallbackReason"
      class="mt-3 rounded-xl border border-amber-100 bg-amber-50/90 px-3 py-3 text-[13px] leading-6 text-amber-900"
    >
      <div class="mb-1 flex items-center gap-2 font-medium">
        <TriangleAlert class="h-4 w-4" />
        回退原因
      </div>
      <p>{{ fallbackReason }}</p>
    </div>

    <div
      v-if="sessionSummary"
      class="mt-3 rounded-xl border border-slate-200 bg-slate-50/80 px-3 py-2 text-[13px] leading-6 text-slate-600"
    >
      会话摘要：{{ sessionSummary }}
    </div>

    <div class="mt-4 flex flex-wrap gap-2">
      <Badge><Activity class="mr-1 h-3.5 w-3.5" />Session</Badge>
      <Badge><Cpu class="mr-1 h-3.5 w-3.5" />Provider</Badge>
      <Badge><Workflow class="mr-1 h-3.5 w-3.5" />Trace</Badge>
    </div>
  </Card>
</template>

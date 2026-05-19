<script setup lang="ts">
import { storeToRefs } from "pinia";
import { Activity, BookOpen, GitBranch, TriangleAlert } from "lucide-vue-next";
import CollapsiblePanel from "@/components/CollapsiblePanel.vue";
import Badge from "@/components/ui/Badge.vue";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const {
  error,
  fallbackReason,
  firstTokenLatencyMs,
  health,
  inputTokens,
  outputTokens,
  phaseLabel,
  providerMode,
  providerRequestedName,
  sessionSummary,
  toolActivities,
  totalTokens,
  traceSteps
} = storeToRefs(runtimeStore);

const { currentProvider } = storeToRefs(providerStore);

function formatMetric(value: number | null | undefined, suffix = "") {
  if (value === null || value === undefined) {
    return "待生成";
  }

  return `${value}${suffix}`;
}
</script>

<template>
  <aside class="flex min-h-[calc(100vh-12rem)] min-w-0 flex-col gap-3 overflow-x-hidden">
    <CollapsiblePanel
      title="运行状态"
      description="这里集中看当前回合是否命中了真实 provider，以及这轮运行拿到了哪些指标。"
      tip="现在状态区不只回答是否 fallback，也开始承接 token 统计和首 token 延迟。"
      tone="sage"
    >
      <template #icon>
        <Activity class="h-4 w-4 text-amber-700" />
      </template>

      <div class="space-y-3 text-sm leading-6 text-stone-700">
        <div class="px-1 py-1">
          <div class="font-medium text-stone-900">{{ health?.appName ?? "Pony Agent" }} {{ health?.appVersion ?? "" }}</div>
          <div>阶段：{{ phaseLabel }}</div>
          <div>运行时：{{ health?.runtime ?? "等待连接" }}</div>
          <div>图引擎：{{ health?.graphEngine ?? "等待连接" }}</div>
        </div>

        <div class="grid gap-2 px-1 py-1 text-[13px]">
          <div><span class="font-medium text-stone-900">目标 provider：</span>{{ providerRequestedName || currentProvider?.name || "尚未选择" }}</div>
          <div><span class="font-medium text-stone-900">运行模式：</span>{{ providerMode || "待执行" }}</div>
          <div><span class="font-medium text-stone-900">输入 token：</span>{{ formatMetric(inputTokens) }}</div>
          <div><span class="font-medium text-stone-900">输出 token：</span>{{ formatMetric(outputTokens) }}</div>
          <div><span class="font-medium text-stone-900">总 token：</span>{{ formatMetric(totalTokens) }}</div>
          <div><span class="font-medium text-stone-900">首 token 延迟：</span>{{ formatMetric(firstTokenLatencyMs, " ms") }}</div>
        </div>

        <div v-if="fallbackReason" class="rounded-[0.45rem] bg-amber-100/70 px-3 py-3 text-[13px] text-amber-950">
          <div class="mb-1 flex items-center gap-2 font-medium">
            <TriangleAlert class="h-4 w-4" />
            fallback 原因
          </div>
          <p>{{ fallbackReason }}</p>
        </div>

        <div v-else-if="error" class="rounded-[0.45rem] bg-rose-50 px-3 py-3 text-[13px] text-rose-900">
          {{ error }}
        </div>

        <div v-if="sessionSummary" class="rounded-[0.45rem] bg-stone-900 px-3 py-3 text-[13px] text-amber-50">
          {{ sessionSummary }}
        </div>
      </div>
    </CollapsiblePanel>

    <CollapsiblePanel
      title="Trace"
      description="这里显示当前回合回传给前端的结构化执行轨迹。"
      tip="后续再接更多 runtime 细节时，仍然沿这条 trace 和 tool 活动流扩展。"
      tone="sky"
    >
      <template #icon>
        <GitBranch class="h-4 w-4 text-amber-700" />
      </template>

      <div class="space-y-2">
        <div
          v-for="step in traceSteps"
          :key="step.id"
          class="flex items-center justify-between rounded-[0.45rem] bg-white px-3 py-2.5 text-sm"
        >
          <span class="font-medium text-stone-900">{{ step.label }}</span>
          <Badge :variant="step.state === 'completed' ? 'success' : step.state === 'active' ? 'default' : step.state === 'error' ? 'danger' : 'secondary'">
            {{ step.state === "completed" ? "已完成" : step.state === "active" ? "进行中" : step.state === "error" ? "失败" : "未进入" }}
          </Badge>
        </div>

        <div
          v-for="tool in toolActivities"
          :key="tool.id"
          class="rounded-[0.45rem] bg-white px-3 py-3 text-sm leading-6 text-stone-700"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="font-medium text-stone-900">{{ tool.name }}</div>
            <Badge :variant="tool.status === 'done' ? 'success' : tool.status === 'running' ? 'default' : tool.status === 'error' ? 'danger' : 'secondary'">
              {{ tool.status === "planned" ? "未触发" : tool.status === "running" ? "执行中" : tool.status === "done" ? "已完成" : "失败" }}
            </Badge>
          </div>
          <p class="mt-1 text-[13px] text-stone-500">{{ tool.summary }}</p>
        </div>
      </div>
    </CollapsiblePanel>

    <CollapsiblePanel
      title="学习指导"
      description="把这块侧边当作 workbench 观察窗，而不只是结果说明。"
      tip="一个回合建议先看状态，再看 trace，最后回到对话区交叉验证文本输出。"
      tone="slate"
      :default-open="false"
    >
      <template #icon>
        <BookOpen class="h-4 w-4 text-amber-800" />
      </template>

      <div class="grid gap-2 text-sm leading-6 text-stone-600">
        <div class="rounded-[0.45rem] bg-white px-3 py-3">
          <div class="font-medium text-stone-900">先看状态</div>
          <p class="mt-1 text-[13px]">确认这轮是否真的命中了 live provider，还是因为凭证缺失或请求失败回退到了 mock。</p>
        </div>
        <div class="rounded-[0.45rem] bg-white px-3 py-3">
          <div class="font-medium text-stone-900">再看指标</div>
          <p class="mt-1 text-[13px]">现在可以直接观察 token 使用量和首 token 延迟，帮助理解 runtime 的真实回包节奏。</p>
        </div>
        <div class="rounded-[0.45rem] bg-white px-3 py-3">
          <div class="font-medium text-stone-900">最后看配置</div>
          <p class="mt-1 text-[13px]">前端 workbench 只是壳层，真正可复用的 agent core 仍然应该能脱离当前桌面壳独立运行。</p>
        </div>
      </div>
    </CollapsiblePanel>
  </aside>
</template>

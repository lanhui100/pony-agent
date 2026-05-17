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
  health,
  phaseLabel,
  providerMode,
  providerModel,
  providerName,
  providerProtocol,
  providerRequestedName,
  sessionSummary,
  toolActivities,
  traceSteps
} = storeToRefs(runtimeStore);

const { currentProvider, currentModel } = storeToRefs(providerStore);
</script>

<template>
  <aside class="flex min-h-[calc(100vh-12rem)] min-w-0 flex-col gap-3 overflow-x-hidden">
    <CollapsiblePanel
      title="运行状态"
      description="当前回合到底有没有命中真实模型、为什么回退，都放在这里。"
      tip="学习阶段最关键的不是只看最后答案，而是确认这一轮的 provider、协议、模型和 fallback 状态。"
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
          <div><span class="font-medium text-stone-900">当前 provider：</span>{{ providerName || currentProvider?.name || "尚未执行" }}</div>
          <div><span class="font-medium text-stone-900">协议：</span>{{ providerProtocol || currentProvider?.protocol || "尚未执行" }}</div>
          <div><span class="font-medium text-stone-900">模型：</span>{{ providerModel || currentModel?.name || "尚未执行" }}</div>
          <div><span class="font-medium text-stone-900">模式：</span>{{ providerMode || "尚未执行" }}</div>
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
      description="这里展示这轮返回给前端的结构化执行轨迹。"
      tip="后续真正接工具链后，这里应继续承接 plan / model / tool / observe 的过程信号，而不是只做静态说明。"
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
          <Badge :variant="step.state === 'completed' ? 'success' : step.state === 'active' ? 'default' : 'secondary'">
            {{ step.state === "completed" ? "已完成" : step.state === "active" ? "进行中" : "未进入" }}
          </Badge>
        </div>

        <div
          v-for="tool in toolActivities"
          :key="tool.id"
          class="rounded-[0.45rem] bg-white px-3 py-3 text-sm leading-6 text-stone-700"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="font-medium text-stone-900">{{ tool.name }}</div>
            <Badge variant="secondary">{{ tool.status }}</Badge>
          </div>
          <p class="mt-1 text-[13px] text-stone-500">{{ tool.summary }}</p>
        </div>
      </div>
    </CollapsiblePanel>

    <CollapsiblePanel
      title="学习指导"
      description="把这块页面当成工作台，而不是最终产品页。"
      tip="你可以先看状态，再看 trace，最后回到对话区交叉验证，这样更容易理解 Rust core 的边界。"
      tone="slate"
      :default-open="false"
    >
      <template #icon>
        <BookOpen class="h-4 w-4 text-amber-800" />
      </template>

      <div class="grid gap-2 text-sm leading-6 text-stone-600">
        <div class="rounded-[0.45rem] bg-white px-3 py-3">
          <div class="font-medium text-stone-900">先看状态</div>
          <p class="mt-1 text-[13px]">确认这轮是否真的命中了 live provider，还是因为凭证缺失或请求失败回退到 mock。</p>
        </div>
        <div class="rounded-[0.45rem] bg-white px-3 py-3">
          <div class="font-medium text-stone-900">再看 trace</div>
          <p class="mt-1 text-[13px]">关注当前回合有哪些结构化步骤返回到了前端，后面工具链接入时也会沿着这条结构扩展。</p>
        </div>
        <div class="rounded-[0.45rem] bg-white px-3 py-3">
          <div class="font-medium text-stone-900">最后看配置</div>
          <p class="mt-1 text-[13px]">模型配置页负责 provider 与 model 管理，但真正部署时，Rust core 仍应可脱离当前桌面壳单独运行。</p>
        </div>
      </div>
    </CollapsiblePanel>
  </aside>
</template>

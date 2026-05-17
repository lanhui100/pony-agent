<script setup lang="ts">
import { storeToRefs } from "pinia";
import { ArrowRight, BookOpenText } from "lucide-vue-next";
import { useRuntimeStore } from "@/stores/runtime";
import Button from "@/components/ui/Button.vue";
import Card from "@/components/ui/Card.vue";
import Input from "@/components/ui/Input.vue";
import Separator from "@/components/ui/Separator.vue";

const runtimeStore = useRuntimeStore();
const { draftMessage, isSubmitting, messages, providerMode } = storeToRefs(runtimeStore);
</script>

<template>
  <Card class="flex h-full min-h-[32rem] flex-col p-4">
    <div class="flex items-start justify-between gap-3">
      <div class="space-y-1">
        <div class="text-sm font-semibold text-slate-900">1. 触发一轮执行</div>
        <p class="text-sm leading-6 text-slate-600">
          在这里输入一句话并发送，你触发的是一次最小可观察的
          <code class="rounded bg-slate-100 px-1.5 py-0.5 text-[13px] text-slate-800">run_turn()</code>
          入口。
        </p>
      </div>
      <BookOpenText class="mt-1 h-4 w-4 shrink-0 text-slate-400" />
    </div>

    <div class="mt-4 rounded-xl border border-slate-200 bg-slate-50/80 px-3 py-2 text-[13px] leading-6 text-slate-600">
      学习提示：发送后会经过这条链路
      <span class="font-medium text-slate-800">前端输入 -> Tauri 命令 -> Rust run_turn() -> provider 决策 -> 结构化结果 -> UI 回显</span>
    </div>

    <div class="mt-4 flex-1 space-y-3 overflow-auto pr-1">
      <div
        v-for="message in messages"
        :key="message.id"
        class="rounded-2xl border px-3.5 py-3 text-sm leading-6"
        :class="
          message.role === 'user'
            ? 'ml-8 border-slate-200 bg-slate-100 text-slate-800'
            : 'mr-8 border-emerald-100 bg-emerald-50/80 text-emerald-900'
        "
      >
        <div class="mb-1 text-[11px] font-medium uppercase tracking-wide text-slate-500">
          {{ message.role === "user" ? "你的输入" : "Pony Agent 返回" }}
        </div>
        <div>{{ message.content }}</div>
      </div>
    </div>

    <Separator class="my-4" />

    <div class="space-y-2">
      <label class="text-xs font-medium tracking-wide text-slate-500">本轮输入</label>
      <div class="flex gap-2">
        <Input
          :model-value="draftMessage"
          class="h-11"
          placeholder="例如：解释一下这轮为什么会回退到 mock"
          @update:model-value="runtimeStore.setDraftMessage($event)"
          @keydown.enter="runtimeStore.submitTurn()"
        />
        <Button class="h-11 rounded-xl px-4" :disabled="isSubmitting" @click="runtimeStore.submitTurn()">
          <ArrowRight class="mr-1 h-4 w-4" />
          {{ isSubmitting ? "执行中" : "发送" }}
        </Button>
      </div>
      <p class="text-xs leading-5 text-slate-500">
        当前版本已经可以区分真实 provider 与本地 mock，并在右侧明确显示本轮的运行模式。
        <span v-if="providerMode" class="font-medium text-slate-700">当前最近一轮：{{ providerMode }}</span>
      </p>
    </div>
  </Card>
</template>

<script setup lang="ts">
import { Lightbulb, Route, Sigma } from "lucide-vue-next";
import Card from "@/components/ui/Card.vue";
</script>

<template>
  <Card class="border-slate-900/10 bg-slate-900 text-slate-50">
    <div class="space-y-4 p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="space-y-1">
          <div class="text-sm font-semibold">4. 这块界面该怎么读</div>
          <p class="text-sm leading-6 text-slate-300">
            你现在看到的是一个“学习型调试面板”，不是最终产品页。它的任务是把 Agent 内部过程尽量翻译成人能理解的中文。
          </p>
        </div>
        <Lightbulb class="mt-1 h-4 w-4 shrink-0 text-slate-400" />
      </div>

      <div class="grid gap-3 sm:grid-cols-3">
        <div class="rounded-xl border border-white/10 bg-white/5 p-3">
          <div class="mb-2 flex items-center gap-2 text-sm font-medium">
            <Route class="h-4 w-4 text-slate-300" />
            看流程
          </div>
          <p class="text-xs leading-5 text-slate-300">先看左侧输入，再看右上状态，最后看右下轨迹，你会更容易理解一次 turn 的边界。</p>
        </div>

        <div class="rounded-xl border border-white/10 bg-white/5 p-3">
          <div class="mb-2 flex items-center gap-2 text-sm font-medium">
            <Sigma class="h-4 w-4 text-slate-300" />
            看结构
          </div>
          <p class="text-xs leading-5 text-slate-300">当前重点不是“AI 聪明不聪明”，而是理解 Rust 返回的数据结构长什么样、为何这样设计。</p>
        </div>

        <div class="rounded-xl border border-white/10 bg-white/5 p-3">
          <div class="mb-2 flex items-center gap-2 text-sm font-medium">
            <Lightbulb class="h-4 w-4 text-slate-300" />
            看下一步
          </div>
          <p class="text-xs leading-5 text-slate-300">下一阶段会在不破坏这个界面的前提下，把 run_turn 从“模拟回包”升级成“真实模型调用”。</p>
        </div>
      </div>
    </div>
  </Card>
</template>

<script setup lang="ts">
import { Clock, ChevronsDownUp, ChevronsUpDown, Search, X } from "lucide-vue-next";

defineProps<{
  actualDuration: boolean;
  allTurnsCollapsed: boolean;
  allCallsCollapsed?: boolean;
  searchQuery: string;
}>();

const emit = defineEmits<{
  "update:actualDuration": [value: boolean];
  "update:searchQuery": [value: string];
  "toggle-all-turns": [];
  "toggle-all-calls": [];
}>();
</script>

<template>
  <header
    class="flex flex-wrap items-center justify-between gap-2 border-b border-stone-200/70 bg-[#faf8f5]/90 px-3 py-2 text-xs backdrop-blur-sm dark:border-stone-800 dark:bg-stone-900/90"
    role="toolbar"
    aria-label="轨迹控制工具栏"
  >
    <div class="flex items-center gap-1.5">
      <!-- 实际时长 / 等宽切换 -->
      <button
        type="button"
        class="inline-flex items-center gap-1.5 rounded-md px-2 py-1 font-medium transition"
        :class="
          actualDuration
            ? 'bg-amber-100/80 text-amber-900 shadow-xs dark:bg-amber-950/60 dark:text-amber-200'
            : 'bg-stone-100 text-stone-600 hover:bg-stone-200/80 dark:bg-stone-800 dark:text-stone-300 dark:hover:bg-stone-700'
        "
        :title="actualDuration ? '切换为等宽模式' : '切换为实际耗时模式'"
        :aria-pressed="actualDuration"
        data-testid="trajectory-toolbar-duration-toggle"
        @click="emit('update:actualDuration', !actualDuration)"
      >
        <Clock class="h-3.5 w-3.5" />
        <span>{{ actualDuration ? "实际耗时" : "等宽视图" }}</span>
      </button>

      <div class="h-3.5 w-[1px] bg-stone-300 dark:bg-stone-700 mx-0.5" aria-hidden="true" />

      <!-- 全部展开/折叠轮次 -->
      <button
        type="button"
        class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-stone-600 transition hover:bg-stone-100 hover:text-stone-900 dark:text-stone-400 dark:hover:bg-stone-800 dark:hover:text-stone-100"
        :title="allTurnsCollapsed ? '展开所有轮次' : '折叠所有轮次'"
        :aria-pressed="allTurnsCollapsed"
        data-testid="trajectory-toolbar-turns-toggle"
        @click="emit('toggle-all-turns')"
      >
        <component :is="allTurnsCollapsed ? ChevronsUpDown : ChevronsDownUp" class="h-3.5 w-3.5" />
        <span>{{ allTurnsCollapsed ? "展开轮次" : "折叠轮次" }}</span>
      </button>

      <!-- 全部展开/折叠调用详情 -->
      <button
        type="button"
        class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-stone-600 transition hover:bg-stone-100 hover:text-stone-900 dark:text-stone-400 dark:hover:bg-stone-800 dark:hover:text-stone-100"
        :title="allCallsCollapsed ? '展开所有调用详情' : '折叠所有调用详情'"
        :aria-pressed="allCallsCollapsed"
        data-testid="trajectory-toolbar-calls-toggle"
        @click="emit('toggle-all-calls')"
      >
        <component :is="allCallsCollapsed ? ChevronsUpDown : ChevronsDownUp" class="h-3.5 w-3.5" />
        <span>{{ allCallsCollapsed ? "展开调用" : "折叠调用" }}</span>
      </button>
    </div>

    <!-- 搜索筛选框 -->
    <div class="relative flex items-center min-w-[160px] sm:w-56">
      <Search class="pointer-events-none absolute left-2.5 h-3.5 w-3.5 text-stone-400" />
      <input
        type="search"
        :value="searchQuery"
        placeholder="搜索轨迹 (step, tool, error)..."
        class="w-full rounded-md border border-stone-200/80 bg-white/90 py-1 pl-8 pr-7 text-xs text-stone-800 placeholder-stone-400 outline-none transition focus:border-amber-400 focus:ring-1 focus:ring-amber-300 dark:border-stone-700 dark:bg-stone-800 dark:text-stone-100 dark:placeholder-stone-500"
        data-testid="trajectory-toolbar-search-input"
        @input="emit('update:searchQuery', ($event.target as HTMLInputElement).value)"
      />
      <button
        v-if="searchQuery"
        type="button"
        class="absolute right-2 text-stone-400 hover:text-stone-600 dark:hover:text-stone-200"
        title="清空搜索"
        @click="emit('update:searchQuery', '')"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </div>
  </header>
</template>

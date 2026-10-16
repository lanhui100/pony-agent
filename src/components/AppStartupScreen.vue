<script setup lang="ts">
import PonyBrandIcon from "@/components/PonyBrandIcon.vue";

const props = withDefaults(
  defineProps<{
    isInitializing: boolean;
    stageText?: string;
  }>(),
  {
    stageText: "正在初始化系统核心..."
  }
);

const emit = defineEmits<{
  (event: "transitionEnd"): void;
}>();

function handleAfterLeave() {
  emit("transitionEnd");
}
</script>

<template>
  <Transition
    enter-active-class="transition-opacity duration-300 ease-out"
    enter-from-class="opacity-0"
    enter-to-class="opacity-100"
    leave-active-class="transition-all duration-500 cubic-bezier(0.16, 1, 0.3, 1)"
    leave-from-class="opacity-100 scale-100 filter blur-0"
    leave-to-class="opacity-0 scale-[1.03] filter blur-sm pointer-events-none"
    @after-leave="handleAfterLeave"
  >
    <div
      v-if="isInitializing"
      data-testid="app-startup-screen"
      class="fixed inset-0 z-50 flex flex-col items-center justify-center select-none overflow-hidden bg-[#faf7f2]/85 dark:bg-[#1c1917]/85 backdrop-blur-xl"
    >
      <!-- 环境光晕背景 (CDS Warm Clay / Amber Glow) -->
      <div
        aria-hidden="true"
        class="pointer-events-none absolute -top-24 left-1/2 -translate-x-1/2 h-[32rem] w-[32rem] rounded-full bg-gradient-to-b from-amber-200/30 via-orange-100/15 to-transparent blur-3xl dark:from-amber-700/15 dark:via-orange-950/10"
      />

      <div class="relative z-10 flex flex-col items-center gap-6 px-6 text-center">
        <!-- 品牌标识与微光脉冲环 -->
        <div class="relative flex items-center justify-center">
          <div
            aria-hidden="true"
            class="absolute -inset-4 rounded-3xl bg-amber-500/15 animate-ping opacity-60 duration-1000 dark:bg-amber-400/10"
          />
          <div
            class="relative flex h-20 w-20 items-center justify-center rounded-2xl bg-white/90 shadow-[0_8px_30px_rgb(0,0,0,0.06)] ring-1 ring-black/5 backdrop-blur-md dark:bg-stone-900/90 dark:ring-white/10 dark:shadow-[0_8px_30px_rgb(0,0,0,0.3)]"
          >
            <PonyBrandIcon className="h-11 w-11" />
          </div>
        </div>

        <!-- 标题与状态提示 -->
        <div class="flex flex-col items-center gap-2">
          <h1 class="text-lg font-semibold tracking-tight text-stone-900 dark:text-stone-100">
            Pony Agent
          </h1>
          <p
            data-testid="app-startup-stage"
            class="text-xs text-stone-500 dark:text-stone-400 font-medium tracking-wide flex items-center gap-2"
          >
            <span class="inline-flex items-center gap-1" aria-hidden="true">
              <span class="h-1.5 w-1.5 rounded-full bg-amber-600 animate-pulse" />
            </span>
            <span>{{ stageText }}</span>
          </p>
        </div>
      </div>
    </div>
  </Transition>
</template>
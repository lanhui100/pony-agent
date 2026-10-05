<script setup lang="ts">
import { computed, ref } from "vue";
import { ArrowUp, Clock, Trash2, Zap } from "lucide-vue-next";
import type { QueuedMessageItem } from "@/types/runtime";
import {
  TooltipContent,
  TooltipPortal,
  TooltipProvider,
  TooltipRoot,
  TooltipTrigger,
} from "reka-ui";

const props = defineProps<{
  messages: QueuedMessageItem[];
}>();

const emit = defineEmits<{
  (event: "steer", id: string): void;
  (event: "remove", id: string): void;
}>();

const isHovered = ref(false);

const isStacked = computed(() => props.messages.length > 2);

function getStackTransform(index: number): Record<string, string> {
  if (!isStacked.value || isHovered.value) {
    return {
      transform: "none",
      zIndex: `${props.messages.length - index}`,
      opacity: "1",
    };
  }

  // 默认堆叠态：最早的消息在最上面 (index 0 在顶)，后续错落沉降
  // index 0: offset 0, scale 1.0
  // index 1: offset 6px, scale 0.98
  // index >= 2: offset 12px, scale 0.96
  const visualIndex = Math.min(index, 3);
  const offsetY = visualIndex * 5;
  const scale = 1 - visualIndex * 0.025;
  const opacity = index > 2 ? "0" : `${1 - visualIndex * 0.15}`;

  return {
    transform: `translateY(${offsetY}px) scale(${scale})`,
    zIndex: `${props.messages.length - index}`,
    opacity,
  };
}
</script>

<template>
  <TooltipProvider :delay-duration="200">
    <div
      v-if="messages.length > 0"
      class="queue-container mx-auto mb-2 w-full max-w-[46.4rem] px-4 sm:px-5"
      data-testid="queued-messages-container"
      @mouseenter="isHovered = true"
      @mouseleave="isHovered = false"
    >
      <div
        class="relative flex flex-col items-end transition-all duration-300 ease-out"
        :class="[isStacked && !isHovered ? 'pb-3' : 'space-y-2 pb-0']"
      >
        <div
          v-for="(msg, index) in messages"
          :key="msg.id"
          class="queue-bubble group relative flex max-w-[85%] items-center justify-between gap-3 rounded-2xl border border-stone-200/70 bg-stone-100/90 px-3.5 py-2 text-[13px] text-stone-600 shadow-sm backdrop-blur-md transition-all duration-300 ease-out"
          :style="getStackTransform(index)"
          :data-testid="`queued-message-${index}`"
        >
          <!-- 左侧排队标记与内容 -->
          <div class="flex min-w-0 items-center gap-2">
            <span
              v-if="msg.mode === 'steer'"
              class="flex items-center gap-1 rounded-full bg-amber-100 px-1.5 py-0.5 text-[10px] font-semibold text-amber-700"
              title="已设置为插队"
            >
              <Zap class="h-2.5 w-2.5 fill-current" />
              插队中
            </span>
            <span
              v-else
              class="flex items-center gap-1 rounded-full bg-stone-200/80 px-1.5 py-0.5 text-[10px] font-medium text-stone-500"
            >
              <Clock class="h-2.5 w-2.5" />
              #{{ index + 1 }} 排队
            </span>
            <span class="truncate font-normal select-text">{{ msg.content }}</span>
          </div>

          <!-- 右侧操作栏 (同款上箭头插队 + 删除) -->
          <div class="flex shrink-0 items-center gap-1 opacity-70 transition-opacity group-hover:opacity-100">
            <TooltipRoot :delay-duration="200">
              <TooltipTrigger as-child>
                <button
                  class="flex h-6 w-6 items-center justify-center rounded-full bg-stone-200/60 text-stone-600 transition-colors hover:bg-stone-800 hover:text-white"
                  type="button"
                  :data-testid="`queue-steer-btn-${index}`"
                  aria-label="立即插队"
                  @click.stop="emit('steer', msg.id)"
                >
                  <ArrowUp class="h-3 w-3" />
                </button>
              </TooltipTrigger>
              <TooltipPortal>
                <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-2.5 py-1 text-[11px] text-stone-700 shadow-sm">
                  立即插队 (优先在当前回合下一步执行)
                </TooltipContent>
              </TooltipPortal>
            </TooltipRoot>

            <TooltipRoot :delay-duration="200">
              <TooltipTrigger as-child>
                <button
                  class="flex h-6 w-6 items-center justify-center rounded-full bg-stone-200/60 text-stone-500 transition-colors hover:bg-red-50 hover:text-red-600"
                  type="button"
                  :data-testid="`queue-remove-btn-${index}`"
                  aria-label="删除排队"
                  @click.stop="emit('remove', msg.id)"
                >
                  <Trash2 class="h-3 w-3" />
                </button>
              </TooltipTrigger>
              <TooltipPortal>
                <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-2.5 py-1 text-[11px] text-stone-700 shadow-sm">
                  删除此条排队消息
                </TooltipContent>
              </TooltipPortal>
            </TooltipRoot>
          </div>
        </div>
      </div>
    </div>
  </TooltipProvider>
</template>

<style scoped>
.queue-bubble {
  transform-origin: top center;
}
</style>

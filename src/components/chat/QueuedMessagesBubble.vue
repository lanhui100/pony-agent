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

/** 折叠态：多于 1 条且未 hover 时，仅 index 0（下一条将执行）完整可见，其余为堆叠边缘。 */
const isCollapsed = computed(() => props.messages.length > 1 && !isHovered.value);

function rowStyle(index: number): Record<string, string> {
  if (!isCollapsed.value) {
    return {
      position: "relative",
      transform: "none",
      opacity: "1",
      zIndex: `${props.messages.length - index}`,
      marginTop: index > 0 ? "0.5rem" : "0",
    };
  }

  // 折叠堆叠态：index 0 在顶（完整气泡），后续消息作为堆叠边缘错落沉降（不可操作）
  const visualIndex = Math.min(index, 3);
  const offsetY = visualIndex * 5;
  const scale = 1 - visualIndex * 0.025;
  const opacity = index > 2 ? 0 : 1 - visualIndex * 0.15;

  if (index === 0) {
    return {
      position: "relative",
      transform: "scale(1)",
      opacity: "1",
      zIndex: `${props.messages.length}`,
    };
  }

  return {
    position: "absolute",
    top: `calc(100% + ${offsetY}px)`,
    right: "0",
    transform: `scale(${scale})`,
    opacity: String(opacity),
    zIndex: `${props.messages.length - index}`,
    pointerEvents: "none",
  };
}
</script>

<template>
  <TooltipProvider :delay-duration="200">
    <div
      v-if="messages.length > 0"
      class="queue-container mx-auto w-full max-w-[46.4rem] px-4 py-2 sm:px-5"
      data-testid="queued-messages-container"
      @mouseenter="isHovered = true"
      @mouseleave="isHovered = false"
    >
      <div class="relative flex flex-col items-end">
        <!-- 折叠态堆叠数量徽标（n > 1 时展示） -->
        <span
          v-if="messages.length > 1 && !isHovered"
          class="absolute -bottom-2.5 right-0 z-50 flex h-5 min-w-5 items-center justify-center rounded-full bg-stone-700 px-1.5 text-[10px] font-semibold text-white shadow-sm"
          data-testid="queue-stack-count"
        >
          +{{ messages.length - 1 }}
        </span>

        <div
          v-for="(msg, index) in messages"
          :key="msg.id"
          class="queue-bubble group relative flex max-w-[85%] items-center justify-between gap-3 rounded-[0.45rem] border border-stone-200/80 bg-stone-100/90 px-3.5 py-2 text-[13px] text-stone-700 shadow-sm transition-all duration-[240ms] ease-out"
          :data-testid="`queued-message-${index}`"
          :data-expanded="isHovered ? 'true' : 'false'"
          :style="rowStyle(index)"
        >
          <!-- 内容区：折叠态仅 index 0（下一条将执行）可见，堆叠边缘不展示文本 -->
          <template v-if="isHovered || index === 0">
            <div class="flex min-w-0 items-center gap-2">
              <span
                v-if="msg.mode === 'steer'"
                class="flex items-center gap-1 rounded-full bg-stone-200 px-1.5 py-0.5 text-[10px] font-semibold text-stone-700"
                title="已设置为插队"
              >
                <Zap class="h-2.5 w-2.5 fill-current" />
                插队中
              </span>
              <span
                v-else
                class="flex items-center gap-1 rounded-full bg-stone-200/80 px-1.5 py-0.5 text-[10px] font-medium text-stone-600"
              >
                <Clock class="h-2.5 w-2.5" />
                #{{ index + 1 }} 排队
              </span>
              <span class="truncate font-normal select-text">{{ msg.content }}</span>
            </div>

            <!-- 右侧操作栏：折叠态仅 index 0 可操作，堆叠边缘不可操作 -->
            <div
              v-if="isHovered || index === 0"
              class="flex shrink-0 items-center gap-1 opacity-70 transition-opacity group-hover:opacity-100"
            >
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
          </template>
        </div>
      </div>
    </div>
  </TooltipProvider>
</template>

<style scoped>
.queue-bubble {
  transform-origin: top right;
}
</style>

<script setup lang="ts">
import {
  PopoverArrow,
  PopoverClose,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger
} from "reka-ui";

withDefaults(
  defineProps<{
    title: string;
    description?: string;
    confirmText?: string;
    cancelText?: string;
    side?: "top" | "right" | "bottom" | "left";
    align?: "start" | "center" | "end";
  }>(),
  {
    description: "",
    confirmText: "确认删除",
    cancelText: "取消",
    side: "bottom",
    align: "end"
  }
);

const emit = defineEmits<{
  (event: "confirm"): void;
}>();
</script>

<template>
  <PopoverRoot>
    <PopoverTrigger as-child>
      <slot />
    </PopoverTrigger>

    <PopoverPortal>
      <PopoverContent
        :side="side"
        :align="align"
        :side-offset="8"
        class="z-50 w-60 rounded-[0.35rem] bg-white/95 px-3 py-2.5 text-stone-900 shadow-lg ring-1 ring-stone-900/8 backdrop-blur"
      >
        <div class="text-[12px] font-medium leading-5">{{ title }}</div>
        <div v-if="description" class="mt-0.5 text-[11px] leading-5 text-stone-500">
          {{ description }}
        </div>
        <div class="mt-2 flex justify-end gap-0.5">
          <PopoverClose as-child>
            <button
              type="button"
              class="confirm-popover-action text-stone-400 transition hover:bg-stone-100/70 hover:text-stone-700"
              data-testid="confirm-popover-cancel"
            >
              {{ cancelText }}
            </button>
          </PopoverClose>
          <PopoverClose as-child>
            <button
              type="button"
              class="confirm-popover-action bg-rose-50/70 text-rose-600 transition hover:bg-rose-100/80 hover:text-rose-700"
              data-testid="confirm-popover-confirm"
              @click="emit('confirm')"
            >
              {{ confirmText }}
            </button>
          </PopoverClose>
        </div>
        <PopoverArrow class="fill-white/95" :width="10" :height="5" />
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>

<style scoped>
.confirm-popover-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  height: 1.375rem;
  padding: 0 0.5rem;
  border-radius: 0.18rem;
  font-size: 0.6875rem;
  font-weight: 400;
  line-height: 1;
  letter-spacing: 0.01em;
}
</style>

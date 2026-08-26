<script setup lang="ts">
// 二次确认弹层（reka-ui Popover 封装）。
// 两种用法：
// 1) 非受控（向后兼容）：不传 open —— confirm/cancel 点击即关闭，行为与历史版本一致；
// 2) 受控异步：传入 open(v-model) + loading(+error) —— 确认键不再立即关闭，
//    loading 期间双按钮禁用并显示 spinner，失败经 error 槽展示、可重试；
//    Esc/外点关闭等价取消，不打断进行中的请求；防重复提交由调用方 inflight 守卫兜底。
import { computed } from "vue";
import { LoaderCircle } from "lucide-vue-next";
import {
  PopoverArrow,
  PopoverClose,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger
} from "reka-ui";

const props = withDefaults(
  defineProps<{
    title: string;
    description?: string;
    confirmText?: string;
    cancelText?: string;
    side?: "top" | "right" | "bottom" | "left";
    align?: "start" | "center" | "end";
    /** 受控开关：传入即进入受控异步模式。 */
    open?: boolean;
    /** 确认请求进行中（仅受控模式消费）。 */
    loading?: boolean;
    /** 失败信息（仅受控模式消费；非空时展示并可重试）。 */
    error?: string;
  }>(),
  {
    description: "",
    confirmText: "确认删除",
    cancelText: "取消",
    side: "bottom",
    align: "end",
    open: undefined,
    loading: false,
    error: ""
  }
);

const emit = defineEmits<{
  (event: "confirm"): void;
  (event: "update:open", open: boolean): void;
}>();

const controlled = computed(() => props.open !== undefined);
</script>

<template>
  <PopoverRoot
    :open="controlled ? props.open : undefined"
    @update:open="(value: boolean) => { if (controlled) emit('update:open', value); }"
  >
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
        <div v-if="description" class="mt-0.5 whitespace-pre-line text-[11px] leading-5 text-stone-500">
          {{ description }}
        </div>
        <p
          v-if="error"
          class="mt-1 text-[10px] leading-4 text-rose-600"
          data-testid="confirm-popover-error"
        >
          {{ error }}
        </p>
        <div class="mt-2 flex justify-end gap-0.5">
          <PopoverClose as-child>
            <button
              type="button"
              class="confirm-popover-action text-stone-400 transition hover:bg-stone-100/70 hover:text-stone-700 disabled:cursor-not-allowed disabled:opacity-50"
              data-testid="confirm-popover-cancel"
              :disabled="loading"
            >
              {{ cancelText }}
            </button>
          </PopoverClose>
          <PopoverClose v-if="!controlled" as-child>
            <button
              type="button"
              class="confirm-popover-action bg-rose-50/70 text-rose-600 transition hover:bg-rose-100/80 hover:text-rose-700"
              data-testid="confirm-popover-confirm"
              @click="emit('confirm')"
            >
              {{ confirmText }}
            </button>
          </PopoverClose>
          <button
            v-else
            type="button"
            class="confirm-popover-action inline-flex items-center gap-1 bg-rose-50/70 text-rose-600 transition hover:bg-rose-100/80 hover:text-rose-700 disabled:cursor-not-allowed disabled:opacity-60"
            data-testid="confirm-popover-confirm"
            :disabled="loading"
            @click="emit('confirm')"
          >
            <LoaderCircle v-if="loading" class="h-3 w-3 animate-spin" />
            {{ loading ? "处理中…" : confirmText }}
          </button>
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

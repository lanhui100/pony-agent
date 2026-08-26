<script setup lang="ts">
// 三点操作菜单基件（reka-ui DropdownMenu 封装）：行容器锚定触发器，
// 菜单项支持 danger / disabled + 原因 tooltip；选择即回传 id 后关闭。
// 已知限制：
// - select 事件截断了 reka 的可取消 CustomEvent——不支持"点击后保持菜单
//   开启"的 checkbox 类条目（未来扩展即 API break）；
// - 内容区未设 max-height 滚动上限（当前条目固定少量；长菜单化前需补）。
import type { Component } from "vue";
import {
  DropdownMenuArrow,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger
} from "reka-ui";

export interface DropdownMenuItemSpec {
  id: string;
  label: string;
  icon?: Component;
  /** 危险动作渲染为 rose 语义。 */
  danger?: boolean;
  disabled?: boolean;
  /** 禁用原因（原生 title 提示，对齐既有按钮的 tooltip 惯例）。 */
  disabledTitle?: string;
}

withDefaults(
  defineProps<{
    items: DropdownMenuItemSpec[];
    align?: "start" | "center" | "end";
    /** 分组间显示分隔线（按相邻项 danger 差异自动插入由调用方控制）。 */
    showArrow?: boolean;
  }>(),
  {
    align: "end",
    showArrow: true
  }
);

const emit = defineEmits<{
  (event: "select", id: string): void;
}>();

function onDisabledTitle(item: DropdownMenuItemSpec) {
  return item.disabled ? item.disabledTitle ?? undefined : undefined;
}
</script>

<template>
  <DropdownMenuRoot>
    <DropdownMenuTrigger as-child>
      <slot />
    </DropdownMenuTrigger>

    <DropdownMenuPortal>
      <DropdownMenuContent
        :align="align"
        :side-offset="6"
        class="z-50 min-w-[7rem] rounded-[0.35rem] bg-white/95 py-1 shadow-lg ring-1 ring-stone-900/8 backdrop-blur"
      >
        <template v-for="(item, index) in items" :key="item.id">
          <DropdownMenuSeparator
            v-if="index > 0 && items[index - 1].danger !== item.danger"
            class="my-1 h-px bg-stone-900/8"
          />
          <DropdownMenuItem
            class="flex cursor-pointer items-center gap-1.5 px-2 py-1 text-[11px] leading-4 text-stone-700 outline-none transition-colors data-[highlighted]:bg-[#f6dfb8] data-[highlighted]:text-stone-900 data-[disabled]:cursor-not-allowed data-[disabled]:opacity-40 data-[disabled]:data-[highlighted]:bg-transparent data-[disabled]:data-[highlighted]:text-stone-700"
            :class="item.danger ? 'text-rose-600 data-[highlighted]:bg-rose-50/80 data-[highlighted]:text-rose-700' : ''"
            :disabled="item.disabled"
            :title="onDisabledTitle(item)"
            :data-testid="`dropdown-item-${item.id}`"
            @select="() => emit('select', item.id)"
          >
            <component
              :is="item.icon"
              v-if="item.icon"
              class="h-3 w-3 shrink-0"
            />
            <span class="min-w-0 truncate">{{ item.label }}</span>
          </DropdownMenuItem>
        </template>

        <DropdownMenuArrow v-if="showArrow" class="fill-white/95" :width="10" :height="5" />
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>

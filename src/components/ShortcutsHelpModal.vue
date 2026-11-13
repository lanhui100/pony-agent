<script setup lang="ts">
import { computed } from "vue";
import {
  DialogRoot,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "reka-ui";
import { Keyboard, X } from "lucide-vue-next";
import {
  SHORTCUT_SECTIONS,
  isMacPlatform,
  resolveShortcutKey,
  type ShortcutSection
} from "@/lib/chat/keyboard-shortcuts";

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
}>();

const isOpen = computed({
  get: () => props.open,
  set: (val: boolean) => emit("update:open", val),
});

const isMac = typeof navigator !== "undefined" && isMacPlatform(navigator.platform);

/**
 * 渲染投影：清单真值源在 `@/lib/chat/keyboard-shortcuts`（纯数据 + 平台无关 token），
 * 组件只负责把 `Mod` 解析成当前平台的 ⌘ / Ctrl 后渲染，避免与键盘 handler 手写漂移。
 */
const shortcutSections = computed<ShortcutSection[]>(() =>
  SHORTCUT_SECTIONS.map((section) => ({
    title: section.title,
    items: section.items.map((item) => ({
      description: item.description,
      keys: item.keys.map((key) => resolveShortcutKey(key, isMac))
    }))
  }))
);
</script>

<template>
  <DialogRoot v-model:open="isOpen">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-[90] bg-black/40 backdrop-blur-xs transition-opacity" />
      <DialogContent
        class="fixed left-1/2 top-1/2 z-[95] max-h-[85vh] w-[90vw] max-w-[34rem] -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-xl border border-stone-200/90 bg-white shadow-2xl transition-all focus:outline-none dark:border-stone-800 dark:bg-stone-900"
        data-testid="shortcuts-help-dialog"
      >
        <div class="flex items-center justify-between border-b border-stone-200/70 px-5 py-3.5 dark:border-stone-800">
          <div class="flex items-center gap-2">
            <div class="flex h-7 w-7 items-center justify-center rounded-lg bg-stone-100 text-stone-700 dark:bg-stone-800 dark:text-stone-300">
              <Keyboard class="h-4 w-4" />
            </div>
            <div>
              <DialogTitle class="text-[14px] font-semibold text-stone-900 dark:text-stone-100">
                快捷键指南
              </DialogTitle>
              <DialogDescription class="text-[11px] text-stone-500 dark:text-stone-400">
                按下快捷键即可快速执行常用操作
              </DialogDescription>
            </div>
          </div>
          <button
            type="button"
            class="rounded-md p-1 text-stone-400 hover:bg-stone-100 hover:text-stone-600 dark:hover:bg-stone-800 dark:hover:text-stone-200"
            aria-label="关闭快捷键帮助"
            @click="isOpen = false"
          >
            <X class="h-4 w-4" />
          </button>
        </div>

        <div class="max-h-[68vh] overflow-y-auto px-5 py-4 space-y-5">
          <div v-for="section in shortcutSections" :key="section.title" class="space-y-2">
            <h4 class="text-[11px] font-semibold uppercase tracking-wider text-stone-400 dark:text-stone-500">
              {{ section.title }}
            </h4>
            <div class="divide-y divide-stone-100 rounded-lg border border-stone-200/70 bg-stone-50/40 dark:divide-stone-800 dark:border-stone-800 dark:bg-stone-900/40">
              <div
                v-for="(item, idx) in section.items"
                :key="idx"
                class="flex items-center justify-between px-3 py-2 text-xs"
              >
                <span class="text-stone-700 dark:text-stone-300">{{ item.description }}</span>
                <div class="flex items-center gap-1 shrink-0 ml-2">
                  <kbd
                    v-for="(k, kIdx) in item.keys"
                    :key="kIdx"
                    class="min-w-5 rounded border border-stone-200 bg-white px-1.5 py-0.5 text-center font-mono text-[11px] font-medium text-stone-700 shadow-2xs dark:border-stone-700 dark:bg-stone-800 dark:text-stone-200"
                  >
                    {{ k }}
                  </kbd>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div class="border-t border-stone-200/70 bg-stone-50/60 px-5 py-2.5 text-right text-[11px] text-stone-400 dark:border-stone-800 dark:bg-stone-900/60">
          按 <kbd class="rounded border border-stone-200 bg-white px-1 py-0.2 font-mono text-[10px] text-stone-600 dark:border-stone-700 dark:bg-stone-800 dark:text-stone-300">Esc</kbd> 退出
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

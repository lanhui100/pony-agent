<script setup lang="ts">
// 三级树会话行（三级条目）：标题截断 + hover 原生 tooltip 全文 + 相对时间 +
// 三点菜单（重命名/归档/删除对话）+ 行内重命名。纯展示：数据与回调全经 props/emits。
import { computed, ref } from "vue";
import { Ellipsis } from "lucide-vue-next";
import DropdownMenu from "@/components/ui/DropdownMenu.vue";
import type { DropdownMenuItemSpec } from "@/components/ui/DropdownMenu.vue";
import type { SessionOverview } from "@/types/runtime";

defineOptions({ name: "SessionRow" });

const props = defineProps<{
  session: SessionOverview;
  menuItems: DropdownMenuItemSpec[];
  selected: boolean;
  renaming: boolean;
  renameDraft: string;
  renameError: string;
  renamingBusy: boolean;
  /** 隐藏三点菜单（瞬态空白会话：无可操作内容）。 */
  hideMenu?: boolean;
}>();

const emit = defineEmits<{
  (event: "open-session", conversationId: string): void;
  (event: "menu-select", itemId: string): void;
  (event: "update:renameDraft", value: string): void;
  (event: "submit-rename"): void;
  (event: "cancel-rename"): void;
}>();

const headline = computed(() =>
  props.session.title?.trim() || props.session.summary?.trim() || props.session.conversationId
);

const timeLabel = ref("");
function refreshTime() {
  if (!props.session.updatedAtMs) {
    timeLabel.value = "未保存";
    return;
  }
  const now = new Date();
  const updated = new Date(props.session.updatedAtMs);
  const sameDay =
    now.getFullYear() === updated.getFullYear()
    && now.getMonth() === updated.getMonth()
    && now.getDate() === updated.getDate();
  timeLabel.value = sameDay
    ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(updated)
    : `${Math.max(
        1,
        Math.floor(
          (new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
            - new Date(updated.getFullYear(), updated.getMonth(), updated.getDate()).getTime())
          / 86_400_000
        )
      )}天前`;
}
refreshTime();

function onRenameInput(event: Event) {
  emit("update:renameDraft", (event.target as HTMLInputElement).value);
}
</script>

<template>
  <div class="flex items-center gap-2 px-1.5 py-1" :title="renaming ? undefined : headline">
    <button
      v-if="!renaming"
      class="min-w-0 flex-1 text-left"
      type="button"
      :data-testid="`session-switch-${session.conversationId}`"
      @click="emit('open-session', session.conversationId)"
    >
      <span
        class="block truncate text-[12px] leading-5"
        :class="selected ? 'font-medium text-stone-900' : 'text-stone-700'"
      >{{ headline }}</span>
    </button>
    <input
      v-else
      class="min-w-0 flex-1 rounded-[0.2rem] border border-amber-300 bg-white px-1 py-0.5 text-[11px] outline-none"
      :value="renameDraft"
      :disabled="renamingBusy"
      data-testid="session-rename-input"
      @input="onRenameInput"
      @keydown.enter.prevent="emit('submit-rename')"
      @keydown.esc.prevent="emit('cancel-rename')"
    />
    <span v-if="!renaming" class="shrink-0 text-[10px] text-stone-400">{{ timeLabel }}</span>
    <DropdownMenu
      v-if="!renaming && menuItems.length > 0 && !props.hideMenu"
      :items="menuItems"
      @select="(id: string) => emit('menu-select', id)"
    >
      <button
        class="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-[0.2rem] text-stone-400 opacity-0 transition group-hover:opacity-100 group-focus-within:opacity-100 hover:bg-[#f7e3bf] hover:text-stone-900"
        type="button"
        aria-label="会话操作"
        :data-testid="`session-menu-${session.conversationId}`"
      >
        <Ellipsis class="h-3 w-3" />
      </button>
    </DropdownMenu>
    <template v-if="renaming">
      <p v-if="renameError" class="shrink-0 text-[10px] leading-3 text-rose-600">{{ renameError }}</p>
      <button
        class="shrink-0 text-[10px] text-stone-500 transition hover:text-stone-900 disabled:opacity-50"
        type="button"
        :disabled="renamingBusy"
        data-testid="session-rename-submit"
        @click="emit('submit-rename')"
      >确认</button>
      <button
        class="shrink-0 text-[10px] text-stone-400 transition hover:text-stone-700"
        type="button"
        data-testid="session-rename-cancel"
        @click="emit('cancel-rename')"
      >取消</button>
    </template>
  </div>
</template>

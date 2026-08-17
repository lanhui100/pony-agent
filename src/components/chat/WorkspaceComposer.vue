<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { ArrowUp, Check, ChevronDown, Paperclip, Search, Square, Undo2, X } from "lucide-vue-next";
import type { ProviderModelConfig, ProviderReasoningEffort } from "@/types/provider";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import { pickFiles } from "@/lib/runtime/file-attachments";
import Button from "@/components/ui/Button.vue";
import Switch from "@/components/ui/Switch.vue";
import {
  TooltipContent,
  TooltipPortal,
  TooltipRoot,
  TooltipTrigger,
} from "reka-ui";

type ComposerActionKind = "submit" | "resume" | "continue" | "restart";

const props = defineProps<{
  showReasoningContent: boolean;
  canUndoLastTurn: boolean;
  undoShortcutLabel: string;
  handleComposerKeydown: (event: KeyboardEvent) => void;
  handlePrimaryAction: () => void;
  handleUndoLastTurn: () => void;
  setComposerShellRef: (element: unknown) => void;
}>();

const emit = defineEmits<{
  (event: "update:showReasoningContent", value: boolean): void;
}>();

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();
const {
  draftMessage,
  isSubmitting,
  latestExecutionCheckpoint,
  latestGraphRunSubmissionPlan,
  latestRunControlAuditSummary,
  pendingAttachments,
  sessionOperation
} = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const menuOpen = ref(false);
const modelMenuOpen = ref(false);
const reasoningMenuOpen = ref(false);
const menuRef = ref<HTMLElement | null>(null);
const modelTriggerRef = ref<HTMLElement | null>(null);
const floatingMenuRef = ref<HTMLElement | null>(null);
const modelSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const reasoningSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const floatingMenuStyle = ref<Record<string, string>>({
  left: '0px',
  top: '0px',
  visibility: 'hidden'
});
const modelSubmenuDirection = ref<'left' | 'right'>('right');
const reasoningSubmenuDirection = ref<'left' | 'right'>('right');

const modelSearchQuery = ref("");
const modelSearchDebounced = ref("");
let modelSearchDebounceTimer: ReturnType<typeof setTimeout> | undefined;
const INTERNAL_MODEL_NAME_PATTERN = /^model-(?:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|\d+-[0-9a-f]{6}|[a-z0-9-]+-default)$/i;

watch(modelSearchQuery, (value) => {
  if (modelSearchDebounceTimer) {
    clearTimeout(modelSearchDebounceTimer);
  }
  modelSearchDebounceTimer = setTimeout(() => {
    modelSearchDebounced.value = value.trim().toLowerCase();
  }, 250);
});

const filteredProviders = computed(() => {
  const keyword = modelSearchDebounced.value;
  if (!keyword) {
    return providerStore.providers;
  }

  return providerStore.providers
    .map((provider) => ({
      ...provider,
      models: (provider.models ?? []).filter((model) =>
        modelDisplayName(model).toLowerCase().includes(keyword) ||
        model.model.toLowerCase().includes(keyword)
      )
    }))
    .filter((provider) => (provider.models?.length ?? 0) > 0);
});

const currentModelSupportsReasoning = computed(
  () => currentModel.value?.capabilities?.supportsReasoning ?? false
);

const providerTooltipDetail = computed(() => {
  const providerName = currentProvider.value?.name?.trim() || "未选择";
  return `当前：${providerName}`;
});

function modelDisplayName(model: Pick<ProviderModelConfig, "name" | "model"> | null | undefined) {
  const displayName = model?.name?.trim() ?? "";
  if (displayName && !INTERNAL_MODEL_NAME_PATTERN.test(displayName)) {
    return displayName;
  }

  return model?.model?.trim() || "未命名模型";
}

const reasoningSummary = computed(() => {
  if (!currentModelSupportsReasoning.value) {
    return "不支持";
  }

  return providerStore.currentReasoningEffort
    ? reasoningEffortLabelZh(providerStore.currentReasoningEffort)
    : "默认";
});

const reasoningOptions: Array<{ label: string; value: ProviderReasoningEffort | null }> = [
  { label: "默认", value: null },
  { label: "低", value: "low" },
  { label: "中", value: "medium" },
  { label: "高", value: "high" },
  { label: "极高", value: "max" }
];

function reasoningEffortLabelZh(value: ProviderReasoningEffort) {
  switch (value) {
    case "low":
      return "低";
    case "medium":
      return "中";
    case "high":
      return "高";
    case "max":
      return "极高";
  }
}

const composerAction = computed<{
  kind: ComposerActionKind;
  label: string;
  hint: string;
}>(() => {
  const actionSummary = latestRunControlAuditSummary.value?.actionEvidenceSummary ?? null;
  const planCommand = latestGraphRunSubmissionPlan.value?.command?.trim().toLowerCase() || null;
  const checkpointCommand = latestExecutionCheckpoint.value?.submissionCommand?.trim().toLowerCase() || null;
  const projectedCommand = actionSummary?.projectedCommand?.trim().toLowerCase() || null;
  const command = projectedCommand || planCommand || checkpointCommand;
  const checkpoint = latestExecutionCheckpoint.value;

  if (actionSummary?.commandKind === "stop_graph_run" && actionSummary.summary.trim()) {
    return {
      kind: "resume",
      label: "恢复",
      hint: actionSummary.summary.trim()
    };
  }

  if (command === "resume_graph_run_stream") {
    return {
      kind: "resume",
      label: "恢复",
      hint: actionSummary?.summary?.trim() || "检测到暂停中的运行；点击后会恢复该 run 并继续执行。"
    };
  }

  if (command === "continue_graph_run_stream") {
    return {
      kind: "continue",
      label: "继续",
      hint: actionSummary?.summary?.trim() || "检测到可继续的 graph run；点击后会接着当前运行推进。"
    };
  }

  if (
    actionSummary?.startReason === "replay_from_checkpoint" ||
    actionSummary?.startReason === "restart_from_checkpoint" ||
    actionSummary?.degraded ||
    checkpoint?.recoveryMode === "replay_required" ||
    checkpoint?.checkpointKind === "lifecycle_boundary" ||
    (command === "start_graph_run_stream" &&
      latestGraphRunSubmissionPlan.value?.source?.trim().toLowerCase() === "checkpoint")
  ) {
    return {
      kind: "restart",
      label: "重新开始",
      hint: actionSummary?.summary?.trim() || "当前恢复点只保留持久化事实；点击后会重新开始新的执行。"
    };
  }

  return {
    kind: "submit",
    label: "发送",
    hint: "输入消息后开始新一轮执行。"
  };
});

const primaryActionDisabled = computed(() => {
  if (sessionOperation.value) {
    return true;
  }
  if (isSubmitting.value) {
    return false;
  }
  const hasReadyAttachment = pendingAttachments.value.some((attachment) => attachment.status === "ok");
  return draftMessage.value.trim().length === 0 && !hasReadyAttachment;
});

const primaryActionTitle = computed(() =>
  isSubmitting.value ? "请求在安全边界停止当前运行。" : composerAction.value.hint
);

const attachNotice = ref<string | null>(null);

async function handleAttach() {
  attachNotice.value = null;
  try {
    const files = await pickFiles();
    if (!files.length) {
      return;
    }
    const { errors } = await runtimeStore.addPendingAttachments(files);
    if (errors.length) {
      attachNotice.value = errors.join("；");
    }
  } catch (error) {
    attachNotice.value = String(error);
  }
}

function removePendingAttachment(id: string) {
  runtimeStore.removePendingAttachment(id);
  attachNotice.value = null;
}

function formatAttachmentSize(sizeBytes: number): string {
  if (sizeBytes < 1024) {
    return `${sizeBytes} B`;
  }
  if (sizeBytes < 1024 * 1024) {
    return `${(sizeBytes / 1024).toFixed(1)} KB`;
  }
  return `${(sizeBytes / 1024 / 1024).toFixed(1)} MB`;
}

async function toggleMenu() {
  menuOpen.value = !menuOpen.value;
  reasoningMenuOpen.value = false;

  if (menuOpen.value) {
    if (modelSearchDebounceTimer) {
      clearTimeout(modelSearchDebounceTimer);
      modelSearchDebounceTimer = undefined;
    }
    modelSearchQuery.value = "";
    modelSearchDebounced.value = "";
    modelMenuOpen.value = true;
    floatingMenuStyle.value = {
      left: '0px',
      top: '0px',
      visibility: 'hidden'
    };
    await nextTick();
    updateFloatingMenuPosition();
    await nextTick();
    const modelItem = floatingMenuRef.value?.querySelector<HTMLElement>("[data-menu-item='model']");
    if (modelItem) {
      adjustModelSubmenuPosition(modelItem);
    }
    return;
  }

  modelMenuOpen.value = false;
}

function updateFloatingMenuPosition() {
  const trigger = modelTriggerRef.value;
  const floatingMenu = floatingMenuRef.value;
  if (!menuOpen.value || !trigger || !floatingMenu) {
    return;
  }

  const viewportPadding = 8;
  const triggerRect = trigger.getBoundingClientRect();
  const menuRect = floatingMenu.getBoundingClientRect();
  const maxLeft = Math.max(viewportPadding, window.innerWidth - menuRect.width - viewportPadding);
  const maxTop = Math.max(viewportPadding, window.innerHeight - menuRect.height - viewportPadding);
  const left = Math.min(
    Math.max(viewportPadding, triggerRect.right - menuRect.width),
    maxLeft
  );
  const top = Math.min(
    Math.max(viewportPadding, triggerRect.top - menuRect.height - viewportPadding),
    maxTop
  );
  const spaceRight = window.innerWidth - (left + menuRect.width) - viewportPadding;

  modelSubmenuDirection.value = spaceRight >= 248 ? 'right' : 'left';
  reasoningSubmenuDirection.value = spaceRight >= 168 ? 'right' : 'left';
  floatingMenuStyle.value = {
    left: `${left}px`,
    top: `${top}px`,
    visibility: 'visible'
  };
}

function adjustModelSubmenuPosition(button: HTMLElement) {
  const buttonRect = button.getBoundingClientRect();
  const viewportHeight = window.innerHeight;

  // 搜索框头部 + 内容区（最大高度封顶，超出滚动）
  const searchBoxHeight = 34;
  const captionHeight = 34;
  const itemHeight = 30;
  const padding = 14;
  const maxContentHeight = 264;
  const estimatedHeight = Math.min(
    searchBoxHeight + captionHeight + (providerStore.providers.length * itemHeight) + padding,
    searchBoxHeight + maxContentHeight + padding
  );

  const spaceBelow = viewportHeight - buttonRect.bottom - 8;

  if (estimatedHeight > spaceBelow) {
    const overflow = estimatedHeight - spaceBelow;
    modelSubmenuStyle.value = { top: `-${overflow}px` };
  } else {
    modelSubmenuStyle.value = { top: '0' };
  }
}

function adjustReasoningSubmenuPosition(button: HTMLElement) {
  const buttonRect = button.getBoundingClientRect();
  const viewportHeight = window.innerHeight;

  const captionHeight = 28;
  const itemHeight = 30;
  const padding = 12;
  const estimatedHeight = captionHeight + reasoningOptions.length * itemHeight + padding;

  const spaceBelow = viewportHeight - buttonRect.bottom - 8;

  if (estimatedHeight > spaceBelow) {
    const overflow = estimatedHeight - spaceBelow;
    reasoningSubmenuStyle.value = { top: `-${overflow}px` };
  } else {
    reasoningSubmenuStyle.value = { top: '0' };
  }
}

function onModelEnter(event: MouseEvent) {
  modelMenuOpen.value = true;
  reasoningMenuOpen.value = false;
  adjustModelSubmenuPosition(event.currentTarget as HTMLElement);
}

function onModelFocus(event: FocusEvent) {
  modelMenuOpen.value = true;
  reasoningMenuOpen.value = false;
  adjustModelSubmenuPosition(event.currentTarget as HTMLElement);
}

function onReasoningEnter(event: MouseEvent) {
  reasoningMenuOpen.value = true;
  modelMenuOpen.value = false;
  adjustReasoningSubmenuPosition(event.currentTarget as HTMLElement);
}

function onReasoningFocus(event: FocusEvent) {
  reasoningMenuOpen.value = true;
  modelMenuOpen.value = false;
  adjustReasoningSubmenuPosition(event.currentTarget as HTMLElement);
}

async function selectModel(providerId: string, modelId: string) {
  providerStore.selectModel(providerId, modelId);
  menuOpen.value = false;
  modelMenuOpen.value = false;
  await providerStore.saveRegistry();
}

function selectReasoningEffort(value: ProviderReasoningEffort | null) {
  providerStore.setCurrentReasoningEffort(value);
  reasoningMenuOpen.value = false;
}

function toggleReasoningVisibility() {
  emit("update:showReasoningContent", !props.showReasoningContent);
  modelMenuOpen.value = false;
  reasoningMenuOpen.value = false;
}

function handleShowReasoningSwitch(value: boolean) {
  emit("update:showReasoningContent", value);
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  const clickedTrigger = menuRef.value?.contains(target) ?? false;
  const clickedFloatingMenu = floatingMenuRef.value?.contains(target) ?? false;
  if (!clickedTrigger && !clickedFloatingMenu) {
    menuOpen.value = false;
    modelMenuOpen.value = false;
    reasoningMenuOpen.value = false;
  }
}

onMounted(() => {
  window.addEventListener("click", handleClickOutside);
  window.addEventListener("resize", updateFloatingMenuPosition);
  window.addEventListener("scroll", updateFloatingMenuPosition, true);
});

onBeforeUnmount(() => {
  if (modelSearchDebounceTimer) {
    clearTimeout(modelSearchDebounceTimer);
  }
  window.removeEventListener("click", handleClickOutside);
  window.removeEventListener("resize", updateFloatingMenuPosition);
  window.removeEventListener("scroll", updateFloatingMenuPosition, true);
});
</script>

<template>
  <div class="absolute bottom-0 left-0 right-0 z-30 px-4 py-3 sm:px-5 pointer-events-none">
    <div
      :ref="setComposerShellRef"
      class="relative mx-auto w-full max-w-[38.4rem] rounded-[0.6rem] bg-white/76 px-4 py-3 shadow-[0_-4px_20px_-2px_rgba(60,40,20,0.06)] backdrop-blur-[8px]"
      data-testid="workspace-composer-shell"
      style="pointer-events: auto;"
    >
      <textarea
        :value="draftMessage"
        :disabled="Boolean(sessionOperation)"
        data-testid="workspace-composer-input"
        class="min-h-[82px] w-full resize-none bg-transparent px-0 py-0 text-[13px] leading-[1.55] text-stone-800 outline-none placeholder:text-[12px] placeholder:font-normal placeholder:tracking-[0.01em] placeholder:text-stone-400/70"
        placeholder="输入消息，按 Enter 发送，Shift+Enter 换行。"
        @input="runtimeStore.setDraftMessage(($event.target as HTMLTextAreaElement).value)"
        @keydown="handleComposerKeydown"
      />

      <div v-if="pendingAttachments.length" class="mt-2 flex flex-wrap items-center gap-1.5">
        <div
          v-for="(attachment, index) in pendingAttachments"
          :key="attachment.id"
          class="flex max-w-full items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px]"
          :class="attachment.status === 'error'
            ? 'border-red-200 bg-red-50 text-red-600'
            : 'border-stone-200/80 bg-stone-100/80 text-stone-700'"
          :data-testid="`workspace-attachment-chip-${index}`"
          :title="attachment.errorDetail ?? attachment.path ?? ''"
        >
          <span class="truncate">{{ attachment.name }}</span>
          <span class="shrink-0 text-stone-400">{{ formatAttachmentSize(attachment.sizeBytes) }}</span>
          <button
            class="shrink-0 text-stone-400 transition-colors hover:text-stone-600"
            type="button"
            :data-testid="`workspace-attachment-remove-${index}`"
            aria-label="移除附件"
            @click="removePendingAttachment(attachment.id)"
          >
            <X class="h-3 w-3" />
          </button>
        </div>
      </div>
      <p
        v-if="attachNotice"
        class="mt-1.5 text-[11px] leading-4 text-amber-600"
        data-testid="workspace-attach-notice"
      >
        {{ attachNotice }}
      </p>

      <div class="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-t border-stone-200/70 pt-2.5">
        <div class="flex min-w-0 flex-wrap items-center gap-2">

          <TooltipRoot :delay-duration="300">
            <TooltipTrigger as-child>
              <span tabindex="0" class="inline-flex">
                <button
                  class="composer-trigger"
                  type="button"
                  :disabled="isSubmitting"
                  data-testid="workspace-attach-button"
                  @click="handleAttach"
                >
                  <Paperclip class="h-3 w-3" />
                  <span class="sr-only">添加附件</span>
                </button>
              </span>
            </TooltipTrigger>
            <TooltipPortal>
              <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                {{ isSubmitting ? "等待当前轮次结束后再添加附件" : "添加附件" }}
              </TooltipContent>
            </TooltipPortal>
          </TooltipRoot>
        </div>

        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <div ref="menuRef" class="relative">
            <TooltipRoot :delay-duration="300">
              <TooltipTrigger as-child>
                <span tabindex="0" class="inline-flex">
                  <button
                    ref="modelTriggerRef"
                    class="composer-trigger composer-model-trigger"
                    type="button"
                    data-testid="workspace-model-menu-trigger"
                    @click.stop="toggleMenu"
                  >
                    <span class="truncate">{{ currentModel ? modelDisplayName(currentModel) : "选择模型" }}</span>
                    <span v-if="currentModel" class="model-effort-hint">{{ reasoningSummary }}</span>
                    <ChevronDown
                      class="h-2.5 w-2.5 shrink-0 text-stone-400"
                      :class="{ 'rotate-180': menuOpen }"
                    />
                  </button>
                </span>
              </TooltipTrigger>
              <TooltipPortal>
                <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                  <div class="flex flex-col gap-0.5">
                    <span class="font-medium text-stone-700">模型和思考强度</span>
                    <span class="text-stone-500">{{ providerTooltipDetail }}</span>
                  </div>
                </TooltipContent>
              </TooltipPortal>
            </TooltipRoot>
          </div>

          <Teleport to="body">
            <div
              v-if="menuOpen"
              ref="floatingMenuRef"
              class="composer-menu-panel fixed z-[70] min-w-[14rem]"
              :style="floatingMenuStyle"
            >
              <div class="py-0.5">
                <div class="relative">
                  <button
                    class="composer-menu-item"
                    type="button"
                    data-menu-item="model"
                    @mouseenter="onModelEnter"
                    @focus="onModelFocus"
                  >
                    <span>模型</span>
                    <div class="flex items-center gap-2">
                      <span class="truncate text-[10px] text-stone-400">{{ currentModel ? modelDisplayName(currentModel) : "未选择" }}</span>
                      <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                    </div>
                  </button>
                  <div
                    v-if="modelMenuOpen"
                    :class="[
                      'composer-menu-panel absolute z-50 min-w-[15rem]',
                      modelSubmenuDirection === 'right' ? 'left-full ml-1' : 'right-full mr-1'
                    ]"
                    :style="modelSubmenuStyle"
                  >
                    <div class="composer-search-field mx-2 mt-1.5 mb-1">
                      <Search class="composer-search-icon" aria-hidden="true" />
                      <input
                        v-model="modelSearchQuery"
                        type="text"
                        placeholder="搜索模型名称或 ID"
                        data-testid="workspace-model-search"
                        class="composer-search-input"
                      />
                    </div>
                    <div class="composer-menu-scroll">
                      <template v-for="provider in filteredProviders" :key="provider.id">
                        <div class="composer-provider-caption">{{ provider.name }}</div>
                        <button
                          v-for="model in provider.models ?? []"
                          :key="model.id"
                          class="composer-menu-item"
                          type="button"
                          @click="selectModel(provider.id, model.id)"
                        >
                          <span class="truncate">{{ modelDisplayName(model) }}</span>
                          <Check
                            v-if="currentProvider?.id === provider.id && currentModel?.id === model.id"
                            class="h-3.5 w-3.5 shrink-0 text-stone-700"
                          />
                        </button>
                      </template>
                      <div
                        v-if="!filteredProviders.length"
                        class="px-3 py-2 text-[11px] leading-5 text-stone-400"
                      >
                        未找到匹配的模型
                      </div>
                    </div>
                  </div>
                </div>

                <div class="relative">
                  <button
                    class="composer-menu-item"
                    type="button"
                    data-menu-item="reasoning"
                    @mouseenter="onReasoningEnter"
                    @focus="onReasoningFocus"
                  >
                    <span>思考</span>
                    <div class="flex items-center gap-2">
                      <span class="truncate text-[10px] text-stone-400">{{ reasoningSummary }}</span>
                      <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                    </div>
                  </button>
                  <div
                    v-if="reasoningMenuOpen"
                    :class="[
                      'composer-menu-panel absolute z-50 min-w-[10rem]',
                      reasoningSubmenuDirection === 'right' ? 'left-full ml-1' : 'right-full mr-1'
                    ]"
                    :style="reasoningSubmenuStyle"
                  >
                    <div class="composer-menu-caption">思考强度</div>
                    <div class="composer-menu-divider"></div>
                    <template v-if="currentModelSupportsReasoning">
                      <button
                        v-for="option in reasoningOptions"
                        :key="option.label"
                        class="composer-menu-item"
                        type="button"
                        @click="selectReasoningEffort(option.value)"
                      >
                        <span>{{ option.label }}</span>
                        <Check
                          v-if="(providerStore.currentReasoningEffort ?? null) === option.value"
                          class="h-3.5 w-3.5 text-stone-700"
                        />
                      </button>
                    </template>
                    <div
                      v-else
                      class="composer-menu-note px-3 py-2 text-[11px] leading-5 text-stone-400"
                      data-testid="reasoning-unsupported-note"
                    >
                      当前模型不支持思考强度
                    </div>
                  </div>
                </div>

                <div
                  class="composer-menu-item"
                  role="menuitem"
                  tabindex="0"
                  data-testid="reasoning-visibility-toggle"
                  @click="toggleReasoningVisibility"
                  @keydown.enter="toggleReasoningVisibility"
                >
                  <div class="flex min-w-0 flex-col">
                    <span>显示思考</span>
                    <span class="composer-menu-item-hint">
                      {{ showReasoningContent ? "已开启" : "已关闭" }}
                    </span>
                  </div>
                  <span class="shrink-0" @click.stop>
                    <Switch
                      :model-value="showReasoningContent"
                      data-testid="reasoning-visibility-switch"
                      @update:model-value="handleShowReasoningSwitch"
                    />
                  </span>
                </div>
              </div>
            </div>
          </Teleport>

          <TooltipRoot :delay-duration="300">
            <TooltipTrigger as-child>
              <span tabindex="0" class="inline-flex">
                <button
                  class="checkpoint-icon-button !rounded-full"
                  type="button"
                  :disabled="!canUndoLastTurn"
                  data-testid="workspace-undo-button"
                  @click="handleUndoLastTurn"
                >
                  <Undo2 class="h-3.5 w-3.5" />
                  <span class="sr-only">{{ canUndoLastTurn ? '撤回' : '撤回不可用' }}</span>
                </button>
              </span>
            </TooltipTrigger>
            <TooltipPortal>
              <TooltipContent side="top" :side-offset="4" class="z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm">
                {{ canUndoLastTurn ? `${undoShortcutLabel} 撤回` : '没有可撤回的对话' }}
              </TooltipContent>
            </TooltipPortal>
          </TooltipRoot>

          <Button
            class="h-8 w-8 rounded-full p-0"
            size="sm"
            :disabled="primaryActionDisabled"
            :title="primaryActionTitle"
            :data-testid="isSubmitting ? 'workspace-stop-turn' : 'workspace-submit-action'"
            @click="handlePrimaryAction"
          >
            <Square v-if="isSubmitting" class="h-3.5 w-3.5 fill-current" />
            <ArrowUp v-if="!isSubmitting" class="h-3.5 w-3.5" />
            <span class="sr-only">{{ isSubmitting ? "停止" : composerAction.label }}</span>
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.composer-trigger {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.3rem;
  height: 1.35rem;
  min-width: 1.35rem;
  border: none;
  border-radius: 0.25rem;
  background: transparent;
  padding: 0 0.3rem;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  color: rgb(168 162 158);
  cursor: pointer;
  transition:
    color 0.15s ease,
    background-color 0.15s ease,
    transform 0.1s ease;
}

.composer-trigger:hover {
  color: rgb(87 83 78);
  background: rgba(0, 0, 0, 0.04);
}

.composer-trigger:active {
  transform: scale(0.9);
}

.composer-trigger:disabled {
  cursor: not-allowed;
  opacity: 0.3;
}

.composer-trigger:disabled:hover {
  background: transparent;
  color: rgb(168 162 158);
}

.composer-trigger:disabled:active {
  transform: none;
}

.composer-trigger:focus-visible {
  box-shadow: 0 0 0 2px rgba(231, 229, 228, 0.95);
}

.composer-trigger.composer-model-trigger {
  max-width: 12rem;
  padding: 0 0.45rem;
}

.composer-trigger.composer-model-trigger > .truncate {
  max-width: 8rem;
  color: rgb(87 83 78);
}

.composer-trigger.composer-model-trigger:hover > .truncate {
  color: rgb(60 56 52);
}

.model-effort-hint {
  font-size: 10px;
  font-weight: 400;
  line-height: 1;
  color: rgb(168 162 158);
}

.composer-search-field {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  border-bottom: 1px solid rgba(214, 211, 209, 0.85);
  color: rgb(168 162 158);
  transition:
    border-color 0.15s ease,
    box-shadow 0.15s ease,
    color 0.15s ease;
}

.composer-search-field:focus-within {
  border-color: rgba(139, 94, 52, 0.72);
  box-shadow: 0 4px 8px -7px rgba(139, 94, 52, 0.8);
  color: rgb(139 94 52);
}

.composer-search-icon {
  height: 0.8rem;
  width: 0.8rem;
  flex: none;
}

.composer-search-input {
  width: 100%;
  height: 1.6rem;
  border: none;
  background: transparent;
  padding: 0;
  font-size: 11px;
  line-height: 1;
  color: rgb(68 64 60);
  outline: none;
}

.composer-search-input::placeholder {
  color: rgb(168 162 158);
}

.composer-menu-scroll {
  max-height: 16.5rem;
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-width: none;
  -ms-overflow-style: none;
}

.composer-menu-scroll::-webkit-scrollbar {
  display: none;
}

.composer-provider-caption {
  padding: 0.6rem 0.9rem 0.3rem;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  color: rgb(168 162 158);
}

.composer-provider-caption:first-child {
  padding-top: 0.25rem;
}

.composer-provider-caption:not(:first-child) {
  margin-top: 0.2rem;
}

.composer-menu-panel {
  border: 1px solid rgba(231, 229, 228, 0.95);
  border-radius: 0.7rem;
  background: rgba(255, 255, 255, 0.98);
  padding: 0.35rem 0;
  color: rgb(87 83 78);
  box-shadow: 0 12px 32px rgba(41, 37, 36, 0.08);
  backdrop-filter: blur(14px);
}

.composer-menu-caption {
  padding: 0 0.9rem 0.35rem;
  font-size: 10px;
  line-height: 1;
  color: rgb(168 162 158);
}

.composer-menu-divider {
  margin: 0 0.55rem 0.2rem;
  border-top: 1px solid rgba(231, 229, 228, 0.92);
}

.composer-menu-item {
  display: flex;
  width: 100%;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.5rem 0.9rem;
  text-align: left;
  font-size: 12px;
  line-height: 1.2;
  color: rgb(87 83 78);
  transition: background-color 0.16s ease;
}

.composer-menu-item:hover {
  background: rgba(245, 245, 244, 0.9);
}

.composer-menu-item-hint {
  margin-top: 0.12rem;
  font-size: 10px;
  line-height: 1.2;
  color: rgb(168 162 158);
}

.checkpoint-icon-button {
  display: inline-flex;
  height: 1.35rem;
  width: 1.35rem;
  align-items: center;
  justify-content: center;
  border-radius: 0.25rem;
  border: none;
  background: transparent;
  color: rgb(168 162 158);
  cursor: pointer;
  transition:
    color 0.15s ease,
    background-color 0.15s ease,
    transform 0.1s ease;
}

.checkpoint-icon-button:hover {
  color: rgb(87 83 78);
  background: rgba(0, 0, 0, 0.04);
}

.checkpoint-icon-button:active {
  transform: scale(0.82);
  color: rgb(68 64 60);
}

.checkpoint-icon-button:disabled {
  cursor: not-allowed;
  opacity: 0.3;
}

.checkpoint-icon-button:disabled:hover {
  background: transparent;
  color: rgb(168 162 158);
}

.checkpoint-icon-button:disabled:active {
  transform: none;
}
</style>

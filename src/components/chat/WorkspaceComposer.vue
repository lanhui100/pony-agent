<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { ArrowUp, Check, ChevronDown, Square, Undo2 } from "lucide-vue-next";
import type { ProviderConfig, ProviderReasoningEffort } from "@/types/provider";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import Button from "@/components/ui/Button.vue";
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
  sessionOperation
} = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const providerMenuOpen = ref(false);
const hoveredProviderId = ref<string | null>(null);
const reasoningMenuOpen = ref(false);
const providerMenuRef = ref<HTMLElement | null>(null);
const modelSubmenuStyle = ref<Record<string, string>>({ top: '0' });
const reasoningMenuRef = ref<HTMLElement | null>(null);

const currentModelSupportsReasoning = computed(
  () => currentModel.value?.capabilities?.supportsReasoning ?? false
);

const providerLabel = computed(() => {
  const providerName = currentProvider.value?.name?.trim();
  const modelName = currentModel.value?.name?.trim();

  if (providerName && modelName) {
    return `${providerName}/${modelName}`;
  }

  if (providerName) {
    return providerName;
  }

  return "选择 provider/model";
});

const reasoningTriggerTitle = computed(() => {
  if (!currentModel.value) {
    return "当前未选择模型";
  }

  if (!currentModelSupportsReasoning.value) {
    return "当前模型不支持思考强度，可继续设置是否显示思考";
  }

  return "选择思考强度与思考显示方式";
});

const reasoningLabel = computed(() => {
  if (!currentModel.value) {
    return "思考 --";
  }

  if (!currentModelSupportsReasoning.value) {
    return "思考 不支持";
  }

  return providerStore.currentReasoningEffort
    ? `思考 ${reasoningEffortLabelZh(providerStore.currentReasoningEffort)}`
    : "思考 默认";
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

const primaryActionDisabled = computed(
  () => Boolean(sessionOperation.value) || (!isSubmitting.value && draftMessage.value.trim().length === 0)
);

const primaryActionTitle = computed(() =>
  isSubmitting.value ? "请求在安全边界停止当前运行。" : composerAction.value.hint
);

function toggleProviderMenu() {
  providerMenuOpen.value = !providerMenuOpen.value;

  if (providerMenuOpen.value) {
    const initialId = currentProvider.value?.id ?? providerStore.providers[0]?.id ?? null;
    hoveredProviderId.value = initialId;
    reasoningMenuOpen.value = false;

    // Adjust initial submenu position after DOM renders
    if (initialId) {
      nextTick(() => {
        const buttonEl = providerMenuRef.value?.querySelector<HTMLElement>(
          `[data-provider-id="${initialId}"]`
        );
        const provider = providerStore.providers.find((p) => p.id === initialId);
        if (provider && buttonEl) {
          adjustModelSubmenuPosition(provider, buttonEl);
        }
      });
    }
    return;
  }

  hoveredProviderId.value = null;
}

function adjustModelSubmenuPosition(provider: ProviderConfig, button: HTMLElement) {
  const buttonRect = button.getBoundingClientRect();
  const viewportHeight = window.innerHeight;

  // Estimate submenu height from its content
  const modelCount = provider.models?.length ?? 0;
  const captionHeight = 28;
  const itemHeight = 30;
  const padding = 12;
  const estimatedHeight = captionHeight + (modelCount > 0 ? modelCount * itemHeight + padding : itemHeight);

  const spaceBelow = viewportHeight - buttonRect.bottom - 8;

  if (estimatedHeight > spaceBelow) {
    const overflow = estimatedHeight - spaceBelow;
    modelSubmenuStyle.value = { top: `-${overflow}px` };
  } else {
    modelSubmenuStyle.value = { top: '0' };
  }
}

function onProviderEnter(provider: ProviderConfig, event: MouseEvent) {
  hoveredProviderId.value = provider.id;
  adjustModelSubmenuPosition(provider, event.currentTarget as HTMLElement);
}

function onProviderFocus(provider: ProviderConfig, event: FocusEvent) {
  hoveredProviderId.value = provider.id;
  adjustModelSubmenuPosition(provider, event.currentTarget as HTMLElement);
}

function toggleReasoningMenu() {
  if (!currentModel.value) {
    return;
  }

  reasoningMenuOpen.value = !reasoningMenuOpen.value;
  if (reasoningMenuOpen.value) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
  }
}

async function selectModel(providerId: string, modelId: string) {
  providerStore.selectModel(providerId, modelId);
  providerMenuOpen.value = false;
  hoveredProviderId.value = providerId;
  await providerStore.saveRegistry();
}

function selectReasoningEffort(value: ProviderReasoningEffort | null) {
  providerStore.setCurrentReasoningEffort(value);
  reasoningMenuOpen.value = false;
}

function toggleReasoningVisibility() {
  emit("update:showReasoningContent", !props.showReasoningContent);
  reasoningMenuOpen.value = false;
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  if (providerMenuRef.value && target && !providerMenuRef.value.contains(target)) {
    providerMenuOpen.value = false;
    hoveredProviderId.value = null;
  }

  if (reasoningMenuRef.value && target && !reasoningMenuRef.value.contains(target)) {
    reasoningMenuOpen.value = false;
  }
}

onMounted(() => {
  window.addEventListener("click", handleClickOutside);
});

onBeforeUnmount(() => {
  window.removeEventListener("click", handleClickOutside);
});
</script>

<template>
  <div class="absolute bottom-0 left-0 right-0 z-10 px-4 py-3 sm:px-5 pointer-events-none">
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

      <div class="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-t border-stone-200/70 pt-2.5">
        <div class="flex min-w-0 flex-wrap items-center gap-2">

          <div ref="providerMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              @click.stop="toggleProviderMenu"
            >
              <span class="truncate">{{ providerLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="providerMenuOpen"
              class="composer-menu-panel absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[14rem]"
            >
              <div class="composer-menu-caption">提供商</div>
              <div class="composer-menu-divider"></div>
              <div class="py-0.5">
                <div
                  v-for="provider in providerStore.providers"
                  :key="provider.id"
                  class="relative"
                >
                  <button
                    class="composer-menu-item"
                    type="button"
                    :data-provider-id="provider.id"
                    @mouseenter="onProviderEnter(provider, $event)"
                    @focus="onProviderFocus(provider, $event)"
                  >
                    <span class="truncate">{{ provider.name }}</span>
                    <div class="flex items-center gap-2">
                      <Check v-if="currentProvider?.id === provider.id" class="h-3.5 w-3.5 text-stone-700" />
                      <ChevronDown class="h-3.5 w-3.5 -rotate-90 text-stone-400" />
                    </div>
                  </button>
                  <div
                    v-if="hoveredProviderId === provider.id"
                    class="composer-menu-panel absolute left-full ml-1 min-w-[14rem]"
                    :style="modelSubmenuStyle"
                  >
                    <div class="composer-menu-caption">模型</div>
                    <div class="composer-menu-divider"></div>
                    <button
                      v-for="model in provider.models ?? []"
                      :key="model.id"
                      class="composer-menu-item"
                      type="button"
                      @click="selectModel(provider.id, model.id)"
                    >
                      <span class="truncate">{{ model.name }}</span>
                      <Check
                        v-if="currentProvider?.id === provider.id && currentModel?.id === model.id"
                        class="h-3.5 w-3.5 text-stone-700"
                      />
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div ref="reasoningMenuRef" class="relative">
            <button
              class="composer-select-trigger"
              type="button"
              :disabled="!currentModel"
              :title="reasoningTriggerTitle"
              @click.stop="toggleReasoningMenu"
            >
              <span class="truncate">{{ reasoningLabel }}</span>
              <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
            </button>

            <div
              v-if="currentModel && reasoningMenuOpen"
              class="composer-menu-panel absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[10rem]"
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
              <div class="composer-menu-divider"></div>
              <div class="composer-menu-caption">显示设置</div>
              <button
                class="composer-menu-item"
                data-testid="reasoning-visibility-toggle"
                type="button"
                @click="toggleReasoningVisibility"
              >
                <div class="flex min-w-0 flex-col">
                  <span>显示思考</span>
                  <span class="composer-menu-item-hint">
                    {{ showReasoningContent ? "已开启" : "已关闭" }}
                  </span>
                </div>
                <Check v-if="showReasoningContent" class="h-3.5 w-3.5 text-stone-700" />
              </button>
            </div>
          </div>

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
        </div>

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
</template>

<style scoped>
.composer-select-trigger {
  display: inline-flex;
  max-width: 12rem;
  min-height: 1.75rem;
  align-items: center;
  gap: 0.35rem;
  border: 1px solid rgba(214, 211, 209, 0.85);
  border-radius: 9999px;
  background: rgba(255, 255, 255, 0.7);
  padding: 0 0.7rem;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  color: rgb(87 83 78);
  outline: none;
  transition:
    border-color 0.18s ease,
    background-color 0.18s ease,
    color 0.18s ease;
}

.composer-select-trigger:hover {
  border-color: rgba(168, 162, 158, 0.7);
  background: rgba(250, 250, 249, 0.96);
}

.composer-select-trigger:disabled {
  opacity: 0.45;
}

.composer-select-trigger:focus-visible {
  box-shadow: 0 0 0 2px rgba(231, 229, 228, 0.95);
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

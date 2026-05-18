<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { marked } from "marked";
import { ArrowUp, Bot, Check, ChevronDown, Sparkles } from "lucide-vue-next";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import Button from "@/components/ui/Button.vue";
import InfoTip from "@/components/InfoTip.vue";

const runtimeStore = useRuntimeStore();
const providerStore = useProviderStore();

const { draftMessage, isSubmitting, messages } = storeToRefs(runtimeStore);
const { currentProvider, currentModel } = storeToRefs(providerStore);

const providerMenuOpen = ref(false);
const modelMenuOpen = ref(false);
const providerMenuRef = ref<HTMLElement | null>(null);
const modelMenuRef = ref<HTMLElement | null>(null);

const providerLabel = computed(() => currentProvider.value?.name || "提供商");
const modelLabel = computed(() => currentModel.value?.name || "模型");

function formatAssistantModelLabel(modelName?: string | null) {
  return modelName?.trim() || "";
}

function renderAssistantMarkdown(content: string) {
  return marked.parse(content, {
    breaks: true,
    gfm: true
  }) as string;
}

function handleComposerKeydown(event: KeyboardEvent) {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    if (!isSubmitting.value) {
      runtimeStore.submitTurn();
    }
  }
}

function toggleProviderMenu() {
  providerMenuOpen.value = !providerMenuOpen.value;
  if (providerMenuOpen.value) {
    modelMenuOpen.value = false;
  }
}

function toggleModelMenu() {
  if (!currentProvider.value) {
    return;
  }

  modelMenuOpen.value = !modelMenuOpen.value;
  if (modelMenuOpen.value) {
    providerMenuOpen.value = false;
  }
}

function selectProvider(providerId: string) {
  providerStore.selectProvider(providerId);
  providerMenuOpen.value = false;
  modelMenuOpen.value = false;
}

function selectModel(modelId: string) {
  if (!currentProvider.value) {
    return;
  }

  providerStore.selectModel(currentProvider.value.id, modelId);
  modelMenuOpen.value = false;
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;

  if (providerMenuRef.value && target && !providerMenuRef.value.contains(target)) {
    providerMenuOpen.value = false;
  }

  if (modelMenuRef.value && target && !modelMenuRef.value.contains(target)) {
    modelMenuOpen.value = false;
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
  <section class="flex min-h-[calc(100vh-12rem)] min-w-0 flex-col px-4 py-4 sm:px-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-2">
        <div class="flex items-center gap-2">
          <h2 class="text-lg font-semibold tracking-[-0.02em] text-stone-950">用户与 Agent 对话历史</h2>
          <InfoTip text="这里保留对话主视角。输入、历史消息和本轮模型选择都放在同一块，避免视线在多个面板之间来回跳转。" />
        </div>
        <p class="max-w-3xl text-sm leading-6 text-stone-500">
          输入会直接触发一次 `run_turn()`，并把返回结果写回主对话区。
        </p>
      </div>
      <div class="rounded-[0.45rem] bg-stone-900 px-3 py-1.5 text-xs font-medium text-amber-50">
        Rust core online
      </div>
    </div>

    <div class="mt-4 min-w-0 flex-1 overflow-visible rounded-[0.24rem] bg-[#fbf7f1] px-3 py-3 sm:px-4">
      <div class="flex h-full flex-col gap-3 overflow-visible">
        <div class="flex items-center justify-between gap-3 px-1 text-xs text-stone-500">
          <div class="flex items-center gap-2">
            <Sparkles class="h-3.5 w-3.5" />
            前端输入 -> Tauri 命令 -> Rust agent core -> provider -> UI 回显
          </div>
          <div>{{ isSubmitting ? "本轮执行中" : "等待输入" }}</div>
        </div>

        <div class="flex-1 space-y-3 overflow-auto px-1 pb-2">
          <article
            v-for="message in messages"
            :key="message.id"
            class="max-w-[86%] px-1 py-1 text-sm leading-6 sm:max-w-[78%]"
            :class="
              message.role === 'user'
                ? 'ml-auto rounded-[0.24rem] bg-[#766457] px-3 py-2.5 text-right text-amber-50'
                : 'mr-auto text-stone-700'
            "
          >
            <div
              class="mb-1.5 flex items-center gap-2 text-[10px] uppercase tracking-[0.15em]"
              :class="message.role === 'user' ? 'justify-end text-right text-amber-100/70' : 'text-stone-400'"
            >
              <Bot v-if="message.role === 'assistant'" class="h-3.5 w-3.5" />
              <span>{{ message.role === "user" ? "User" : "Agent" }}</span>
            </div>
            <div
              v-if="message.role === 'assistant'"
              class="assistant-markdown"
              :class="
                message.status === 'pending'
                  ? 'text-stone-400'
                  : message.status === 'error'
                    ? 'text-rose-700'
                    : 'text-stone-700'
              "
              v-html="renderAssistantMarkdown(message.content)"
            />
            <div v-else class="flex w-full justify-start">
              <div class="whitespace-pre-wrap text-left">{{ message.content }}</div>
            </div>
            <div
              v-if="message.role === 'assistant' && formatAssistantModelLabel(message.modelName)"
              class="mt-2 text-right text-[10px] leading-4 tracking-[0.02em] text-stone-400"
            >
              {{ formatAssistantModelLabel(message.modelName) }}
            </div>
          </article>
        </div>

        <div class="rounded-[0.2rem] bg-white px-3 py-3">
          <textarea
            :value="draftMessage"
            class="min-h-[84px] w-full resize-none bg-transparent px-0 py-0 text-sm leading-6 text-stone-900 outline-none placeholder:text-stone-400"
            placeholder="例如：解释为什么这一轮命中了 mock，而不是走真实模型。"
            @input="runtimeStore.setDraftMessage(($event.target as HTMLTextAreaElement).value)"
            @keydown="handleComposerKeydown"
          />

          <div class="mt-3 flex flex-wrap items-end justify-between gap-3 pt-2">
            <div class="flex flex-wrap items-end gap-3">
              <div ref="providerMenuRef" class="relative">
                <button
                  class="inline-flex min-w-[6.2rem] items-center gap-1 bg-transparent p-0 text-[10px] leading-4 text-stone-600 outline-none"
                  type="button"
                  @click.stop="toggleProviderMenu"
                >
                  <span>{{ providerLabel }}</span>
                  <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
                </button>

                <div
                  v-if="providerMenuOpen"
                  class="absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[12rem] rounded-[0.2rem] bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
                >
                  <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">提供商</div>
                  <div class="mx-2 border-t border-stone-200/90"></div>
                  <button
                    v-for="provider in providerStore.providers"
                    :key="provider.id"
                    class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-700 hover:bg-[#f8f4ed]"
                    type="button"
                    @click="selectProvider(provider.id)"
                  >
                    <span>{{ provider.name }}</span>
                    <Check v-if="currentProvider?.id === provider.id" class="h-3.5 w-3.5 text-amber-700" />
                  </button>
                </div>
              </div>

              <div v-if="currentProvider" ref="modelMenuRef" class="relative">
                <button
                  class="inline-flex min-w-[7.4rem] items-center gap-1 bg-transparent p-0 text-[10px] leading-4 text-stone-600 outline-none"
                  type="button"
                  @click.stop="toggleModelMenu"
                >
                  <span>{{ modelLabel }}</span>
                  <ChevronDown class="h-2.5 w-2.5 text-stone-400" />
                </button>

                <div
                  v-if="modelMenuOpen"
                  class="absolute bottom-[calc(100%+0.45rem)] left-0 z-20 min-w-[13rem] rounded-[0.2rem] bg-white py-1 text-[12px] text-stone-700 shadow-[0_10px_30px_rgba(61,47,34,0.08)]"
                >
                  <div class="px-3 py-1.5 text-[10px] uppercase tracking-[0.14em] text-stone-400">模型</div>
                  <div class="mx-2 border-t border-stone-200/90"></div>
                  <button
                    v-for="model in currentProvider.models"
                    :key="model.id"
                    class="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-stone-700 hover:bg-[#f8f4ed]"
                    type="button"
                    @click="selectModel(model.id)"
                  >
                    <span>{{ model.name }}</span>
                    <Check v-if="currentModel?.id === model.id" class="h-3.5 w-3.5 text-amber-700" />
                  </button>
                </div>
              </div>
            </div>

            <Button
              class="h-8 w-8 rounded-full p-0"
              size="sm"
              :disabled="isSubmitting"
              @click="runtimeStore.submitTurn()"
            >
              <ArrowUp class="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
:deep(.assistant-markdown p) {
  margin: 0 0 0.7rem;
}

:deep(.assistant-markdown p:last-child) {
  margin-bottom: 0;
}

:deep(.assistant-markdown ul),
:deep(.assistant-markdown ol) {
  margin: 0.45rem 0 0.7rem 1.2rem;
  padding: 0;
}

:deep(.assistant-markdown li + li) {
  margin-top: 0.2rem;
}

:deep(.assistant-markdown pre) {
  margin: 0.7rem 0;
  overflow-x: auto;
  border-radius: 0.2rem;
  background: #f3eee6;
  padding: 0.8rem 0.9rem;
  font-size: 0.82rem;
  line-height: 1.6;
}

:deep(.assistant-markdown code) {
  border-radius: 0.2rem;
  background: #f3eee6;
  padding: 0.08rem 0.28rem;
  font-size: 0.82em;
}

:deep(.assistant-markdown pre code) {
  background: transparent;
  padding: 0;
}

:deep(.assistant-markdown a) {
  color: #8b5e34;
  text-decoration: underline;
}

:deep(.assistant-markdown blockquote) {
  margin: 0.7rem 0;
  border-left: 2px solid #d8c7b2;
  padding-left: 0.8rem;
  color: #6b6257;
}
</style>

<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from "vue";
import { renderMarkdown } from "@/lib/markdown";

const props = defineProps<{
  content: string;
  toneClass?: string;
  wrapperClass?: string;
}>();

const renderedHtml = ref("");
const renderPending = ref(false);
let renderVersion = 0;
let renderTimerId: number | null = null;
let idleCallbackId: number | null = null;

function cancelScheduledRender() {
  if (typeof window === "undefined") {
    return;
  }

  if (renderTimerId != null) {
    window.clearTimeout(renderTimerId);
    renderTimerId = null;
  }

  const cancelIdleCallback = (window as Window & {
    cancelIdleCallback?: (handle: number) => void;
  }).cancelIdleCallback;

  if (idleCallbackId != null && typeof cancelIdleCallback === "function") {
    cancelIdleCallback(idleCallbackId);
    idleCallbackId = null;
  }
}

function scheduleMarkdownRender(content: string) {
  cancelScheduledRender();
  const version = ++renderVersion;
  renderedHtml.value = "";
  renderPending.value = Boolean(content.trim());

  if (!renderPending.value || typeof window === "undefined") {
    renderPending.value = false;
    return;
  }

  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    const requestIdleCallback = (window as Window & {
      requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
    }).requestIdleCallback;

    const runRender = () => {
      const html = renderMarkdown(content);
      if (version !== renderVersion) {
        return;
      }
      renderedHtml.value = html;
      renderPending.value = false;
    };

    if (typeof requestIdleCallback === "function") {
      idleCallbackId = requestIdleCallback(() => {
        idleCallbackId = null;
        runRender();
      }, { timeout: 1800 });
      return;
    }

    window.setTimeout(runRender, 0);
  }, 120);
}

watch(
  () => props.content,
  (content) => {
    scheduleMarkdownRender(content);
  },
  { immediate: true }
);

onBeforeUnmount(() => {
  cancelScheduledRender();
});
</script>

<template>
  <div
    v-if="renderedHtml"
    class="markdown-body"
    :class="[wrapperClass, toneClass]"
    v-html="renderedHtml"
  />
  <div
    v-else
    class="whitespace-pre-wrap"
    :class="[wrapperClass, toneClass]"
    :aria-busy="renderPending ? 'true' : undefined"
  >{{ content }}</div>
</template>

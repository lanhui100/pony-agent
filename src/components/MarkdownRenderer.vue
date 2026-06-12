<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from "vue";
import { renderMarkdown, endsWithNaturalBoundary } from "@/lib/markdown";

const props = defineProps<{
  content: string;
  toneClass?: string;
  wrapperClass?: string;
  streaming?: boolean;
}>();

const renderedHtml = ref("");
const unrenderedSuffix = ref("");
const renderPending = ref(false);
let renderVersion = 0;
let renderTimerId: number | null = null;
let idleCallbackId: number | null = null;
let lastRenderTime = 0;
let lastRenderedContentLength = 0;
let streamRenderScheduled = false;

const STREAMING_TIME_FALLBACK_MS = 1500;
const STREAMING_LENGTH_FALLBACK_CHARS = 800;

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

function shouldRenderNow(content: string): boolean {
  if (!props.streaming) {
    return true;
  }

  if (endsWithNaturalBoundary(content)) {
    return true;
  }

  const elapsed = Date.now() - lastRenderTime;
  if (elapsed >= STREAMING_TIME_FALLBACK_MS && content.length >= STREAMING_LENGTH_FALLBACK_CHARS) {
    return true;
  }

  return false;
}

function executeRender(version: number) {
  const html = renderMarkdown(props.content);
  if (version !== renderVersion) {
    return;
  }
  renderedHtml.value = html;
  renderPending.value = false;
  streamRenderScheduled = false;
  lastRenderTime = Date.now();
  lastRenderedContentLength = props.content.length;
  unrenderedSuffix.value = "";
}

function scheduleStreamingRender() {
  cancelScheduledRender();
  const version = ++renderVersion;
  streamRenderScheduled = true;
  renderPending.value = true;

  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    const requestIdleCallback = (window as Window & {
      requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
    }).requestIdleCallback;

    if (typeof requestIdleCallback === "function") {
      idleCallbackId = requestIdleCallback(() => {
        idleCallbackId = null;
        executeRender(version);
      }, { timeout: 1800 });
      return;
    }

    window.setTimeout(() => executeRender(version), 0);
  }, 60);
}

function scheduleNonStreamingRender() {
  cancelScheduledRender();
  const version = ++renderVersion;
  // 不清空 renderedHtml，保持上一次渲染的内容可见，直到新渲染完成
  renderPending.value = Boolean(props.content.trim());

  if (!renderPending.value || typeof window === "undefined") {
    renderPending.value = false;
    return;
  }

  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    const requestIdleCallback = (window as Window & {
      requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
    }).requestIdleCallback;

    if (typeof requestIdleCallback === "function") {
      idleCallbackId = requestIdleCallback(() => {
        idleCallbackId = null;
        executeRender(version);
      }, { timeout: 1800 });
      return;
    }

    window.setTimeout(() => executeRender(version), 0);
  }, 120);
}

function handleContentChange(content: string) {
  if (props.streaming) {
    if (streamRenderScheduled) {
      // A render is already pending — just update the suffix with new content
      unrenderedSuffix.value = content.slice(lastRenderedContentLength);
      return;
    }

    if (!shouldRenderNow(content)) {
      // No boundary or time fallback — show suffix as plain text
      unrenderedSuffix.value = content.slice(lastRenderedContentLength);
      return;
    }

    // Conditions met — schedule a render
    unrenderedSuffix.value = content.slice(lastRenderedContentLength);
    scheduleStreamingRender();
  } else {
    scheduleNonStreamingRender();
  }
}

watch(
  () => props.content,
  (content) => {
    handleContentChange(content);
  },
  { immediate: true }
);

watch(
  () => props.streaming,
  (isStreaming, wasStreaming) => {
    if (wasStreaming && !isStreaming) {
      // Streaming ended — force a final full render
      streamRenderScheduled = false;
      lastRenderedContentLength = 0;
      unrenderedSuffix.value = "";
      scheduleNonStreamingRender();
    }
  }
);

onBeforeUnmount(() => {
  cancelScheduledRender();
});
</script>

<template>
  <template v-if="streaming">
    <div v-if="renderedHtml || unrenderedSuffix" :class="[wrapperClass, toneClass]">
      <div v-if="renderedHtml" class="markdown-body" v-html="renderedHtml" />
      <span v-if="unrenderedSuffix" class="streaming-unrendered-suffix whitespace-pre-wrap">{{ unrenderedSuffix }}</span>
    </div>
    <div v-else class="whitespace-pre-wrap" :class="[wrapperClass, toneClass]">{{ content }}</div>
  </template>
  <template v-else>
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
</template>
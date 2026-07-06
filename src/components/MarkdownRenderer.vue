<script setup lang="ts">
import { onBeforeUnmount, ref, useAttrs, watch } from "vue";
import { renderMarkdown, renderPartialMarkdown, endsWithNaturalBoundary } from "@/lib/markdown";

defineOptions({
  inheritAttrs: false
});

const props = defineProps<{
  content: string;
  toneClass?: string;
  wrapperClass?: string;
  streaming?: boolean;
}>();

const emit = defineEmits<{
  (event: "render-complete", payload: { contentLength: number; streaming: boolean }): void;
}>();

const attrs = useAttrs();

const renderedHtml = ref("");
const unrenderedSuffix = ref("");
const renderPending = ref(false);
let renderVersion = 0;
let renderTimerId: number | null = null;
let idleCallbackId: number | null = null;
let lastRenderTime = 0;
let lastRenderedContentLength = 0;
let lastRenderedFullContent = "";
let streamRenderScheduled = false;

const STREAMING_RENDER_DEBOUNCE_MS = 90;
const STREAMING_RENDER_INTERVAL_MS = 140;
const STREAMING_FORCE_RENDER_CHARS = 48;
const STREAMING_TIME_FALLBACK_MS = 900;
const STREAMING_LENGTH_FALLBACK_CHARS = 320;

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

  if (!lastRenderedContentLength && content.length >= STREAMING_FORCE_RENDER_CHARS) {
    return true;
  }

  if (content.length - lastRenderedContentLength >= STREAMING_FORCE_RENDER_CHARS) {
    return true;
  }

  if (endsWithNaturalBoundary(content)) {
    return true;
  }

  const elapsed = Date.now() - lastRenderTime;
  if (elapsed >= STREAMING_RENDER_INTERVAL_MS && content.length > lastRenderedContentLength) {
    return true;
  }

  if (elapsed >= STREAMING_TIME_FALLBACK_MS && content.length >= STREAMING_LENGTH_FALLBACK_CHARS) {
    return true;
  }

  return false;
}

async function executeRender(version: number) {
  try {
    const renderFn = props.streaming ? renderPartialMarkdown : renderMarkdown;
    const html = await renderFn(props.content);
    if (version !== renderVersion) return;
    renderedHtml.value = html;
    lastRenderedFullContent = props.content;
    renderPending.value = false;
    streamRenderScheduled = false;
    lastRenderTime = Date.now();
    lastRenderedContentLength = props.content.length;
    unrenderedSuffix.value = "";
    emit("render-complete", {
      contentLength: props.content.length,
      streaming: Boolean(props.streaming)
    });
  } catch (err) {
    console.error("[MarkdownRenderer] executeRender failed:", err);
    if (version !== renderVersion) return;
    renderedHtml.value = "";
    lastRenderedFullContent = props.content;
    renderPending.value = false;
    streamRenderScheduled = false;
    lastRenderTime = Date.now();
    lastRenderedContentLength = props.content.length;
    unrenderedSuffix.value = props.content;
  }
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
  }, STREAMING_RENDER_DEBOUNCE_MS);
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
    if (content.length < lastRenderedContentLength) {
      // Batch flush: content was moved to the fade span.
      // Clear renderedHtml to prevent duplication with the separate fade <span>.
      renderedHtml.value = "";
      lastRenderedContentLength = 0;
    }

    if (streamRenderScheduled) {
      unrenderedSuffix.value = content.slice(lastRenderedContentLength);
      return;
    }

    if (!shouldRenderNow(content)) {
      unrenderedSuffix.value = content.slice(lastRenderedContentLength);
      return;
    }

    unrenderedSuffix.value = content.slice(lastRenderedContentLength);
    scheduleStreamingRender();
  } else if (lastRenderedFullContent !== content) {
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
      // Streaming ended
      streamRenderScheduled = false;
      unrenderedSuffix.value = "";

      // Otherwise force a final full render
      lastRenderedContentLength = 0;
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
    <div v-if="renderedHtml || unrenderedSuffix" v-bind="attrs" :class="[wrapperClass, toneClass]">
      <div v-if="renderedHtml" class="markdown-body" v-html="renderedHtml" />
      <span v-if="unrenderedSuffix" class="streaming-unrendered-suffix whitespace-pre-wrap">{{ unrenderedSuffix }}</span>
    </div>
    <div v-else v-bind="attrs" class="whitespace-pre-wrap" :class="[wrapperClass, toneClass]">{{ content }}</div>
  </template>
  <template v-else>
    <div
      v-if="renderedHtml"
      v-bind="attrs"
      class="markdown-body"
      :class="[wrapperClass, toneClass]"
      v-html="renderedHtml"
    />
    <div
      v-else
      v-bind="attrs"
      class="whitespace-pre-wrap"
      :class="[wrapperClass, toneClass]"
      :aria-busy="renderPending ? 'true' : undefined"
    >{{ content }}</div>
  </template>
</template>

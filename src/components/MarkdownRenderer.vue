<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useAttrs, watch } from "vue";
import {
  endsWithNaturalBoundary,
  isSimpleTextContent,
  markdownRenderEpoch,
  renderMarkdown,
  renderPartialMarkdown
} from "@/lib/markdown";

defineOptions({
  inheritAttrs: false
});

const props = defineProps<{
  content: string;
  toneClass?: string;
  wrapperClass?: string;
  streaming?: boolean;
  preferPlainTextStreaming?: boolean;
  forceMarkdownStreaming?: boolean;
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
const PLAINTEXT_RENDER_COMPLETE_MIN_INTERVAL_MS = 120;
const PLAINTEXT_RENDER_COMPLETE_MIN_CHARS = 24;
let lastPlainTextRenderCompleteAt = 0;

/** 流式渲染时是否走纯文本快路径（跳过 markdown 解析） */
const plainTextMode = computed(() => {
  if (!props.streaming) return false;
  if (props.forceMarkdownStreaming) return false;
  if (props.preferPlainTextStreaming) return true;
  return isSimpleTextContent(props.content);
});

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

  if (props.forceMarkdownStreaming && content.length > lastRenderedContentLength) {
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
  if (streamRenderScheduled) {
    return;
  }

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
    // 纯文本快路径：跳过 markdown 解析，避免 v-html 重排
    if (plainTextMode.value) {
      const previousContent = lastRenderedFullContent;
      if (renderedHtml.value || unrenderedSuffix.value || lastRenderedFullContent !== content) {
        renderedHtml.value = "";
        unrenderedSuffix.value = "";
        lastRenderedContentLength = 0;
        lastRenderedFullContent = content;
        const now = Date.now();
        const contentDelta = content.length - previousContent.length;
        if (
          previousContent.length > 0
          && contentDelta > 0
          && (
            contentDelta >= PLAINTEXT_RENDER_COMPLETE_MIN_CHARS
            || endsWithNaturalBoundary(content)
            || now - lastPlainTextRenderCompleteAt >= PLAINTEXT_RENDER_COMPLETE_MIN_INTERVAL_MS
          )
        ) {
          lastPlainTextRenderCompleteAt = now;
          emit("render-complete", {
            contentLength: content.length,
            streaming: true
          });
        }
      }
      return;
    }

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
      lastPlainTextRenderCompleteAt = 0;

      // Otherwise force a final full render
      lastRenderedContentLength = 0;
      scheduleNonStreamingRender();
    }
  }
);

watch(markdownRenderEpoch, () => {
  cancelScheduledRender();
  renderPending.value = false;
  streamRenderScheduled = false;
  lastRenderedContentLength = 0;
  lastRenderedFullContent = "";
  unrenderedSuffix.value = "";

  if (!props.content.trim() || (props.streaming && plainTextMode.value)) {
    return;
  }

  if (props.streaming) {
    scheduleStreamingRender();
    return;
  }

  scheduleNonStreamingRender();
});

onBeforeUnmount(() => {
  cancelScheduledRender();
});
</script>

<template>
  <!-- 单一稳定根元素：无论 streaming 状态如何变化，不会替换 DOM 子树 -->
  <div v-bind="attrs" :class="[wrapperClass, toneClass]">
    <!-- 流式 + 纯文本快路径：直接显示文本，不加 v-html -->
    <div v-if="streaming && plainTextMode && content" class="markdown-body whitespace-pre-wrap">{{ content }}</div>
    <!-- 流式 + 正常 markdown 渲染路径 -->
    <template v-else-if="streaming">
      <div v-if="renderedHtml" class="markdown-body" v-html="renderedHtml" />
      <div v-if="forceMarkdownStreaming && unrenderedSuffix" class="mr-streaming-raw whitespace-pre-wrap">
        <span v-for="(char, i) in unrenderedSuffix.split('')" :key="i" class="mr-streaming-char">{{ char }}</span>
      </div>
      <template v-if="!forceMarkdownStreaming">
        <span v-if="unrenderedSuffix" class="streaming-unrendered-suffix whitespace-pre-wrap">{{ unrenderedSuffix }}</span>
        <div v-else-if="content" class="whitespace-pre-wrap">{{ content }}</div>
      </template>
      <div v-else-if="content && !renderedHtml && !unrenderedSuffix" class="mr-streaming-raw whitespace-pre-wrap">
        <span v-for="(char, i) in content.split('')" :key="i" class="mr-streaming-char">{{ char }}</span>
      </div>
    </template>
    <!-- 最终渲染（非流式） -->
    <template v-else>
      <div v-if="renderedHtml" class="markdown-body" v-html="renderedHtml" />
      <div v-else-if="content" class="whitespace-pre-wrap" :aria-busy="renderPending ? 'true' : undefined">{{ content }}</div>
    </template>
  </div>
</template>

<style scoped>
.mr-streaming-raw {
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.7;
  color: #3d342d;
}

.mr-streaming-char {
  display: inline;
  white-space: pre-wrap;
  animation-name: mr-stream-char-fade-in;
  animation-duration: 50ms;
  animation-timing-function: ease-out;
  animation-fill-mode: both;
}

@keyframes mr-stream-char-fade-in {
  from {
    opacity: 0;
    transform: translateY(-0.04em);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .mr-streaming-char {
    animation: none !important;
    opacity: 1 !important;
    transform: none !important;
  }
}
</style>

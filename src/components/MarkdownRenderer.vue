<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useAttrs, watch } from "vue";
import {
  endsWithNaturalBoundary,
  isAllowedExternalHref,
  isSimpleTextContent,
  markdownRenderEpoch,
  renderMarkdown,
  renderPartialMarkdown,
  splitMarkdownBlocks
} from "@/lib/markdown";
import { openExternalUrl } from "@/lib/open-external-url";

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
const blockedLinkNotice = ref("");
let blockedLinkNoticeTimerId: number | null = null;
let renderVersion = 0;
let renderTimerId: number | null = null;
let idleCallbackId: number | null = null;
let lastRenderedContentLength = 0;
let lastRenderedFullContent = "";
let streamRenderScheduled = false;

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

/**
 * F4b：markdown 内的链接点击统一拦截。
 *
 * 消毒层（`src/lib/markdown.ts`）已剥离非白名单 href，此处是第二道闸：
 * - 站内锚点（`#…`）：交给浏览器默认行为（无外部网络动作）；
 * - 白名单 https：`preventDefault` 后走 `openExternalUrl`（Rust 侧再强制校验）；
 * - 其余（含已被剥离 href 的空值、相对路径、非白名单 host、http/ftp）：阻断 + 提示。
 */
function onMarkdownClick(event: MouseEvent) {
  const eventTarget = event.target as Element | null;
  const anchor = eventTarget?.closest?.("a") ?? null;

  if (!anchor) {
    return;
  }

  const href = (anchor.getAttribute("href") ?? "").trim();

  if (href.startsWith("#")) {
    return;
  }

  event.preventDefault();

  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(href);

  if (hasScheme && isAllowedExternalHref(href)) {
    void openExternalUrl(href);
    return;
  }

  showBlockedLinkNotice();
}

function showBlockedLinkNotice() {
  blockedLinkNotice.value = "已阻止打开未经安全校验的链接";
  blockedLinkNoticeTimerId = window.setTimeout(() => {
    blockedLinkNotice.value = "";
    blockedLinkNoticeTimerId = null;
  }, 3000);
}

function shouldRenderNow(content: string): boolean {
  if (!props.streaming) {
    return true;
  }

  // 流式模式下只要有新内容到达即可参与刷新，让增量块和尾部平滑过渡
  if (content.length > lastRenderedContentLength) {
    return true;
  }

  if (endsWithNaturalBoundary(content)) {
    return true;
  }

  return false;
}

async function executeRender(version: number) {
  try {
    let html: string;
    if (props.streaming) {
      // 块级增量优化与代码块隔离
      const { committed, tail } = splitMarkdownBlocks(props.content);
      if (committed.length > 0) {
        const committedHtmlList = await Promise.all(committed.map((b) => renderMarkdown(b)));
        const tailHtml = tail.trim() ? await renderPartialMarkdown(tail) : "";
        html = [...committedHtmlList, tailHtml].filter(Boolean).join("\n");
      } else {
        html = await renderPartialMarkdown(props.content);
      }
    } else {
      html = await renderMarkdown(props.content);
    }
    if (version !== renderVersion) return;
    renderedHtml.value = html;
    lastRenderedFullContent = props.content;
    renderPending.value = false;
    streamRenderScheduled = false;
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

  // 流式增量更新采用紧凑微节流，优先通过微任务执行；不再套用长达 1800ms 的 rIC，避免在频繁推流时卡死
  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    executeRender(version);
  }, 16);
}

function scheduleNonStreamingRender(immediate = false) {
  cancelScheduledRender();
  const version = ++renderVersion;
  // 不清空 renderedHtml，保持上一次渲染的内容可见，直到新渲染完成
  renderPending.value = Boolean(props.content.trim());

  if (!renderPending.value || typeof window === "undefined") {
    renderPending.value = false;
    return;
  }

  if (immediate) {
    executeRender(version);
    return;
  }

  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    executeRender(version);
  }, 16);
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

      // Otherwise force a final full render immediately
      lastRenderedContentLength = 0;
      scheduleNonStreamingRender(true);
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
  if (blockedLinkNoticeTimerId != null) {
    window.clearTimeout(blockedLinkNoticeTimerId);
    blockedLinkNoticeTimerId = null;
  }
});
</script>

<template>
  <!-- 单一稳定根元素：无论 streaming 状态如何变化，不会替换 DOM 子树 -->
  <div v-bind="attrs" :class="[wrapperClass, toneClass]" @click="onMarkdownClick">
    <div v-if="blockedLinkNotice" class="mr-blocked-link-notice" role="status" aria-live="polite">
      {{ blockedLinkNotice }}
    </div>
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
.mr-blocked-link-notice {
  margin: 4px 0;
  padding: 2px 8px;
  border-radius: 6px;
  background: rgba(180, 120, 40, 0.1);
  color: #8a5a1f;
  font-size: 12px;
  line-height: 1.6;
}

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

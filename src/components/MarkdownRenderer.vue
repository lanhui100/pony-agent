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
const renderPending = ref(false);
const blockedLinkNotice = ref("");
let blockedLinkNoticeTimerId: number | null = null;
let renderVersion = 0;
let renderTimerId: number | null = null;
let idleCallbackId: number | null = null;
/**
 * Wave-59 F3 / RC3：上一次成功渲染所消费的**输入快照**。
 *
 * 必须是 `executeRender` 入口处取到的 `props.content`，而不是 await 之后的 `props.content`；
 * 否则 await 期间到达的字符会被记成「已渲染」却没有进入 HTML，下一次渲染才补齐
 * ⇒ 文字出现-消失-再出现（RC3）。
 */
let lastRenderedSource = "";
/**
 * Wave-59 F3：在飞渲染的版本号（null ⇒ 无在飞渲染）。
 * 与 `renderDirty` 一起构成 stale-while-render 的补排闭环：渲染在飞期间到达的新内容
 * 不再叠加新排程（不呈现尾巴），但**必须**被记为 dirty，落地后立刻补排一次，
 * 否则被吞掉的增量会永远停在上一次快照（在线流式内容会卡住）。
 */
let renderInFlightVersion: number | null = null;
let renderDirty = false;

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

/** 纯文本快路径的唯一渲染形态：转义后写入同一承载节点（不新增尾巴节点，不做逐字符动画）。 */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

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

/**
 * Wave-59 F3 / D-59-1「stale-while-render」：只呈现上一次渲染完成的 HTML。
 *
 * 未渲染的尾巴不再以任何独立节点呈现（RC1 的块级新行位移、RC2 的逐字符重播随之消失），
 * 代价是可见文本滞后 ≤ 1 个渲染周期（16ms + parse）。
 */
async function executeRender(version: number) {
  // RC3：输入快照在 await 之前取，保证「写入 renderedHtml 的 source」与「标记已渲染的 source」同一。
  const source = props.content;
  try {
    let html: string;
    if (props.streaming) {
      // 块级增量优化与代码块隔离
      const { committed, tail } = splitMarkdownBlocks(source);
      if (committed.length > 0) {
        const committedHtmlList = await Promise.all(committed.map((block) => renderMarkdown(block)));
        const tailHtml = tail.trim() ? await renderPartialMarkdown(tail) : "";
        html = [...committedHtmlList, tailHtml].filter(Boolean).join("\n");
      } else {
        html = await renderPartialMarkdown(source);
      }
    } else {
      html = await renderMarkdown(source);
    }
    if (version !== renderVersion) return;
    renderedHtml.value = html;
    lastRenderedSource = source;
    finishRender(version);
    emit("render-complete", {
      contentLength: source.length,
      streaming: Boolean(props.streaming)
    });
  } catch (err) {
    console.error("[MarkdownRenderer] executeRender failed:", err);
    if (version !== renderVersion) return;
    // 失败也保持正文可见：降级为转义纯文本，不得留下空白承载节点（F3-5 无白屏帧）。
    renderedHtml.value = escapeHtml(source);
    lastRenderedSource = source;
    finishRender(version);
    emit("render-complete", {
      contentLength: source.length,
      streaming: Boolean(props.streaming)
    });
  }
}

/**
 * 收尾一次渲染：清在飞标记，并在「在飞期间又来了新内容」时立即补排一次。
 * 只有当前生效版本（version === renderVersion）才会走到这里，旧版本由版本守卫提前 return。
 */
function finishRender(version: number) {
  if (renderInFlightVersion === version) {
    renderInFlightVersion = null;
  }
  renderPending.value = false;
  if (!renderDirty || renderInFlightVersion != null) {
    return;
  }
  renderDirty = false;
  if (props.streaming) {
    scheduleStreamingRender();
    return;
  }
  scheduleNonStreamingRender();
}

function scheduleStreamingRender() {
  // 在飞期间的增量不叠加排程（stale-while-render：不呈现未渲染尾巴），只记 dirty 待补排。
  if (renderInFlightVersion != null) {
    renderDirty = true;
    return;
  }

  cancelScheduledRender();
  const version = ++renderVersion;
  renderInFlightVersion = version;
  renderPending.value = true;

  // F3-5②：承载节点尚无 HTML 时立即渲染（微任务内落地，避免「content > 0 但正文为空」的白屏帧）；
  // 之后在已渲染内容之上做 16ms 紧凑节流增量（stale-while-render）。
  if (!renderedHtml.value) {
    void executeRender(version);
    return;
  }

  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    void executeRender(version);
  }, 16);
}

function scheduleNonStreamingRender(immediate = false) {
  cancelScheduledRender();
  const version = ++renderVersion;
  renderDirty = false;
  // 不清空 renderedHtml，保持上一次渲染的内容可见，直到新渲染完成
  renderPending.value = Boolean(props.content.trim());

  if (!renderPending.value || typeof window === "undefined") {
    // A-59-1：本分支不会安排任何渲染 ⇒ 不得留下在飞标记。
    // 否则该实例后续的流式更新会被 `renderDirty` 判定为「等前一次渲染收尾」而永久吞掉
    // （stale-while-render 闭环断裂：内容停在空白快照上）。
    renderPending.value = false;
    renderInFlightVersion = null;
    return;
  }

  // 只有真正排程/发起的渲染才登记在飞版本号（由 finishRender 或版本守卫收尾）。
  renderInFlightVersion = version;

  // F3-3：完成帧首次落地不排队，避免「先清空再重绘」；已渲染内容仍保留至新 HTML 落地。
  if (immediate || !renderedHtml.value) {
    void executeRender(version);
    return;
  }

  renderTimerId = window.setTimeout(() => {
    renderTimerId = null;
    void executeRender(version);
  }, 16);
}

function handleContentChange(content: string) {
  if (props.streaming) {
    // 纯文本快路径：无 markdown 语法 ⇒ 同步转义写入承载节点（不解析、不排程、不产生尾巴节点）
    if (plainTextMode.value) {
      const previousContent = lastRenderedSource;
      renderedHtml.value = escapeHtml(content);
      lastRenderedSource = content;
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
      return;
    }

    // markdown 流式：排程器内部对「在飞渲染」去重并记 dirty（stale-while-render：不呈现未渲染尾巴）
    scheduleStreamingRender();
    return;
  }

  if (lastRenderedSource !== content) {
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
      // Streaming ended：保留当前已渲染 HTML 直到完整渲染落地（不重建节点、不清空正文）
      renderDirty = false;
      lastPlainTextRenderCompleteAt = 0;
      scheduleNonStreamingRender(true);
    }
  }
);

watch(markdownRenderEpoch, () => {
  cancelScheduledRender();
  renderPending.value = false;
  renderInFlightVersion = null;
  renderDirty = false;
  lastRenderedSource = "";

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
  <!--
    Wave-59 F3-2/F3-3：单一稳定根 + 唯一常驻承载节点。
    - `.markdown-body` 无 v-if，从首帧起就存在（F3-2a），内容增长与 streaming→done 只替换
      其 `v-html` 内容，不重建 DOM 节点（F3-2b/F3-3a 的节点身份断言）；
    - 未渲染尾巴不以任何独立节点呈现（F3-1），也不再有逐字符 span（D-59-2）。
  -->
  <div v-bind="attrs" :class="[wrapperClass, toneClass]" @click="onMarkdownClick">
    <div v-if="blockedLinkNotice" class="mr-blocked-link-notice" role="status" aria-live="polite">
      {{ blockedLinkNotice }}
    </div>
    <div
      class="markdown-body"
      :class="plainTextMode ? 'whitespace-pre-wrap' : undefined"
      :data-streaming="streaming ? 'true' : undefined"
      :aria-busy="renderPending ? 'true' : undefined"
      v-html="renderedHtml"
    />
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
</style>

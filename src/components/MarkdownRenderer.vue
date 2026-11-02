<script lang="ts">
/**
 * wave-pds-3 v8：**尾巴能力的共享纯函数**（无状态、不读布局、可跨组件复用）。
 *
 * 为什么放在普通 `<script>`（而非 `<script setup>`）：契约 v8 要求「纯文本快路径」也发尾巴，
 * 而该路径由 `WorkspaceTurnItem` 自建 `.markdown-body` 承载、**不经过** MarkdownRenderer。
 * 两条路径必须共用同一套「开关 / 降级 / 资格门 / 单行判定」，否则会出现"同一特性两种语义"。
 * 本波写域只含既有文件（不新建 `src/lib/*`），故以 SFC 普通脚本块导出，供同目录组件按名导入
 * （`WorkspaceTurnItem` 本来就已 import 本组件，不引入新的耦合方向）。
 */
import { renderPartialMarkdown as renderTailCandidateMarkdown } from "@/lib/markdown";

export const STREAM_TAIL_SWITCH_STORAGE_KEY = "pony-agent.stream-render.tail-reveal";
export const STREAM_TAIL_REDUCE_MOTION_ATTR = "data-pds-reduce-motion";
/** 单行判定的宽度安全余量（px）：留出掩码渐隐区，避免压线换行。 */
export const STREAM_TAIL_LINE_SAFETY_PX = 6;

/** 契约 §2：开关 **mount-only** 读取一次并缓存；缺省 = on，只有精确 `"off"` 才关闭。 */
export function readStreamTailSwitchPreference(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(STREAM_TAIL_SWITCH_STORAGE_KEY) !== "off";
  } catch {
    return false;
  }
}

/** 契约 §2 / §3.5：系统降级或祖先 `data-pds-reduce-motion="true"` ⇒ 强制关闭尾巴。 */
export function isStreamTailReducedMotionActive(query: MediaQueryList | null | undefined): boolean {
  if (query?.matches) return true;
  if (typeof document === "undefined") return false;
  return document.documentElement.getAttribute(STREAM_TAIL_REDUCE_MOTION_ATTR) === "true";
}

/**
 * 契约 v9 §1：**尾巴资格门 = 渲染结果自证（render-then-verify）**。
 *
 * v7 用 8 条行正则近似「纯段落」，被实测证明**不是充要条件**（tab 缩进、`#` 标题、`***`/`___`、
 * 单 `-` setext 全漏网，且漏网后果正是 v7 宣称消除的形态）。v9 改为：先用**同一渲染器**
 * （`renderPartialMarkdown`）渲染候选尾巴，再看结果里有没有块级元素 —— 判据与渲染器同源，
 * 渲染器升级判据自动跟随，不存在"正则没想到的写法"。
 */
const NON_PARAGRAPH_BLOCK_TAG_PATTERN =
  /<(h[1-6]|ul|ol|li|pre|table|thead|tbody|tr|td|th|blockquote|hr|div|section|figure|dl|dt|dd)\b/i;
/** 顶层 `<p>` 计数（渲染结果不会出现嵌套 `<p>`，故直接数即可）。 */
const PARAGRAPH_TAG_PATTERN = /<p(\s|>)/gi;

export interface StreamTailCandidate {
  /** 资格：渲染结果里没有块级元素，且顶层 `<p>` 至多一个。 */
  eligible: boolean;
  /** 已渲染好的尾巴 HTML（资格成立时可直接复用，避免同一帧渲染两次）。 */
  html: string;
}

/**
 * 判定候选尾巴文本是否「纯段落」并可发尾巴。
 * 返回 `html` 供调用方复用 —— 资格判定与真正渲染共用**同一次** `renderPartialMarkdown`。
 */
export async function checkStreamTailCandidate(tailSource: string): Promise<StreamTailCandidate> {
  const html = await renderTailCandidateMarkdown(tailSource);
  const hasBlockLevelTag = NON_PARAGRAPH_BLOCK_TAG_PATTERN.test(html);
  const topLevelParagraphs = (html.match(PARAGRAPH_TAG_PATTERN) ?? []).length;
  return { eligible: !hasBlockLevelTag && topLevelParagraphs <= 1, html };
}

/**
 * 契约 v5 R-A4：**遮罩只在尾巴实际单行时启用**。
 *
 * 判定 = 渲染文本不含换行 且（能测宽时）宽度放得进一行。宽度由调用方用 ResizeObserver 缓存后传入
 * （**禁止**在渲染路径里读 `clientWidth` / `getBoundingClientRect`，那会每 tick 触发强制布局）。
 * 测不到宽度（jsdom / 尚未布局）时按单行处理，遮罩是否生效交给 CSS。
 */
export function isStreamTailTextSingleLine(
  text: string,
  availableWidth: number,
  measure: ((value: string) => number) | null
): boolean {
  if (!text || text.includes("\n")) return false;
  if (availableWidth <= 0) return true;
  if (!measure) return true;
  return measure(text) <= availableWidth - STREAM_TAIL_LINE_SAFETY_PX;
}
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useAttrs, watch } from "vue";
import {
  countUnclosedCodeFences,
  endsWithNaturalBoundary,
  isAllowedExternalHref,
  isSimpleTextContent,
  markdownRenderEpoch,
  renderMarkdown,
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
/** 承载节点（`.markdown-body`）：尾巴的单行宽度测量基准，也是唯一的正文承载节点。 */
const bodyRef = ref<HTMLElement | null>(null);
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

// ─── wave-pds-3：正文尾巴（契约 v5 §1 DOM 协议 / §3.1 遮罩浮现 / §2 开关） ──────────
//
// 设计要点：
// - D-59-2 保持：**不建任何字符级节点**；浮现由「尾巴右缘的静态 mask 渐变」承担（§3.1），
//   它不是 animation，内容推进时不会重启，因此与 RC2 的闪跳有本质区别。
// - `.markdown-body` 仍是**唯一**承载节点、身份稳定：尾巴是该节点 `v-html` 内容的**最后一段**，
//   不额外引入兄弟节点，也就不会在内容增长时替换承载节点身份（V3）。
// - **尾巴 = `splitMarkdownBlocks` 给出的整个未闭合块**（契约 v5 R-A1）：直接复用仓内既有块切分器，
//   绝不再"按最后一个换行切"——那样会把同一个块劈成两半各自渲染，导致表格空 tbody / `text\n---`
//   变 `<hr>` / 字面 `**` / 嵌套列表塌成并列 / 多行段落裂成两个 `<p>`。
// - 尾巴可以自然多行（契约 v5 R-A4 取消「≤1 行」硬约束）：遮罩只在**实际单行**时启用，
//   多行时 `mask-image: none` 优雅降级，而不是牺牲 markdown 语义。
// 开关 / 降级 / 资格门 / 单行判定：与纯文本快路径共用普通 `<script>` 里的同一套纯函数
// （见文件顶部的 STREAM_TAIL_* / readStreamTailSwitchPreference / checkStreamTailCandidate）。
const tailRevealPreferenceOn = readStreamTailSwitchPreference();
// jsdom 与老浏览器可能没有 matchMedia：此时按「未降级」处理，不应因缺少 API 而永久关闭尾巴。
const reduceMotionQuery = typeof window !== "undefined" && typeof window.matchMedia === "function"
  ? window.matchMedia("(prefers-reduced-motion: reduce)")
  : null;

function isTailRevealActive(): boolean {
  return tailRevealPreferenceOn && !isStreamTailReducedMotionActive(reduceMotionQuery);
}

// ── 单行判定用的测量缓存（**绝不在每 tick 付代价**：宽度由 ResizeObserver 缓存，字体只解析一次）──
/** 承载节点内容宽度（px）；0 = 尚未测到（jsdom 等无布局环境）。刻意用普通变量：
    它只被渲染路径读取，故意**不参与响应式**，避免 ResizeObserver 回调引发额外失效。 */
let tailAvailableWidth = 0;
/** 字体测量器（canvas measureText）；挂载时解析一次，之后仅在被重置后按需重建。 */
let tailMeasureText: ((text: string) => number) | null = null;
let bodyResizeObserver: ResizeObserver | null = null;

/** 解析字体测量器。**代价高**（getComputedStyle + canvas），只在挂载/失效后调用。 */
function refreshTailMeasurer(element: HTMLElement | null) {
  tailMeasureText = null;
  // 无布局（jsdom，clientWidth=0）时不要去碰 canvas：那会触发 jsdom 未实现的 getContext。
  if (!element || typeof window === "undefined" || element.clientWidth <= 0) return;
  const context = document.createElement("canvas").getContext?.("2d") ?? null;
  if (!context) return;
  const style = window.getComputedStyle(element);
  context.font = `${style.fontStyle || "normal"} ${style.fontWeight || "400"} `
    + `${style.fontSize || "14px"} ${style.fontFamily || "sans-serif"}`;
  tailMeasureText = (text: string) => context.measureText(text).width;
}

function ensureTailMeasurer(): ((text: string) => number) | null {
  if (!tailMeasureText) refreshTailMeasurer(bodyRef.value);
  return tailMeasureText;
}

onMounted(() => {
  const body = bodyRef.value;
  if (!body) return;
  refreshTailMeasurer(body);
  if (typeof ResizeObserver === "undefined") return;
  // 回调必须**极便宜**：流式期每帧盒子都在变高，这里只做一次「宽度是否变化」的比较。
  bodyResizeObserver = new ResizeObserver((entries) => {
    const entry = entries[0];
    if (!entry) return;
    const width = Number.isFinite(entry.contentRect?.width)
      ? Math.max(0, Math.floor(entry.contentRect.width))
      : 0;
    if (width > 0 && width !== tailAvailableWidth) {
      tailAvailableWidth = width;
      // 宽度变了才会影响「一行能放多少字」，此时才顺带重解析一次字体。
      refreshTailMeasurer(bodyRef.value);
      // 宽度**首次可知**（挂载后第一帧还没布局）或真实变化时，已渲染尾巴的「单行/多行」分类
      // 可能改变 ⇒ 需要重算一次，否则 `data-pds-tail-multiline` 会一直停在首帧的误判上
      // （实测：首帧 available=0 被判成单行，长段落尾巴实际折成两行却仍挂遮罩）。
      // 该回调只在**宽度**变化时走到这里，不会与流式期的逐帧高度变化形成重渲染循环。
      if (props.streaming && renderedHtml.value) {
        scheduleStreamingRender();
      }
    }
  });
  bodyResizeObserver.observe(body);
});

/**
 * 尾巴是否**实际单行**（契约 v5 R-A4：只有实际单行才启用遮罩）。
 *
 * 判定 = 渲染文本不含换行 且（能测宽时）宽度放得进一行。宽度来自 ResizeObserver 缓存，
 * 不做每 tick 的强制布局；测不到宽度（jsdom）时按单行处理，遮罩是否生效交给 CSS。
 */
function isTailRenderedSingleLine(renderedText: string): boolean {
  return isStreamTailTextSingleLine(renderedText, tailAvailableWidth, ensureTailMeasurer());
}

/** 尾巴节点（契约 §1）：inline 级、带 `data-tail-len`（**源尾巴字符数**，v5 R-A3）。 */
function buildTailMarkup(tailSource: string, tailHtml: string, singleLine: boolean): string {
  // marked 的输出结尾带换行（`<p>x</p>\n`）；它会在尾巴节点内形成一个尾随空白文本节点，
  // 使 textContent 比源后缀多一个 `\n`（V1-5 的「严格后缀、不 trim」判据会因此变红）。
  const normalizedHtml = tailHtml.replace(/\n+$/, "");
  if (!normalizedHtml) return "";
  const multilineAttr = singleLine ? "" : ' data-pds-tail-multiline="true"';
  return `<span data-stream-tail data-tail-len="${tailSource.length}"${multilineAttr}>${normalizedHtml}</span>`;
}

/**
 * 已定稿块渲染结果缓存（**NFR / V8 帧预算**的最小解，不是重构）。
 *
 * 契约 §1：块一旦闭合，其内容就移入 stableHtml、**不再变化**；而实现每帧都会重渲染
 * **全部**已定稿块（marked.parse + DOMPurify），使流式后期的单帧成本随内容线性增长 ——
 * 这正是 V8「p95 增量 ≤6ms」在长流上的主要风险来源。按「块源文本」缓存是行为等价的最小解：
 * 同一段源文本恒产出同一段 HTML；仅在 `markdownRenderEpoch` 变化（解析器热更新）时整体失效。
 */
const committedHtmlCache = new Map<string, string>();
const COMMITTED_CACHE_LIMIT = 256;
let committedHtmlCacheEpoch = -1;

async function renderCommittedBlocks(blocks: string[]): Promise<string[]> {
  if (committedHtmlCacheEpoch !== markdownRenderEpoch.value) {
    committedHtmlCache.clear();
    committedHtmlCacheEpoch = markdownRenderEpoch.value;
  }
  return Promise.all(blocks.map(async (block) => {
    const cached = committedHtmlCache.get(block);
    if (cached != null) return cached;
    const html = await renderMarkdown(block);
    if (committedHtmlCache.size >= COMMITTED_CACHE_LIMIT) committedHtmlCache.clear();
    committedHtmlCache.set(block, html);
    return html;
  }));
}

/**
 * 流式正文组装：已定稿块 + 唯一尾巴节点。
 *
 * 契约 v5/v7：尾巴 = `splitMarkdownBlocks(content).tail`（整个未闭合块，不切行、不 trim、不重排），
 * 且**仅当它是纯段落**时才发尾巴；非纯段落（代码/列表/表格/引用/HTML/数学/setext）整块追加。
 */
async function composeStreamingHtml(source: string): Promise<string> {
  const { committed, tail } = splitMarkdownBlocks(source);

  // 契约 v9：资格判定 = 渲染结果自证（同一次 renderPartialMarkdown 的结果直接复用为尾巴 HTML）。
  const candidate = isTailRevealActive() && Boolean(tail.trim())
    ? await checkStreamTailCandidate(tail)
    : null;

  if (!candidate?.eligible) {
    // 契约 §2（关闭开关）/ v9 资格门（非纯段落）—— 都走 stableHtml，不渲染 `[data-stream-tail]`。
    // 渲染语义与 HEAD 的「自动收尾 + 增量提交」保持一致，见 renderNonEligibleTail 的注释。
    if (committed.length > 0) {
      const [committedHtmlList, tailHtml] = await Promise.all([
        renderCommittedBlocks(committed),
        renderNonEligibleTail(tail)
      ]);
      return [...committedHtmlList, tailHtml].filter(Boolean).join("\n");
    }
    return renderNonEligibleTail(source);
  }

  // 资格成立：整块 partial 渲染结果作为唯一尾巴节点（多行尾巴由 CSS 关掉遮罩）。
  const committedHtmlList = await renderCommittedBlocks(committed);
  const singleLine = isTailRenderedSingleLine(tail);
  return [...committedHtmlList, buildTailMarkup(tail, candidate.html, singleLine)].filter(Boolean).join("\n");
}

/**
 * 非资格尾巴（不发尾巴时）的渲染：保持 HEAD 的「自动收尾 + 增量提交」语义，但**不闭合围栏**。
 *
 * V9-a 的实测：v7 改用 `renderMarkdown(tail)` 后丢了 `autoCloseBoundaries` 兜底，当内容末尾正好是
 * 围栏 opener 帧（如 `"…正文\n```ts"`）时，marked 的 fence 词法需要 opener 行后跟换行才能命中，
 * 否则会把字面 ``` 当段落吐出来（屏上露 1–2 帧）。
 * 修法：若尾巴里存在**未闭合围栏且末行没有换行**，只补一个 `\n`（让 fence 词法命中），
 * **不补闭合围栏** —— 这样既不会漏字面围栏（V9-a），也不会触发
 * `normalizeMarkdownSource` 的 md-unwrap（` ```md ` 的 `#` 被当标题吃掉，N3/V12-3）。
 */
async function renderNonEligibleTail(tail: string): Promise<string> {
  if (!tail.trim()) return "";
  const needsFenceLineBreak = countUnclosedCodeFences(tail) > 0 && !tail.endsWith("\n");
  return renderMarkdown(needsFenceLineBreak ? `${tail}\n` : tail);
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
      // wave-pds-3：块级增量 + 唯一尾巴节点（`[data-stream-tail]`）由 composeStreamingHtml 统一组装。
      html = await composeStreamingHtml(source);
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
  bodyResizeObserver?.disconnect();
  bodyResizeObserver = null;
  tailMeasureText = null;
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
    - wave-pds-3：流式期间 `v-html` 内容的**最后一段**是唯一 `<span data-stream-tail>`（§1），
      同样写进这个承载节点 —— 不新增兄弟节点，也不建任何字符级节点（D-59-2 保持）。
  -->
  <div v-bind="attrs" :class="[wrapperClass, toneClass]" @click="onMarkdownClick">
    <div v-if="blockedLinkNotice" class="mr-blocked-link-notice" role="status" aria-live="polite">
      {{ blockedLinkNotice }}
    </div>
    <div
      ref="bodyRef"
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

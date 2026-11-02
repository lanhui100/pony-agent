/**
 * wave-pds-3（流式视觉层接入）验收测试共享夹具 —— **非 spec**，vitest include 只收 `*.spec.ts`。
 *
 * 契约源（只读，唯一真值）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`
 *
 * 设计原则（与实现解耦，刻意不 import 任何 executor 私有符号）：
 * 1. 只使用契约点名的公开锚点：`[data-stream-tail]` / `[data-tail-len]` /
 *    `localStorage["pony-agent.stream-render.tail-reveal"]` / `[data-reduce-motion]` /
 *    `.markdown-body` / `tests` 里的既有 testid。
 * 2. 尾巴节点的**归属组件刻意不作假设**：V1/V2/V7 全部在**生产路径**
 *    （`HomeWorkspace` → `[data-testid="workspace-content-column"]`）采样，
 *    因此尾巴放在 `MarkdownRenderer.vue` 还是 `WorkspaceTurnItem.vue` 都不影响判定。
 * 3. 所有等待都用确定性推进（真实宏任务 + nextTick）或显式条件，不写盲 sleep 做竞态判定。
 * 4. jsdom 做不到的事（布局/像素/帧时间）一律留给 `e2e/*.pw.ts` 的真实 Chromium，
 *    本文件只声明能力边界，不伪造能力。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import type { Component } from "vue";
import { vi } from "vitest";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import type { ChatMessage } from "@/types/runtime";

export const REPO_ROOT = process.cwd();

/** 契约 §1：唯一尾巴节点的属性锚点。 */
export const TAIL_SELECTOR = "[data-stream-tail]";
/** 契约 §1：尾巴长度属性（协议自洽性判据）。 */
export const TAIL_LEN_ATTR = "data-tail-len";
/** 契约 §0（F3-1 修订）：这三类历史尾巴节点必须继续为 0。 */
export const LEGACY_TAIL_SELECTORS = [
  ".mr-streaming-raw",
  ".streaming-unrendered-suffix",
  ".mr-streaming-char",
  ".assistant-streaming-char"
] as const;
/** 契约 §2：回退开关。缺省 = on；值 "off" = 关闭。 */
export const TAIL_SWITCH_STORAGE_KEY = "pony-agent.stream-render.tail-reveal";
/**
 * 契约 v2 §2 / §3.5：应用内降级开关（**祖先**节点属性）。
 * v2 裁定统一为 `data-pds-reduce-motion`（与 primitives.css 同源命名，消除 v1 的命名分裂）。
 */
export const REDUCE_MOTION_ATTR = "data-pds-reduce-motion";
/** 契约 v2 §1：工具行必需属性锚点（供入场动效与验收定位）。 */
export const TOOL_ROW_SELECTOR = "[data-pds-tool-row]";

/** 契约 §7 中点名的、本波会改动动效的源文件（含 SFC `<style>`）。 */
export const CSS_SOURCES = [
  "src/styles.css",
  ".agents/skills/streaming-motion/primitives.css",
  ".agents/skills/streaming-motion/tokens.css",
  "src/components/MarkdownRenderer.vue",
  "src/components/chat/WorkspaceTurnItem.vue",
  "src/components/chat/AgentRuntimeStatusBar.vue",
  "src/components/HomeWorkspace.vue"
] as const;

// ───────────────────────────── 源码 / CSS 读取 ─────────────────────────────

export function readRepoFile(relativePath: string): string {
  return readFileSync(resolve(REPO_ROOT, relativePath), "utf8");
}

/** 抽出 SFC 的全部 `<style>` 块正文（非 SFC 文件原样返回）。 */
export function readStyleText(relativePath: string): string {
  const raw = readRepoFile(relativePath);
  if (!relativePath.endsWith(".vue")) return raw;
  const blocks = raw.match(/<style[^>]*>([\s\S]*?)<\/style>/g) ?? [];
  return blocks.map((block) => block.replace(/^<style[^>]*>/, "").replace(/<\/style>$/, "")).join("\n");
}

/**
 * 剥注释后再做源码守卫：否则 `/* [data-stream-tail] { display:block } *\/` 这类
 * 注释文本就能骗过 CSS 断言。仅去掉注释，不改动选择器/声明。
 */
export function stripCssComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

export function stripJsComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:])\/\/.*$/gm, "$1");
}

/** 全仓 CSS 池：`{ path, css }[]`（只含已存在的文件；缺失由调用方显式断言）。 */
export function collectCssPool(): Array<{ path: string; css: string }> {
  return CSS_SOURCES.map((path) => ({
    path,
    css: stripCssComments(safeRead(path))
  }));
}

function safeRead(relativePath: string): string {
  try {
    return readStyleText(relativePath);
  } catch {
    return "";
  }
}

/** 把 CSS 池拼成一段便于做「全仓规则存在性」断言的文本（带来源标注）。 */
export function cssPoolText(): string {
  return collectCssPool()
    .map((entry) => `/* ---- ${entry.path} ---- */\n${entry.css}`)
    .join("\n");
}

/** 提取所有匹配 `@media (prefers-reduced-motion: reduce) { ... }` 的块正文。 */
export function extractReducedMotionMediaBlocks(css: string): string[] {
  return extractBalancedBlocks(css, /@media\s*\(\s*prefers-reduced-motion\s*:\s*reduce\s*\)\s*\{/g);
}

/** 提取所有 `[data-reduce-motion="true"]` 作用域下的规则块正文。 */
export function extractReduceMotionAttributeBlocks(css: string): string[] {
  const blocks: string[] = [];
  const opener = /\[data-reduce-motion\s*=\s*["']true["']\s*\]\s*/g;
  let match: RegExpExecArray | null;
  while ((match = opener.exec(css)) !== null) {
    const start = css.indexOf("{", match.index + match[0].length - 1);
    if (start < 0) continue;
    const block = readBalanced(css, start);
    if (block) blocks.push(block);
  }
  return blocks;
}

function extractBalancedBlocks(css: string, opener: RegExp): string[] {
  const blocks: string[] = [];
  let match: RegExpExecArray | null;
  while ((match = opener.exec(css)) !== null) {
    const start = match.index + match[0].length - 1;
    const block = readBalanced(css, start);
    if (block) blocks.push(block);
  }
  return blocks;
}

/** 从 `{` 位置读出配平的花括号块正文（含内部嵌套）。 */
function readBalanced(css: string, openIndex: number): string {
  let depth = 0;
  for (let index = openIndex; index < css.length; index += 1) {
    const char = css[index];
    if (char === "{") depth += 1;
    else if (char === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(openIndex + 1, index);
    }
  }
  return "";
}

/** 抽出所有选择器列表里含 `[data-stream-tail]` 的规则正文（用于 display / mask 声明判定）。 */
export function extractTailRules(css: string): Array<{ selector: string; body: string }> {
  return extractRulesMatching(css, "[data-stream-tail]");
}

/** 抽出所有选择器列表里含 `token` 的规则正文（不做嵌套展开，只扫单层规则）。 */
export function extractRulesMatching(css: string, token: string): Array<{ selector: string; body: string }> {
  const rules: Array<{ selector: string; body: string }> = [];
  const rulePattern = /([^{}@]+)\{([^{}]*)\}/g;
  let match: RegExpExecArray | null;
  while ((match = rulePattern.exec(css)) !== null) {
    const selector = (match[1] ?? "").trim();
    const body = (match[2] ?? "").trim();
    if (selector.includes(token)) rules.push({ selector, body });
  }
  return rules;
}

export interface ReducedMotionRule {
  /** 降级轨道：系统媒体查询 / 应用内根属性。 */
  track: "media" | "attribute";
  selector: string;
  body: string;
}

/**
 * 抽出「降级轨道」内作用于某选择器片段的规则。
 *
 * 双轨判据（契约 §2 / §3.5 强制）：
 * - `media`：`@media (prefers-reduced-motion: reduce) { ... }` 内的规则；
 * - `attribute`：选择器里带 `[data-pds-reduce-motion="true"]` 前缀的规则。
 *   （契约 v2 已统一命名为 `data-pds-reduce-motion`，与 primitives.css 一致；不再有命名分歧。）
 */
export function findReducedMotionRules(css: string, selectorToken: string): ReducedMotionRule[] {
  const found: ReducedMotionRule[] = [];

  for (const block of extractReducedMotionMediaBlocks(css)) {
    for (const rule of extractRulesMatching(block, selectorToken)) {
      found.push({ track: "media", ...rule });
    }
  }

  const attributePattern = /([^{}@]*\[data-pds-reduce-motion\s*=\s*["']true["']\s*\][^{}]*)\{([^{}]*)\}/g;
  let match: RegExpExecArray | null;
  while ((match = attributePattern.exec(css)) !== null) {
    const selector = (match[1] ?? "").trim();
    const body = (match[2] ?? "").trim();
    if (selector.includes(selectorToken)) found.push({ track: "attribute", selector, body });
  }

  return found;
}

/** 规则正文里是否声明了「关闭动效」三元组中的某一项。 */
export function declaresMotionOff(body: string): boolean {
  return /(?:^|;)\s*animation\s*:\s*none\b/.test(body)
    || /(?:^|;)\s*transition\s*:\s*none\b/.test(body)
    || /(?:^|;)\s*mask-image\s*:\s*none\b/.test(body);
}

// ───────────────────────────── jsdom 环境桩 ─────────────────────────────

/**
 * jsdom 缺失的浏览器 API：与 wave-59 验收夹具同构（`scrollIntoView` 无布局、rAF 同步执行），
 * 保证流式释放逻辑能确定性推进；不参与任何断言。
 */
export function installDomStubs() {
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: function scrollIntoViewStub() {
      /* jsdom 无布局，滚动无副作用 */
    }
  });

  vi.stubGlobal("requestAnimationFrame", ((callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  }) as typeof requestAnimationFrame);
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
}

/**
 * 确定性推进：N 轮「真实宏任务 + nextTick」。
 * 只用于让异步渲染/定时器落地，不承担「睡够时间就等于发生」的竞态判定。
 */
export async function settle(rounds = 3, delayMs = 24) {
  for (let index = 0; index < rounds; index += 1) {
    await new Promise<void>((resolvePromise) => window.setTimeout(resolvePromise, delayMs));
    await nextTick();
  }
}

/** markdown 渲染链路（16ms 节流 + async 解析）所需的确定性推进。 */
export async function settleRender() {
  await settle(6, 24);
}

/** 契约 §2：设置尾巴开关（`null` = 删除键，即缺省 on）。 */
export function setTailSwitch(value: "off" | "on" | null) {
  if (value === null) {
    window.localStorage.removeItem(TAIL_SWITCH_STORAGE_KEY);
    return;
  }
  window.localStorage.setItem(TAIL_SWITCH_STORAGE_KEY, value);
}

/**
 * 契约 §2：系统级降级。matchMedia 只对 reduced-motion 查询返回 true，
 * 其余查询返回 false（避免把无关媒体查询一并伪装成「命中」，制造与降级无关的偏差）。
 */
export function stubReducedMotion(matches: boolean) {
  const impl = (query: string) => ({
    matches: matches && /prefers-reduced-motion\s*:\s*reduce/.test(query),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false
  });
  vi.stubGlobal("matchMedia", impl as unknown as typeof window.matchMedia);
}

/** 契约 §2：应用内降级开关（根节点属性）。 */
export function setRootReduceMotion(on: boolean) {
  if (on) {
    document.documentElement.setAttribute(REDUCE_MOTION_ATTR, "true");
  } else {
    document.documentElement.removeAttribute(REDUCE_MOTION_ATTR);
  }
}

// ───────────────────────────── 挂载（生产路径） ─────────────────────────────

export const ButtonStub = defineComponent({
  props: {
    disabled: { type: Boolean, default: false },
    title: { type: String, default: "" }
  },
  emits: ["click"],
  template: "<button class=\"button-stub\" type=\"button\" @click=\"$emit('click')\"><slot /></button>"
});

export const ScrollAreaStub = defineComponent({
  setup(_props, { slots }) {
    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

/** motion-v 指令在单测环境未注册；no-op 桩只为消除解析噪声，不参与断言。 */
export const noopMotionDirective = {
  mounted() {
    /* no-op */
  },
  updated() {
    /* no-op */
  },
  beforeUpdate() {
    /* no-op */
  }
};

/**
 * 挂载**真实** HomeWorkspace（不桩 MarkdownRenderer / 不桩 WorkspaceTurnItem），
 * 使尾巴节点的归属组件对断言透明。
 */
export function mountWorkspace(pinia: unknown) {
  return mount(
    defineComponent({
      name: "PdsStreamingWorkspaceHost",
      render() {
        return h(TooltipProvider, null, { default: () => h(HomeWorkspace) });
      }
    }),
    {
      global: {
        plugins: [pinia as never],
        directives: { motion: noopMotionDirective },
        stubs: {
          ScrollArea: ScrollAreaStub,
          Button: ButtonStub,
          Transition: false,
          TransitionGroup: false
        }
      }
    }
  );
}

/** 内容列：契约 §6「内容区」的稳定锚点（wave-59 起沿用）。 */
export const CONTENT_COLUMN_SELECTOR = '[data-testid="workspace-content-column"]';

export function contentColumn(wrapper: { find: (selector: string) => { exists: () => boolean; element: Element } }) {
  const found = wrapper.find(CONTENT_COLUMN_SELECTOR);
  return found.exists() ? found.element : null;
}

/** 在 store 里铺一条 user + assistant 轮次（生产路径的最小可流式状态）。 */
export async function patchAssistant(
  store: { $patch: (payload: Record<string, unknown>) => Promise<void> },
  content: string,
  status: ChatMessage["status"]
) {
  await store.$patch({
    messages: [
      {
        id: "user-pds",
        turnId: "turn-pds",
        role: "user",
        content: "继续",
        status: "done",
        tokenCount: null,
        reasoningContent: null,
        modelName: null,
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      },
      {
        id: "assistant-pds",
        turnId: "turn-pds",
        role: "assistant",
        content,
        status,
        tokenCount: null,
        reasoningContent: null,
        modelName: "OpenAI/GPT-5",
        toolName: null,
        detail: null,
        durationSeconds: null,
        errorDetail: null
      }
    ],
    isSubmitting: status === "pending",
    phase: status === "pending" ? "calling_model" : "ready",
    activeTurnId: "turn-pds",
    activeTurnStartedAtMs: Date.now()
  });
}

// ───────────────────────────── 流式内容夹具 ─────────────────────────────

/**
 * 合成 markdown 流：每一步都**以未定稿块收尾**（无尾随空行），
 * 因此任何符合契约 §1 的实现都必然存在「当前未定稿块」⇒ 尾巴必须存在。
 *
 * 用 markdown 而不是纯文本，是为了绕开契约未定义的纯文本快路径歧义（见交付说明）。
 */
export const MD_COMMITTED_BLOCKS = [
  "## 流式标题\n\n",
  "第一段正文，包含中文与 English 混排的输出内容。\n\n",
  "- 列表项一\n- 列表项二\n\n",
  "```ts\nconst answer = 42;\n```\n\n"
] as const;

export const MD_TAIL_TEXTS = [
  "尾段正在输出",
  "尾段正在输出第二句，长度继续增长",
  "尾段正在输出第三句，长度继续增长直至超过一行",
  "尾段正在输出第四句，长度继续增长，用于观察尾巴是否仍为尾段",
  "尾段正在输出第五句，收尾"
] as const;

/**
 * 生成 N 个流式快照（累积定稿块 + 一个未定稿尾段）。
 *
 * 关键约束（实测踩坑）：**每一个快照都必须含至少一个 markdown 定稿块**。
 * 否则首帧是纯文本 ⇒ `shouldUseMarkdownAssistantRendering` 走纯文本快路径，
 * 第二帧检测到 markdown 语法后切到 MarkdownRenderer，承载节点被合法替换一次，
 * 会把「开关/尾巴」的断言污染成「渲染路径切换」的假红。
 */
export function mdStreamSteps(): string[] {
  const steps: string[] = [];
  for (let index = 0; index < MD_TAIL_TEXTS.length; index += 1) {
    const committed = MD_COMMITTED_BLOCKS.slice(0, Math.min(index + 1, MD_COMMITTED_BLOCKS.length)).join("");
    steps.push(`${committed}${MD_TAIL_TEXTS[index]}`);
  }
  return steps;
}

/** 全部定稿块 + 尾段（用于「全文可见」断言的内容完整度判据）。 */
export function mdFullContent(): string {
  return `${MD_COMMITTED_BLOCKS.join("")}${MD_TAIL_TEXTS[MD_TAIL_TEXTS.length - 1]}`;
}

/** 内容里必须可见的文本标记（markdown 语法字符不计）。 */
export const MD_VISIBLE_MARKERS = [
  "流式标题",
  "第一段正文",
  "列表项一",
  "列表项二",
  "const answer = 42;",
  "尾段正在输出"
] as const;

// ───────────────────────────── 采样 ─────────────────────────────

export interface StreamNodesProbe {
  /** `[data-stream-tail]` 数量（契约 §1：必须恒为 1）。 */
  tailCount: number;
  /** `[data-tail-len]` 数量（协议属性；出现多个即等于出现第二个尾巴）。 */
  tailLenAttrCount: number;
  /** 三类历史尾巴节点数量（契约 §0：必须为 0）。 */
  legacy: Record<string, number>;
  /** 每个尾巴节点的协议快照。 */
  tails: Array<{
    text: string;
    textLength: number;
    declaredLen: number | null;
    tagName: string;
    className: string;
    display: string;
  }>;
  /** 承载节点文本长度（自检：内容确实到达了 DOM）。 */
  bodyTextLength: number;
  /** 承载节点存在性（身份锚点）。 */
  bodyPresent: boolean;
}

/**
 * 一次性采样：把「某类节点是否存在/数量多少」做成可断言的纯数据，
 * 避免选择器散落在用例里。`root` 必须是**内容区**（不是整个应用）。
 */
export function probeStreamNodes(root: Element | null): StreamNodesProbe {
  if (!root) {
    return {
      tailCount: 0,
      tailLenAttrCount: 0,
      legacy: {},
      tails: [],
      bodyTextLength: 0,
      bodyPresent: false
    };
  }

  const tails = Array.from(root.querySelectorAll(TAIL_SELECTOR));
  const legacy: Record<string, number> = {};
  for (const selector of LEGACY_TAIL_SELECTORS) {
    legacy[selector] = root.querySelectorAll(selector).length;
  }

  const body = root.querySelector(".markdown-body");

  return {
    tailCount: tails.length,
    tailLenAttrCount: root.querySelectorAll(`[${TAIL_LEN_ATTR}]`).length,
    legacy,
    tails: tails.map((tail) => {
      const declared = tail.getAttribute(TAIL_LEN_ATTR);
      return {
        text: tail.textContent ?? "",
        textLength: (tail.textContent ?? "").length,
        declaredLen: declared != null && declared.trim() !== "" ? Number(declared) : null,
        tagName: tail.tagName.toLowerCase(),
        className: tail.className ?? "",
        display: window.getComputedStyle(tail).display
      };
    }),
    bodyTextLength: (body?.textContent ?? "").length,
    bodyPresent: Boolean(body)
  };
}

/** 历史尾巴节点总数（契约 §0 的判据聚合）。 */
export function legacyTailTotal(probe: StreamNodesProbe): number {
  return Object.values(probe.legacy).reduce((sum, count) => sum + count, 0);
}

/** 挂载真实组件（用于需要 inspect 子组件的场景）。 */
export function mountWithTooltipProvider(component: Component, props: Record<string, unknown>) {
  const Host = defineComponent({
    name: "PdsStreamingTooltipHost",
    setup() {
      return () => h(TooltipProvider, {}, { default: () => h(component, props) });
    }
  });
  return mount(Host, { global: { stubs: { teleport: true } } });
}

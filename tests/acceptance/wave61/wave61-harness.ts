/**
 * Wave-61 冻结验收测试共享夹具（**非 spec**，vitest include 只收 `*.spec.ts`）。
 *
 * 契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md`（§1 L-* / §2 E-* / §3 M-*）。
 *
 * 设计原则（与实现解耦）：
 * 1. 本文件只提供「可重复的数据注入 + 挂载 + CSS 面读取」能力，不携带任何断言语义；
 * 2. 只使用契约点名的公开锚点（`data-testid` / 契约类名 / 现有 props 面），
 *    不引用 executor 的私有符号；
 * 3. 所有等待走显式 tick，不写盲 sleep。
 *
 * 为什么要读 CSS 文本（M-* 动效契约）：
 *   jsdom 既不执行 Tailwind，也**不处理** SFC 的 `<style>` 块（vitest `css` 默认关闭），
 *   因此 `getComputedStyle` 在 jsdom 层无法观测到 200ms / cubic-bezier / reduced-motion 屏蔽规则。
 *   契约 §3 允许的降级口径是「scoped CSS 中存在 enter 类规则 + 降级关闭规则」，
 *   故本夹具提供 `readWave61CssSurfaces()`：把**应用真实会打包的两处 CSS 面**
 *   （`src/components/HomeStatusPanel.vue` 的 `<style>` 块 + `src/styles.css`）解析成
 *   可断言的规则/关键帧结构；spec 再把这些规则与新挂载 DOM 的真实 class 集合求交，
 *   以排除「CSS 写了但 DOM 从未挂该类」的假实现。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import fs from "node:fs";
import path from "node:path";
import { defineComponent, h } from "vue";
import type { Component } from "vue";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import Tooltip from "@/components/ui/Tooltip.vue";

// ── 契约锚点（§1/§2 原文 testid，不得由测试自行发明） ─────────────────────────

/** §1 L-3：计数行锚点。 */
export const COUNTS_ROW = "status-metrics-counts";
/** §1 L-3：token 行新增锚点。 */
export const TOKENS_ROW = "status-metrics-tokens";
/** §1 L-2：圆环位 testid（**不得**使用 `status-metric-` 前缀）。 */
export const RING_SLOT = "status-context-usage";

/** §1 L-2：计数位顺序（轮次 → 模型调用 → 工具调用）。 */
export const COUNT_SLOT_TESTIDS = [
  "status-metric-turn-count",
  "status-metric-model-call-count",
  "status-metric-tool-call-count"
] as const;

/** §2 E-2：token 位顺序（输入 → 输出 → 缓存 → 生成速度）。 */
export const TOKEN_SLOT_TESTIDS = [
  "status-metric-input-tokens",
  "status-metric-output-tokens",
  "status-metric-cache-tokens",
  "status-metric-token-speed"
] as const;

/** §1 L-4：位点预留下限（契约表格原文，Lead 实测校准版）。 */
export const SLOT_MIN_WIDTH = {
  count: "min-w-[3rem]",
  token: "min-w-[3rem]",
  speed: "min-w-[3.5rem]",
  ring: "min-w-[3rem]"
} as const;

/** §1 L-4 末句：任何位点 min-w 不得超过 3.5rem（4.25rem=68px 会撑破 63.5px track）。 */
export const SLOT_MIN_WIDTH_CEILING_REM = 3.5;

/**
 * §3 M-1 裁决 R4（Lead 机制钉死）：入场动效必须是「位点自身静态 class + `@keyframes`」，
 * 类名固定 `.status-slot-enter`；圆环位附加 `.status-slot-enter--ring`。
 * 理由（契约原文）：位点外层是 `Tooltip`（Fragment 根），`<TransitionGroup>` 不会下发 enter 类。
 */
export const SLOT_ENTER_CLASS = "status-slot-enter";
export const RING_ENTER_CLASS = "status-slot-enter--ring";

/** §1 L-5：指标卡 class（`px-3 py-2` 挂在它身上）。 */
export const METRIC_CARD_CLASS = "bg-[#f6f0e8]";

/**
 * 「有值」夹具：所有位点都能渲染（用于结构/动效层断言，与 §2 空值夹具互补）。
 * 数值选型沿用 wave-17/59 既有夹具量纲，避免引入新的口径争议。
 */
export const STATUS_PANEL_VALUE_PROPS: Record<string, unknown> = {
  sessionId: "session-wave61",
  turnCount: 9,
  modelCallCount: 12,
  toolCallCount: 345,
  copied: false,
  inputTokensTotal: 900,
  outputTokensTotal: 1200,
  cacheHitTokensTotal: 3450,
  cacheHitRatio: "12%",
  tokenGenerationSpeed: 42.7,
  showContextUsage: true,
  contextDisplayTokens: 9,
  contextWindowTokens: 200_000
};

/**
 * 挂载被测组件：包进 reka-ui TooltipProvider（`Tooltip.vue` 的 TooltipRoot 强依赖该 context，
 * 缺失会在 setup 抛错）。与 wave-17/59 既有夹具同构。
 */
export function mountWithTooltipProvider(component: Component, props: Record<string, unknown>) {
  const Host = defineComponent({
    name: "Wave61TooltipHost",
    setup() {
      return () => h(TooltipProvider, {}, { default: () => h(component, props) });
    }
  });
  return mount(Host, { global: { stubs: { teleport: true } } });
}

/** 以「有值」夹具挂载 HomeStatusPanel，可按需覆盖单个 prop（空值用例靠它构造）。 */
export function mountStatusPanel(overrides: Record<string, unknown> = {}) {
  return mountWithTooltipProvider(HomeStatusPanel, {
    ...STATUS_PANEL_VALUE_PROPS,
    ...overrides
  });
}

/** 取挂载树内（含根）所有元素，用于按 classList 精确匹配 Tailwind 任意值 class。 */
export function allElements(root: Element): Element[] {
  return [root, ...Array.from(root.querySelectorAll("*"))];
}

/** 统计带指定 class token 的元素（class token 含 `[`/`#`/`]`，不能用 CSS 选择器安全表达）。 */
export function countByClassToken(root: Element, classToken: string): number {
  return allElements(root).filter((element) => element.classList.contains(classToken)).length;
}

/**
 * `tabular-nums` 允许落在承载位自身，或落在其 ≤`maxAncestorHops` 层祖先（位容器/行容器）。
 * 与 `tests/acceptance/wave17/tabular-nums-contract.spec.ts` 判定同口径（wave-17 C2 不回归）。
 */
export function hasTabularNumsAtOrAbove(element: Element, maxAncestorHops = 2): boolean {
  let node: Element | null = element;
  for (let hop = 0; hop <= maxAncestorHops && node != null; hop += 1) {
    if (node.classList.contains("tabular-nums")) {
      return true;
    }
    node = node.parentElement;
  }
  return false;
}

/** 失败信息用的祖先链回显。 */
export function tabularNumsFailureDetail(element: Element, maxAncestorHops = 2): string {
  const chain: string[] = [];
  let node: Element | null = element;
  for (let hop = 0; hop <= maxAncestorHops && node != null; hop += 1) {
    chain.push(`<${node.tagName.toLowerCase()} class="${node.className}">`);
    node = node.parentElement;
  }
  return chain.join(" ↑ ");
}

/** 元素子树内所有 class token 的并集（用于把 CSS 规则与真实 DOM 求交）。 */
export function collectClassTokens(elements: Element[]): Set<string> {
  const tokens = new Set<string>();
  for (const element of elements) {
    for (const token of Array.from(element.classList)) {
      tokens.add(token);
    }
  }
  return tokens;
}

// ── CSS 面读取与解析（M-* 动效契约在 jsdom 层的唯一可观测代理） ─────────────────

export interface Wave61CssRule {
  /** 文件来源（失败信息可追溯）。 */
  source: string;
  /** 选择器原文（含祖先 at-rule 之外的完整选择器文本）。 */
  selector: string;
  /** 声明块原文（已剥离注释）。 */
  body: string;
  /** 祖先 at-rule prelude，例如 `@media (prefers-reduced-motion: reduce)`。 */
  atRules: string[];
}

export interface Wave61Keyframes {
  source: string;
  name: string;
  /** 关键帧内部帧（selector = `from` / `to` / `0%` / `100%` / `50%`…）。 */
  frames: Wave61CssRule[];
  body: string;
}

export interface Wave61CssSurface {
  rules: Wave61CssRule[];
  keyframes: Wave61Keyframes[];
  /** 参与解析的文件（用于失败信息）。 */
  files: string[];
}

const COMPONENT_RELATIVE_PATH = path.join("src", "components", "HomeStatusPanel.vue");
const GLOBAL_CSS_RELATIVE_PATH = path.join("src", "styles.css");

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

/** 去空白 + 小写：用于「数值/关键字是否出现在某条声明里」的比对（排布顺序不敏感）。 */
export function compactCss(text: string): string {
  return text.replace(/\s+/g, "").toLowerCase();
}

/** 选择器压缩：去空白、去引号、小写（`[data-pds-reduce-motion="true"]` ⇒ `[data-pds-reduce-motion=true]`）。 */
export function compactSelector(selector: string): string {
  return selector.replace(/\s+/g, "").replace(/["']/g, "").toLowerCase();
}

/** 选择器里的 class token 列表（`.a .b:hover` ⇒ `["a","b"]`）。 */
export function selectorClassTokens(selector: string): string[] {
  return Array.from(selector.matchAll(/\.([A-Za-z0-9_\\-]+)/g)).map((match) => match[1]!);
}

/**
 * 极简 CSS 解析：把文本拆成「普通规则 / @keyframes / 其他 at-rule（递归）」。
 * 只需覆盖本项目实际写法（无 CSS 嵌套语法糖、无 `@import` 内联），够用于契约断言。
 */
function parseCssInto(
  css: string,
  source: string,
  atRules: string[],
  rules: Wave61CssRule[],
  keyframes: Wave61Keyframes[]
): void {
  const text = stripComments(css);
  let buffer = "";
  let index = 0;
  while (index < text.length) {
    const char = text[index]!;
    if (char === "{") {
      const prelude = buffer.trim();
      let depth = 1;
      let cursor = index + 1;
      while (cursor < text.length && depth > 0) {
        if (text[cursor] === "{") {
          depth += 1;
        } else if (text[cursor] === "}") {
          depth -= 1;
        }
        cursor += 1;
      }
      const body = text.slice(index + 1, cursor - 1);
      if (/^@(-[a-z]+-)?keyframes\b/i.test(prelude)) {
        const name = prelude.replace(/^@(-[a-z]+-)?keyframes\s+/i, "").trim();
        const frames: Wave61CssRule[] = [];
        parseCssInto(body, source, [...atRules, prelude], frames, []);
        keyframes.push({ source, name, frames, body });
      } else if (prelude.startsWith("@")) {
        parseCssInto(body, source, [...atRules, prelude], rules, keyframes);
      } else {
        rules.push({ source, selector: prelude, body, atRules });
      }
      buffer = "";
      index = cursor;
      continue;
    }
    if (char === ";") {
      // 顶层 at-statement（@charset / @import 等），无声明块，丢弃。
      buffer = "";
      index += 1;
      continue;
    }
    buffer += char;
    index += 1;
  }
}

/** 抽取 SFC 内的 `<style>` 块正文。 */
function extractSfcStyleBlocks(vueSource: string): string[] {
  return Array.from(vueSource.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/gi)).map((match) => match[1]!);
}

/**
 * 读取本契约允许的两处 CSS 面：
 * - `src/components/HomeStatusPanel.vue` 的 `<style>`（契约首选：scoped 动效）；
 * - `src/styles.css`（契约兜底：仅当 scoped 无法满足 reduced-motion 双轨约定时）。
 * 文件缺失或从未声明任何动效规则时，由 spec 的断言失败暴露（不在此静默降级）。
 */
export function readWave61CssSurfaces(): Wave61CssSurface {
  const projectRoot = process.cwd();
  const componentPath = path.join(projectRoot, COMPONENT_RELATIVE_PATH);
  const globalCssPath = path.join(projectRoot, GLOBAL_CSS_RELATIVE_PATH);

  const files: string[] = [];
  const rules: Wave61CssRule[] = [];
  const keyframes: Wave61Keyframes[] = [];

  if (!fs.existsSync(componentPath)) {
    throw new Error(`wave-61 夹具失效：被测组件不存在 → ${componentPath}`);
  }
  if (!fs.existsSync(globalCssPath)) {
    throw new Error(`wave-61 夹具失效：全局样式不存在 → ${globalCssPath}`);
  }

  const componentSource = fs.readFileSync(componentPath, "utf-8");
  files.push(COMPONENT_RELATIVE_PATH);
  for (const block of extractSfcStyleBlocks(componentSource)) {
    parseCssInto(block, COMPONENT_RELATIVE_PATH, [], rules, keyframes);
  }

  const globalCss = fs.readFileSync(globalCssPath, "utf-8");
  files.push(GLOBAL_CSS_RELATIVE_PATH);
  parseCssInto(globalCss, GLOBAL_CSS_RELATIVE_PATH, [], rules, keyframes);

  return { rules, keyframes, files };
}

/** 取关键帧的 `from`（或 `0%`）与 `to`（或 `100%`）帧；缺失返回 null。 */
export function keyframeFrame(
  keyframesBlock: Wave61Keyframes,
  which: "from" | "to"
): Wave61CssRule | null {
  const aliases = which === "from" ? ["from", "0%"] : ["to", "100%"];
  return (
    keyframesBlock.frames.find((frame) => aliases.includes(compactSelector(frame.selector))) ?? null
  );
}

/** 取规则声明块中 `animation` / `animation-name` 引用到的、本 CSS 面已定义的 keyframes 名。 */
export function referencedKeyframeNames(body: string, knownNames: string[]): string[] {
  const found = new Set<string>();
  const declarations = stripComments(body).split(";");
  for (const declaration of declarations) {
    const colonIndex = declaration.indexOf(":");
    if (colonIndex < 0) {
      continue;
    }
    const property = declaration.slice(0, colonIndex).trim().toLowerCase();
    if (property !== "animation" && property !== "animation-name") {
      continue;
    }
    const value = declaration.slice(colonIndex + 1);
    for (const name of knownNames) {
      const pattern = new RegExp(`(^|[\\s,])${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([\\s,]|$)`, "i");
      if (pattern.test(value)) {
        found.add(name);
      }
    }
  }
  return Array.from(found);
}

/** 规则选择器的 class token 是否与真实 DOM class 集合有交集（否则该规则不可能命中被测位点）。 */
export function ruleTargetsClasses(rule: Wave61CssRule, classTokens: Set<string>): boolean {
  return selectorClassTokens(rule.selector).some((token) => classTokens.has(token));
}

/**
 * 读取挂载树内全部 `Tooltip` 组件的 `text` 文案。
 *
 * 为什么要走组件层而不是 DOM 文本：
 *   reka-ui 的 `TooltipContent` 只在 tooltip 打开时才渲染（且走 Portal），
 *   jsdom 下未打开的 tooltip 在 DOM 里**完全不可见**；
 *   而契约 §2 E-2（裁决 R1）与 §1 L-2 都要求「信息迁移/承载在 tooltip 文案里」，
 *   故这里读取组件 props 作为「文案存在性」的可观测代理（文案内容仍是契约原文语义）。
 */
export function collectTooltipTexts(wrapper: {
  findAllComponents: (definition: unknown) => Array<{ props: (key: string) => unknown }>;
}): string[] {
  return wrapper
    .findAllComponents(Tooltip)
    .map((tooltip) => tooltip.props("text"))
    .filter((text): text is string => typeof text === "string");
}

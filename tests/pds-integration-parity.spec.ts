/**
 * PDS-SM 接入生产路径 · 契约 A1–A7（`docs/decisions/0058`、`.dev-team/contracts/pds-integration.md`）
 *
 * 与 `tests/useStreamingPresentationState.spec.ts` 的分工：
 * - 那个文件是**等价性基线**（只读，锁 14 条历史行为）；
 * - 本文件锁**接入本身**：生产代码确实复用了 reducer（而不是复制算法）、常量确实单源化、
 *   且 hook 的逐 tick 输出与 reducer 对拍一致。
 *
 * A2/A3 是源码守卫，A4b/A4d 是对重构前算法手算的黄金表：这三组实现前红、实现后绿。
 * A4/A4c/A5/A6 实现前后都必须绿。
 * 其中 A4 只证"接线正确"（两边同源），**不构成**算法未漂移的证据——那是 A4b/A4d 的职责。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { computed, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStreamingPresentationState } from "@/lib/useStreamingPresentationState";
import { DEFAULT_REVEAL_BUDGET, INITIAL_REVEAL_STATE, stepReveal } from "../.agents/skills/streaming-motion/logic/budget";
import type { ChatMessage } from "@/types/runtime";

const REPO_ROOT = process.cwd();

function readSource(relativePath: string): string {
  return readFileSync(resolve(REPO_ROOT, relativePath), "utf8");
}

/**
 * 剥掉注释后再做源码守卫：否则 `// 调用了 stepReveal(...)` 这样的注释就能骗过断言。
 * 不是完整词法器，但对本仓库的 TS 源码足够（这些文件没有含 `//` 的字符串字面量）。
 */
function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:])\/\/.*$/gm, "$1");
}

/**
 * 「宿主侧不得重实现释放算法」的判据集合。
 *
 * 重要限定：**源码守卫不可能穷举所有重实现写法**，它是冒烟级而不是证明级。
 * 真正不可绕过的判据是 A4b / A4d 的黄金表——任何算术改动都会改变数字，必然变红。
 * 这里覆盖的是"照着旧实现抄回来"的现实写法；覆盖面本身由下面的自测用例锁定，
 * 所以新增绕过形态时会被明确发现，而不是静默失效。
 */
const INLINE_ALGORITHM_RULES: Array<{ label: string; pattern: RegExp }> = [
  { label: "出现 backlog 概念（积压加速归 reducer）", pattern: /\bbacklog\b/ },
  { label: "出现停滞阈值常量（含 0x22 等非十进制写法）", pattern: /\bSTALL[A-Z_0-9]*\s*=/ },
  { label: "自行自增停滞计数", pattern: /stallTicks\s*(\+\+|\+=)/ },
  { label: "按位移模拟除法", pattern: /\b[A-Za-z_$][\w$]*\s*(?:>>|<<)\s*\d/ },
  { label: "用小数乘法模拟比例加速", pattern: /\b[A-Za-z_$][\w$]*\s*\*\s*0?\.\d+/ },
  { label: "对长度/游标做除法", pattern: /\b(?:incoming|visible|content)\b[^;\n]*[/]\s*\d/ }
];

function findInlineAlgorithmViolations(code: string): string[] {
  return INLINE_ALGORITHM_RULES
    .filter((rule) => rule.pattern.test(code))
    .map((rule) => rule.label);
}

function createAssistantMessage(content: string, status: ChatMessage["status"] = "pending"): ChatMessage {
  return {
    id: "assistant-1",
    turnId: "turn-1",
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
  };
}

describe("PDS-SM 接入 · A2：生产代码复用 reducer 而非复制算法", () => {
  it("useStreamingPresentationState 从 streaming-motion/logic 引入 stepReveal", () => {
    // Arrange
    const raw = readSource("src/lib/useStreamingPresentationState.ts");
    const code = stripComments(raw);

    // Act
    const hasImport = /from\s+["'][^"']*streaming-motion\/logic\/budget["']/.test(code);

    // Assert：必须在**代码**里真正调用（剥注释后判定，防止「注释里写 stepReveal(」蒙混）
    expect(hasImport, "必须 import 自 .agents/skills/streaming-motion/logic/budget").toBe(true);
    expect(code, "必须实际调用 stepReveal（仅 import 或仅注释提及不算接入）").toMatch(/\bstepReveal\s*\(/);
  });

  it("useStreamingPresentationState 不再持有内联的释放算法", () => {
    // Arrange
    const code = stripComments(readSource("src/lib/useStreamingPresentationState.ts"));

    // Act
    const violations = findInlineAlgorithmViolations(code);

    // Assert
    expect(violations).toEqual([]);
  });

  it("守卫自身的覆盖面：已知绕过形态必须全部被拦下", () => {
    // Arrange：含上一轮独立审查实测可绕过本仓旧守卫的写法
    const bypassAttempts = [
      "const backlog = 0;",
      "const STALL_LIMIT_TICKS = 0x22;",
      "const probe = backlog >> 3;",
      "const probe = backlog * 0.125;",
      "const probe = backlog / DIVISOR;",
      "const probe = (incoming - visible) / 8;",
      "stallTicks += 1;"
    ];

    // Act
    const missed = bypassAttempts.filter((snippet) => findInlineAlgorithmViolations(snippet).length === 0);

    // Assert
    expect(missed, `以下绕过形态未被守卫拦下：${JSON.stringify(missed)}`).toEqual([]);
  });

  it("守卫不误伤：合法的接入写法与类型标注必须放行", () => {
    // Arrange
    const legitimate = [
      "const next = stepReveal(previous, { incomingLength: incoming, reduceMotion }, budget);",
      "const charsPerTick = Math.max(1, Math.floor(value));",
      "const nextReveal = previousReveal.visible > incoming ? { ...advanced, visible: previousReveal.visible } : advanced;",
      'import { DEFAULT_REVEAL_BUDGET } from "../../.agents/skills/streaming-motion/logic/budget";',
      "const streamRevealStateByMessageId = shallowReactive<Record<string, RevealState>>({});"
    ];

    // Act
    const falsePositives = legitimate.filter((snippet) => findInlineAlgorithmViolations(snippet).length > 0);

    // Assert
    expect(falsePositives, `以下合法写法被误伤：${JSON.stringify(falsePositives)}`).toEqual([]);
  });
});

describe("PDS-SM 接入 · A3：useTimelineAutoScroll 视口常量单源化", () => {
  it("引入 DEFAULT_SCROLL_INTENT", () => {
    // Arrange
    const code = stripComments(readSource("src/lib/useTimelineAutoScroll.ts"));

    // Act + Assert
    expect(code).toMatch(/from\s+["'][^"']*streaming-motion\/logic\/intent["']/);
    expect(code).toMatch(/\bDEFAULT_SCROLL_INTENT\b/);
  });

  it("两个常量正向投影自 DEFAULT_SCROLL_INTENT 的对应字段", () => {
    // Arrange
    const code = stripComments(readSource("src/lib/useTimelineAutoScroll.ts"));

    // Act + Assert：正向断言来源字段——只断言「不等于 260」会被 0x104 / 2.6e2 / 130*2 绕过
    expect(code).toMatch(/AUTO_SCROLL_THRESHOLD_PX\s*=\s*DEFAULT_SCROLL_INTENT\.pinTolerancePx\s*;/);
    expect(code).toMatch(/USER_SCROLL_IDLE_MS\s*=\s*DEFAULT_SCROLL_INTENT\.idleResumeMs\s*;/);
  });

  it("不再出现任何硬编码的 260 / 3000 字面量", () => {
    // Arrange
    const code = stripComments(readSource("src/lib/useTimelineAutoScroll.ts"));

    // Act + Assert
    expect(code).not.toMatch(/\b260\b/);
    expect(code).not.toMatch(/\b3000\b/);
  });
});

describe("PDS-SM 接入 · A4：hook 与 reducer 逐 tick 对拍", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("内容按不规则增长时，hook 的可见长度与 stepReveal 完全一致", () => {
    // Arrange：包含短内容、跨过 120 积压阈值的长内容、以及原地重复（触发停滞计数）
    const growth = [4, 9, 20, 60, 200, 200, 200, 233, 400];
    const fullContent = "流".repeat(400);
    const messages = ref<ChatMessage[]>([createAssistantMessage(fullContent.slice(0, growth[0]!))]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    let shadow = INITIAL_REVEAL_STATE;
    const observed: Array<{ hook: number; reducer: number; incoming: number }> = [];

    // Act
    for (const length of growth) {
      messages.value = [createAssistantMessage(fullContent.slice(0, length))];
      state.syncStreamingPresentationState();

      shadow = stepReveal(shadow, { incomingLength: length }, DEFAULT_REVEAL_BUDGET);

      observed.push({
        hook: state.assistantDisplayContent(messages.value[0]!).length,
        reducer: shadow.visible,
        incoming: length
      });
    }

    // Assert：逐 tick 逐个比对，失败时把整条曲线打出来
    expect(observed.map((item) => item.hook)).toEqual(observed.map((item) => item.reducer));

    // Assert 自检：确实发生了渐进释放（排除"两边都等于 incoming"的假绿）
    expect(observed.some((item) => item.hook < item.incoming)).toBe(true);
    expect(observed[observed.length - 1]!.hook).toBeGreaterThan(observed[0]!.hook);
  });

  it("自定义释放速率通过 localStorage 传入时，两边仍逐 tick 一致", () => {
    // Arrange
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "2");
    const budget = { ...DEFAULT_REVEAL_BUDGET, charsPerTick: 2 };
    const fullContent = "y".repeat(60);
    const messages = ref<ChatMessage[]>([createAssistantMessage(fullContent.slice(0, 3))]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    let shadow = INITIAL_REVEAL_STATE;
    const observed: Array<[number, number]> = [];

    // Act
    for (const length of [3, 6, 30, 60]) {
      messages.value = [createAssistantMessage(fullContent.slice(0, length))];
      state.syncStreamingPresentationState();
      shadow = stepReveal(shadow, { incomingLength: length }, budget);
      observed.push([state.assistantDisplayContent(messages.value[0]!).length, shadow.visible]);
    }

    // Assert
    expect(observed).toEqual(observed.map(([hook]) => [hook, hook]));
  });
});

describe("PDS-SM 接入 · A4b：黄金表（独立于新实现的特征化断言）", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  /**
   * 这组期望值按**重构前**（`git show HEAD:src/lib/useStreamingPresentationState.ts`）的算法手算：
   * 每 tick 释放 4；积压 > 120 时额外释放 floor(积压/8)；同一长度重复时累计停滞计数。
   *
   * 之所以必须独立于 `stepReveal` 写死：A4 的对拍两边同源（hook 现在就是调 reducer），
   * 只能证明"接线没错"，无法证伪"算法被改"。本用例是唯一能抓住算法漂移的判据。
   */
  const GOLDEN: Array<{ incoming: number; visible: number }> = [
    { incoming: 4, visible: 4 },
    { incoming: 8, visible: 8 },
    { incoming: 12, visible: 12 },
    { incoming: 50, visible: 16 },
    { incoming: 50, visible: 20 },
    { incoming: 50, visible: 24 },
    { incoming: 200, visible: 50 }, // 积压 176 > 120 → +floor(176/8)=22
    { incoming: 200, visible: 72 },
    { incoming: 210, visible: 93 },
    { incoming: 210, visible: 97 } // 积压 117 ≤ 120 → 不再加速
  ];

  it("固定内容序列下逐 tick 释放长度与重构前一致", () => {
    // Arrange
    const fullContent = "g".repeat(210);
    const messages = ref<ChatMessage[]>([createAssistantMessage(fullContent.slice(0, GOLDEN[0]!.incoming))]);
    const state = useStreamingPresentationState(computed(() => messages.value));
    const actual: number[] = [];

    // Act
    for (const step of GOLDEN) {
      messages.value = [createAssistantMessage(fullContent.slice(0, step.incoming))];
      state.syncStreamingPresentationState();
      actual.push(state.assistantDisplayContent(messages.value[0]!).length);
    }

    // Assert
    expect(actual).toEqual(GOLDEN.map((step) => step.visible));
  });
  it("自定义速率下同样成立（覆盖加速档）", () => {
    // Arrange：速率 2 + 一次大积压，期望值同样按重构前算法手算
    //   2（配额 2）→ 4（配额 2）→ 6（配额 2）→ 32（积压 194 > 120，+floor(194/8)=24）
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "2");
    const fullContent = "k".repeat(200);
    const plan = [
      { incoming: 10, visible: 2 },
      { incoming: 10, visible: 4 },
      { incoming: 10, visible: 6 },
      { incoming: 200, visible: 32 }
    ];
    const messages = ref<ChatMessage[]>([createAssistantMessage(fullContent.slice(0, plan[0]!.incoming))]);
    const state = useStreamingPresentationState(computed(() => messages.value));
    const actual: number[] = [];

    // Act
    for (const step of plan) {
      messages.value = [createAssistantMessage(fullContent.slice(0, step.incoming))];
      state.syncStreamingPresentationState();
      actual.push(state.assistantDisplayContent(messages.value[0]!).length);
    }

    // Assert
    expect(actual).toEqual(plan.map((step) => step.visible));
  });
});

describe("PDS-SM 接入 · A4c：无变化时不得产生多余响应式失效", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("已追平且内容未变时，再次 sync 不重算下游 computed", () => {
    // Arrange：4 字符，一次 sync 即追平（配额 4）
    const messages = ref<ChatMessage[]>([createAssistantMessage("abcd")]);
    const state = useStreamingPresentationState(computed(() => messages.value));
    state.syncStreamingPresentationState();
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe("abcd");

    let evaluations = 0;
    const display = computed(() => {
      evaluations += 1;
      return state.assistantDisplayContent(messages.value[0]!);
    });
    expect(display.value).toBe("abcd");
    const baseline = evaluations;

    // Act：内容与游标都没变化
    state.syncStreamingPresentationState();

    // Assert：computed 未被失效，读取命中缓存（评估次数不增）
    expect(display.value).toBe("abcd");
    expect(evaluations).toBe(baseline);
  });
});

describe("PDS-SM 接入 · A4d：截断后再次增长（游标不下调语义）", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  /**
   * 期望值同样按**重构前**算法手算（旧实现把收缩后的值只在本地夹取用于显示，
   * 不回写游标）：因此内容被置空后再增长时，会瞬间显示到收缩前的进度。
   *
   * 这是生产可达路径：`src/stores/runtime.ts` 的超时重连分支会把 pending 消息的
   * content 置为 `""`（`TIMEOUT_RETRY_PENDING_MESSAGE` 判定处）。
   */
  const GOLDEN_TRUNCATION: Array<{ incoming: number; visible: number }> = [
    { incoming: 60, visible: 4 },
    { incoming: 60, visible: 8 },
    { incoming: 60, visible: 12 },
    { incoming: 0, visible: 0 }, // 截断：光标不下调，但显示由 slice 自然截断为空
    { incoming: 11, visible: 11 }, // 再增长：瞬间显示全部 11 字符（而非从 0 逐字重打）
    { incoming: 60, visible: 16 }
  ];

  it("截断置空后再增长，显示立即追平收缩前进度", () => {
    // Arrange
    const fullContent = "T".repeat(60);
    const messages = ref<ChatMessage[]>([createAssistantMessage(fullContent)]);
    const state = useStreamingPresentationState(computed(() => messages.value));
    const actual: number[] = [];

    // Act
    for (const step of GOLDEN_TRUNCATION) {
      messages.value = [createAssistantMessage(fullContent.slice(0, step.incoming))];
      state.syncStreamingPresentationState();
      actual.push(state.assistantDisplayContent(messages.value[0]!).length);
    }

    // Assert
    expect(actual).toEqual(GOLDEN_TRUNCATION.map((step) => step.visible));
  });
});

describe("PDS-SM 接入 · A6：非法释放速率的刻意偏离", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("速率写成 0.5 时按 1 释放（旧实现会算出 0 并停滞约 2s）", () => {
    // Arrange：旧实现 Number("0.5") → floor → 0，游标恒为 0 直到停滞兜底触发
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "0.5");
    const messages = ref<ChatMessage[]>([createAssistantMessage("0123456789")]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    // Act
    state.syncStreamingPresentationState();
    const first = state.assistantDisplayContent(messages.value[0]!).length;
    state.syncStreamingPresentationState();
    const second = state.assistantDisplayContent(messages.value[0]!).length;

    // Assert：必须持续推进（夹到 >=1），而不是卡在 0
    expect(first).toBe(1);
    expect(second).toBe(2);
  });
});

describe("PDS-SM 接入 · A5：降级路径", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("reduced motion 下 hook 与 reducer 的 reduceMotion 分支输出一致（一次性全量）", () => {
    // Arrange
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: true }));
    const content = "z".repeat(300);
    const messages = ref<ChatMessage[]>([createAssistantMessage(content)]);
    const state = useStreamingPresentationState(computed(() => messages.value));

    // Act
    state.syncStreamingPresentationState();

    // Assert：hook 一次性给全文；reducer 的 reduceMotion 分支同样一步到 incoming
    expect(state.assistantDisplayContent(messages.value[0]!)).toBe(content);
    expect(stepReveal(INITIAL_REVEAL_STATE, { incomingLength: 300, reduceMotion: true }).visible).toBe(300);
  });
});

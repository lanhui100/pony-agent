/**
 * PDS-SM L4 逻辑层 A：字符释放预算（budget.ts）
 *
 * 分级：L1 纯单测（无 IO / 无时钟 / 无 DOM），按 test-expert 规范补对抗畸形输入。
 * 不变量来源：`docs/decisions/0057-streaming-motion-framework-agnostic-spec.md`。
 */
import { describe, expect, it } from "vitest";
import {
  DEFAULT_REVEAL_BUDGET,
  INITIAL_REVEAL_STATE,
  revealQuota,
  stepReveal,
  visiblePrefix,
  type RevealBudgetConfig,
  type RevealState
} from "../.agents/skills/streaming-motion/logic/budget";

function run(
  state: RevealState,
  ticks: Array<number>,
  config?: RevealBudgetConfig,
  reduceMotion = false
): RevealState {
  return ticks.reduce<RevealState>(
    (acc, incomingLength) => stepReveal(acc, { incomingLength, reduceMotion }, config),
    state
  );
}

describe("PDS-SM budget：基础释放", () => {
  it("首 tick 从 0 开始，只释放一个基础配额", () => {
    // Arrange
    const state = INITIAL_REVEAL_STATE;

    // Act
    const next = stepReveal(state, { incomingLength: 10 });

    // Assert
    expect(next.visible).toBe(DEFAULT_REVEAL_BUDGET.charsPerTick);
    expect(next.lastIncomingLength).toBe(10);
  });

  it("已追平后继续喂入相同长度：游标不动、停滞计数归零", () => {
    // Arrange
    const caughtUp: RevealState = { visible: 10, lastIncomingLength: 10, stallTicks: 5 };

    // Act
    const next = stepReveal(caughtUp, { incomingLength: 10 });

    // Assert
    expect(next).toEqual({ visible: 10, lastIncomingLength: 10, stallTicks: 0 });
  });

  it("不越过入参长度：即使配额大于积压也只收敛到 incoming", () => {
    // Arrange
    const state: RevealState = { visible: 0, lastIncomingLength: 0, stallTicks: 0 };

    // Act
    const next = stepReveal(state, { incomingLength: 2 });

    // Assert
    expect(next.visible).toBe(2);
  });
});

describe("PDS-SM budget：单调性与不变量", () => {
  it("30 tick 线性增长下：visible 单调不减且恒不越界", () => {
    // Arrange
    let state = INITIAL_REVEAL_STATE;
    const observed: Array<{ visible: number; incoming: number }> = [];

    // Act
    for (let tick = 1; tick <= 30; tick += 1) {
      const incoming = tick * 7;
      state = stepReveal(state, { incomingLength: incoming });
      observed.push({ visible: state.visible, incoming });
    }

    // Assert
    for (let i = 1; i < observed.length; i += 1) {
      expect(observed[i]!.visible).toBeGreaterThanOrEqual(observed[i - 1]!.visible);
    }
    for (const sample of observed) {
      expect(sample.visible).toBeLessThanOrEqual(sample.incoming);
      expect(sample.visible).toBeGreaterThanOrEqual(0);
    }
  });

  it("content 变短（reset / 截断）：游标跟随收缩，不抛错也不残留越界值", () => {
    // Arrange
    const state: RevealState = { visible: 100, lastIncomingLength: 100, stallTicks: 0 };

    // Act
    const next = stepReveal(state, { incomingLength: 40 });

    // Assert
    expect(next.visible).toBe(40);
    expect(next.lastIncomingLength).toBe(40);
  });
});

describe("PDS-SM budget：积压加速", () => {
  it("积压未达阈值时不加速（阈值是严格大于关系）", () => {
    // Arrange
    const half = DEFAULT_REVEAL_BUDGET.backlogThreshold;

    // Act + Assert
    expect(revealQuota(half)).toBe(DEFAULT_REVEAL_BUDGET.charsPerTick);
    expect(revealQuota(half + 1)).toBe(
      DEFAULT_REVEAL_BUDGET.charsPerTick + Math.floor((half + 1) / DEFAULT_REVEAL_BUDGET.backlogDivisor)
    );
  });

  it("积压越过阈值后配额随积压线性上升", () => {
    // Arrange
    const small = revealQuota(200);
    const large = revealQuota(800);

    // Act + Assert
    expect(small).toBeGreaterThan(DEFAULT_REVEAL_BUDGET.charsPerTick);
    expect(large).toBeGreaterThan(small);
  });

  it("零积压时配额为 0，不产生空转推进", () => {
    expect(revealQuota(0)).toBe(0);
  });
});

describe("PDS-SM budget：停滞收敛", () => {
  it("同一入参长度连续到达 stallTicks 次后一次性释放全部", () => {
    // Arrange：配额压到 1，确保 3 个 tick 内追不平，才能观察停滞分支
    const config: RevealBudgetConfig = { charsPerTick: 1, backlogThreshold: 10_000, backlogDivisor: 8, stallTicks: 3 };
    let state: RevealState = { visible: 0, lastIncomingLength: 50, stallTicks: 0 };

    // Act
    state = stepReveal(state, { incomingLength: 50 }, config);
    expect(state.visible).toBe(1);
    expect(state.stallTicks).toBe(1);

    state = stepReveal(state, { incomingLength: 50 }, config);
    expect(state.visible).toBe(2);
    expect(state.stallTicks).toBe(2);

    const flushed = stepReveal(state, { incomingLength: 50 }, config);

    // Assert
    expect(flushed.visible).toBe(50);
    expect(flushed.stallTicks).toBe(0);
  });

  it("内容仍在增长时停滞计数被清零，不会误触发全量释放", () => {
    // Arrange
    const config: RevealBudgetConfig = { charsPerTick: 1, backlogThreshold: 10_000, backlogDivisor: 8, stallTicks: 3 };
    let state: RevealState = { visible: 0, lastIncomingLength: 0, stallTicks: 0 };

    // Act
    state = stepReveal(state, { incomingLength: 10 }, config);
    state = stepReveal(state, { incomingLength: 11 }, config);
    state = stepReveal(state, { incomingLength: 12 }, config);

    // Assert
    expect(state.stallTicks).toBe(0);
    expect(state.visible).toBeLessThan(12);
  });
});

describe("PDS-SM budget：降级", () => {
  it("reduceMotion 为 true 时一步释放全部字符", () => {
    // Arrange
    const state = INITIAL_REVEAL_STATE;

    // Act
    const next = stepReveal(state, { incomingLength: 5_000, reduceMotion: true });

    // Assert
    expect(next.visible).toBe(5_000);
    expect(next.stallTicks).toBe(0);
  });

  it("reduceMotion 也遵守 content 收缩的安全边界", () => {
    // Arrange
    const state: RevealState = { visible: 300, lastIncomingLength: 300, stallTicks: 2 };

    // Act
    const next = stepReveal(state, { incomingLength: 12, reduceMotion: true });

    // Assert
    expect(next.visible).toBe(12);
  });
});

describe("PDS-SM budget：对抗畸形输入", () => {
  it.each([
    ["NaN", Number.NaN],
    ["负数", -42],
    ["Infinity", Number.POSITIVE_INFINITY],
    ["-Infinity", Number.NEGATIVE_INFINITY]
  ])("长度为 %s 时整帧不推进，且绝不回退已显示游标", (_label, incomingLength) => {
    // Arrange：已显示 5 个字符、停滞计数为 2
    const state: RevealState = { visible: 5, lastIncomingLength: 5, stallTicks: 2 };

    // Act
    const next = stepReveal(state, { incomingLength });

    // Assert：游标与停滞计数原样保留（把非法长度当 0 会让正文整段消失再重打）
    expect(next.visible).toBe(5);
    expect(next.lastIncomingLength).toBe(5);
    expect(next.stallTicks).toBe(2);
  });

  it("小数长度向下取整后正常推进", () => {
    // Arrange
    const state: RevealState = { visible: 0, lastIncomingLength: 0, stallTicks: 0 };

    // Act
    const next = stepReveal(state, { incomingLength: 3.7 });

    // Assert
    expect(next.visible).toBe(3);
    expect(Number.isInteger(next.visible)).toBe(true);
  });

  it("配置下界被夹取：charsPerTick 0 → 1，stallTicks 0 → 1（下限语义可观测）", () => {
    // Arrange
    const clamped: RevealBudgetConfig = {
      charsPerTick: 0,
      backlogThreshold: 10_000,
      backlogDivisor: 8,
      stallTicks: 0
    };

    // Act + Assert：配额被夹到 1（而非 0 ⇒ 永久停滞）
    expect(revealQuota(100, clamped)).toBe(1);

    // Act：同一长度重复一次即达夹取后的 stallTicks 下限 1 ⇒ 一步收敛
    const once = stepReveal({ visible: 0, lastIncomingLength: 50, stallTicks: 0 }, { incomingLength: 50 }, clamped);

    // Assert
    expect(once.visible).toBe(50);
  });
});

describe("PDS-SM budget：可见前缀选择器", () => {
  it("按游标截断，且入参 content 短于游标时不越界", () => {
    // Arrange
    const state: RevealState = { visible: 4, lastIncomingLength: 10, stallTicks: 0 };

    // Act + Assert
    expect(visiblePrefix("abcdefghij", state)).toBe("abcd");
    expect(visiblePrefix("ab", state)).toBe("ab");
    expect(visiblePrefix("abcdefghij", { ...state, visible: -3 })).toBe("");
  });

  it("多 tick 累积后的前缀与游标严格一致，最终收敛为全文", () => {
    // Arrange
    const content = "流式正文用于验证可见前缀与游标严格一致";
    let state = INITIAL_REVEAL_STATE;

    // Act：先推进 3 tick（此时必然只露出前缀）
    state = run(state, [content.length, content.length, content.length]);

    // Assert：前缀与游标严格对齐，且尚未追平
    expect(visiblePrefix(content, state)).toBe(content.slice(0, state.visible));
    expect(state.visible).toBeLessThan(content.length);

    // Act：继续推进到追平
    state = run(state, Array.from({ length: content.length }, () => content.length));

    // Assert：收敛为全文
    expect(visiblePrefix(content, state)).toBe(content);
  });
});

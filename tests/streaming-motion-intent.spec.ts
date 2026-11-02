/**
 * PDS-SM L4 逻辑层 B：滚动意图状态机（intent.ts）
 *
 * 分级：L1 纯单测（无 DOM / 无时钟 / 无监听），按 test-expert 规范补对抗畸形输入。
 * 语义来源：`docs/decisions/0057-streaming-motion-framework-agnostic-spec.md`。
 */
import { describe, expect, it } from "vitest";
import {
  DEFAULT_SCROLL_INTENT,
  INITIAL_SCROLL_INTENT_STATE,
  reduceScrollIntent,
  type ScrollIntentConfig,
  type ScrollIntentState
} from "../.agents/skills/streaming-motion/logic/intent";

const FAR = 5_000;
const NEAR = 10;

function append(state: ScrollIntentState, atMs: number, distanceToBottom = FAR, config?: ScrollIntentConfig) {
  return reduceScrollIntent(state, { type: "content-appended", distanceToBottom, atMs }, config);
}

function gesture(state: ScrollIntentState, atMs: number, config?: ScrollIntentConfig) {
  return reduceScrollIntent(state, { type: "user-gesture", kind: "wheel", atMs }, config);
}

describe("PDS-SM intent：默认参数", () => {
  it("默认容差与冷却取本仓校准值，而非上游常量（40px / 150ms）", () => {
    expect(DEFAULT_SCROLL_INTENT.pinTolerancePx).toBe(260);
    expect(DEFAULT_SCROLL_INTENT.inputCooldownMs).toBe(150);
    expect(DEFAULT_SCROLL_INTENT.idleResumeMs).toBe(3000);
  });
});

describe("PDS-SM intent：跟随与释放", () => {
  it("初始为跟随态时，内容追加触发 follow", () => {
    // Act
    const decision = append(INITIAL_SCROLL_INTENT_STATE, 1_000);

    // Assert
    expect(decision.action).toBe("follow");
    expect(decision.state.pinned).toBe(true);
  });

  it("用户物理输入立即释放跟随，并要求撤销挂起的程序化滚动", () => {
    // Act
    const decision = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000);

    // Assert
    expect(decision.action).toBe("release");
    expect(decision.state.pinned).toBe(false);
    expect(decision.state.lastUserInputAtMs).toBe(2_000);
  });

  it("冷却窗口内到达的内容绝不夺回视口", () => {
    // Arrange
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;

    // Act
    const decision = append(released, 2_100);

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
    expect(decision.state.contentChangedSincePause).toBe(true);
  });

  it("冷却边界为闭区间：恰好 inputCooldownMs 仍被拒绝，超出 1ms 即放行", () => {
    // Arrange：用户已滚回贴底，唯一阻止跟随的因素只剩冷却窗口
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;
    const atBoundary = 2_000 + DEFAULT_SCROLL_INTENT.inputCooldownMs;

    // Act + Assert
    expect(append(released, atBoundary, NEAR).action).toBe("none");
    expect(append(released, atBoundary + 1, NEAR).action).toBe("follow");
  });

  it("冷却结束后若仍远离底部，依旧不跟随", () => {
    // Arrange
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;

    // Act
    const decision = append(released, 9_000, FAR);

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
  });
});

describe("PDS-SM intent：贴底重新吸附", () => {
  it("viewport-scrolled 回读距底在容差内 → 重新 pinned", () => {
    // Arrange
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;

    // Act
    const decision = reduceScrollIntent(released, { type: "viewport-scrolled", distanceToBottom: NEAR });

    // Assert
    expect(decision.state.pinned).toBe(true);
    expect(decision.state.contentChangedSincePause).toBe(false);
    expect(decision.action).toBe("none");
  });

  it("viewport-scrolled 距底超容差 → 解除 pinned", () => {
    // Act
    const decision = reduceScrollIntent(INITIAL_SCROLL_INTENT_STATE, {
      type: "viewport-scrolled",
      distanceToBottom: DEFAULT_SCROLL_INTENT.pinTolerancePx + 1
    });

    // Assert
    expect(decision.state.pinned).toBe(false);
  });

  it("容差边界为闭区间：恰好等于 pinTolerancePx 视为贴底", () => {
    // Act
    const decision = reduceScrollIntent(INITIAL_SCROLL_INTENT_STATE, {
      type: "viewport-scrolled",
      distanceToBottom: DEFAULT_SCROLL_INTENT.pinTolerancePx
    });

    // Assert
    expect(decision.state.pinned).toBe(true);
  });

  it("未 pinned 时用户滚回贴底，下一个内容追加恢复 follow", () => {
    // Arrange
    let state = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;
    state = reduceScrollIntent(state, { type: "viewport-scrolled", distanceToBottom: NEAR }).state;

    // Act
    const decision = append(state, 9_000, NEAR);

    // Assert
    expect(decision.action).toBe("follow");
  });

  it("未 pinned 且远离底部时，即使内容继续增长也不吸附", () => {
    // Arrange
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;

    // Act
    const decision = append(released, 9_000, FAR + 1);

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
  });
});

describe("PDS-SM intent：空闲恢复", () => {
  function pausedWithGrowth(): ScrollIntentState {
    let state = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;
    state = append(state, 2_100, FAR).state;
    return state;
  }

  it("静默超时且期间有内容增长 → 恢复跟随", () => {
    // Arrange
    const state = pausedWithGrowth();
    expect(state.contentChangedSincePause).toBe(true);

    // Act
    const decision = reduceScrollIntent(state, {
      type: "idle-tick",
      atMs: 2_000 + DEFAULT_SCROLL_INTENT.idleResumeMs
    });

    // Assert
    expect(decision.action).toBe("follow");
    expect(decision.state.pinned).toBe(true);
    expect(decision.state.contentChangedSincePause).toBe(false);
  });

  it("静默未达阈值 → 不恢复（用户可能仍在缓慢阅读）", () => {
    // Arrange
    const state = pausedWithGrowth();

    // Act
    const decision = reduceScrollIntent(state, {
      type: "idle-tick",
      atMs: 2_000 + DEFAULT_SCROLL_INTENT.idleResumeMs - 1
    });

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
  });

  it("静默超时但期间没有新内容 → 不恢复（无新内容就没有夺回视口的理由）", () => {
    // Arrange：只发生过手势，从未有内容增长
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;
    expect(released.contentChangedSincePause).toBe(false);

    // Act
    const decision = reduceScrollIntent(released, {
      type: "idle-tick",
      atMs: 2_000 + DEFAULT_SCROLL_INTENT.idleResumeMs * 10
    });

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
  });

  it("已处于 pinned 时 idle-tick 不产生多余 follow", () => {
    // Act
    const decision = reduceScrollIntent(INITIAL_SCROLL_INTENT_STATE, { type: "idle-tick", atMs: 9_999 });

    // Assert
    expect(decision.action).toBe("none");
  });

  it("回归：从未观测到物理输入（例如只拖滚动条）时永不自动恢复", () => {
    // Arrange：viewport-scrolled 只改变 pinned，不产生 lastUserInputAtMs
    let state = reduceScrollIntent(INITIAL_SCROLL_INTENT_STATE, {
      type: "viewport-scrolled",
      distanceToBottom: FAR
    }).state;
    expect(state.pinned).toBe(false);
    expect(state.lastUserInputAtMs).toBe(Number.NEGATIVE_INFINITY);
    state = append(state, 1_000, FAR).state;

    // Act：任意长的静默都不应让状态机夺回视口
    const decision = reduceScrollIntent(state, { type: "idle-tick", atMs: 10_000_000 });

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
  });

  it("回归：idle-tick 不清除 content-appended 累积的增长标志", () => {
    // Arrange
    const state = pausedWithGrowth();

    // Act：未达阈值的一次心跳
    const early = reduceScrollIntent(state, { type: "idle-tick", atMs: 2_200 });

    // Assert：状态机只读该标志，不写回
    expect(early.action).toBe("none");
    expect(early.state.contentChangedSincePause).toBe(true);
  });
});

describe("PDS-SM intent：完整场景序列", () => {
  it("跟随 → 用户上滚 → 冷却保护 → 滚回底部 → 恢复跟随", () => {
    // Arrange
    let state = INITIAL_SCROLL_INTENT_STATE;
    let clock = 1_000;
    const actions: string[] = [];

    // Act：1) 正常跟随
    let decision = append(state, clock, FAR);
    actions.push(decision.action);
    state = decision.state;

    // 2) 用户滚轮 → 释放
    clock += 60;
    decision = gesture(state, clock);
    actions.push(decision.action);
    state = decision.state;

    // 3) 冷却期内的 3 次内容追加全部被拒绝
    for (let i = 0; i < 3; i += 1) {
      clock += 40;
      decision = append(state, clock, FAR);
      actions.push(decision.action);
      state = decision.state;
    }

    // 4) 用户滚回底部
    decision = reduceScrollIntent(state, { type: "viewport-scrolled", distanceToBottom: NEAR });
    state = decision.state;

    // 5) 下一次追加恢复跟随
    clock += 40;
    decision = append(state, clock, NEAR);
    actions.push(decision.action);

    // Assert
    expect(actions).toEqual(["follow", "release", "none", "none", "none", "follow"]);
    expect(decision.state.pinned).toBe(true);
  });
});

describe("PDS-SM intent：对抗畸形输入", () => {
  it.each([
    ["NaN", Number.NaN],
    ["-Infinity", Number.NEGATIVE_INFINITY],
    ["Infinity", Number.POSITIVE_INFINITY]
  ])("viewport-scrolled 的 distanceToBottom 为 %s 时不抛错且不误判贴底", (_label, distanceToBottom) => {
    // Act
    const decision = reduceScrollIntent(INITIAL_SCROLL_INTENT_STATE, { type: "viewport-scrolled", distanceToBottom });

    // Assert
    expect(decision.state.pinned).toBe(false);
    expect(decision.action).toBe("none");
  });

  it("content-appended 的距离为 NaN 时不吸附、不抛错", () => {
    // Arrange
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;

    // Act
    const decision = append(released, 9_000, Number.NaN);

    // Assert
    expect(decision.action).toBe("none");
    expect(decision.state.pinned).toBe(false);
  });

  it("content-appended 的时间戳不可用时 fail-closed：判为冷却中，不移动视口", () => {
    // Arrange：已脱离跟随，且**贴底**——若走进非冷却分支会返回 follow，足以区分两条分支
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;
    expect(released.pinned).toBe(false);

    // Act
    const decision = append(released, Number.NaN, NEAR);

    // Assert：-Infinity 时间戳 ⇒ 差值 -Infinity ≤ 冷却窗口 ⇒ 冷却分支 ⇒ 不跟随
    expect(decision.action).toBe("none");

    // 对照：同一初态 + 合法且远过冷却的时间戳，必须能跟随（证明上面的 none 不是恒真）
    expect(append(released, 2_000 + 10_000, NEAR).action).toBe("follow");
  });

  it("畸形配置被夹到合法下限：pinTolerancePx 为 NaN 时回落默认值，仍能识别贴底", () => {
    // Arrange
    const broken: ScrollIntentConfig = {
      pinTolerancePx: Number.NaN,
      inputCooldownMs: -5,
      idleResumeMs: Number.POSITIVE_INFINITY
    };
    const released = gesture(INITIAL_SCROLL_INTENT_STATE, 2_000).state;

    // Act：距底 10px。若 NaN 传播进 isNearBottom，比较恒为 false ⇒ 返回 none
    const decision = append(released, 2_000 + 10_000, NEAR, broken);

    // Assert
    expect(decision.action).toBe("follow");
    expect(decision.state.pinned).toBe(true);
  });
});

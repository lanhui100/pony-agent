/**
 * Pony Design System — Streaming Motion (PDS-SM)
 * L4 逻辑层 B：滚动意图（Scroll Intent）
 *
 * 纯函数状态机。不读 DOM、不挂监听、不读当前时钟 —— 时间一律由事件携带。
 * 宿主的职责只有三件事：把真实事件翻译成本文件的 ScrollEvent、执行返回的 action、
 * 忽略自己触发的滚动事件（否则 `viewport-scrolled` 会自我确认）。
 *
 * 解决的问题：流式输出每 60ms 追加一次内容。若每 tick 都无条件把视口拉到底，
 * 用户向上翻历史时会被反复拽回 —— 这是"操作对抗"，不是性能问题。
 * 规则：用户的物理输入永远优先，只有"贴底 + 已过冷却窗口"才允许程序化跟随。
 */

export type UserGestureKind = "wheel" | "touchstart" | "pointerdown" | "keydown";

export interface ScrollIntentConfig {
  /** 距底容差（px）。小于等于该值视为"贴底"，可重新吸附。 */
  pinTolerancePx: number;
  /** 用户物理输入后的冷却窗口（ms）。窗口内禁止任何程序化滚动。 */
  inputCooldownMs: number;
  /** 用户暂停跟随后，静止多久且期间有内容增长，才自动恢复跟随（ms）。 */
  idleResumeMs: number;
}

/**
 * 默认值取本仓实测校准值，而非上游常量：
 * - `pinTolerancePx` 260：上游 Claude 用 40px，本仓 wave-14 实测 40px 在长回复下
 *   频繁误判"已离开底部"，并产生 ±1 行的方向反转抖动；
 * - `inputCooldownMs` 150：与上游一致，实测足够；
 * - `idleResumeMs` 3000：上游无此机制（只有 150ms 去抖），本仓实测需要更长的
 *   静默期才不会在用户缓慢阅读时夺回视口。
 */
export const DEFAULT_SCROLL_INTENT: ScrollIntentConfig = {
  pinTolerancePx: 260,
  inputCooldownMs: 150,
  idleResumeMs: 3000
};

export type ScrollEvent =
  | {
      /** 内容增长（新 token 落地）。`atMs` 由宿主提供，用于冷却判定。 */
      type: "content-appended";
      distanceToBottom: number;
      atMs: number;
    }
  | {
      /** 用户的物理输入。必须先于任何内容追加事件被投递。 */
      type: "user-gesture";
      kind: UserGestureKind;
      atMs: number;
    }
  | {
      /** 视口滚动落地后回读到的距底距离。宿主必须忽略自己发起的滚动。 */
      type: "viewport-scrolled";
      distanceToBottom: number;
    }
  | {
      /**
       * 空闲心跳。用于超时后自动恢复跟随。
       *
       * 是否需要恢复完全由 `state` 决定（`pinned` / `lastUserInputAtMs` / `contentChangedSincePause`），
       * 事件本身只携带时钟。这样不存在"事件字段覆盖状态"的分叉。
       */
      type: "idle-tick";
      atMs: number;
    };

export interface ScrollIntentState {
  /** 是否处于"跟随最新"状态。 */
  pinned: boolean;
  /** 最近一次用户物理输入的时间戳（ms）。`-Infinity` 表示**从未**观测到物理输入。 */
  lastUserInputAtMs: number;
  /** 解除跟随后，期间是否发生过内容增长。由 `content-appended` 维护，决定 idle 是否恢复。 */
  contentChangedSincePause: boolean;
}

export const INITIAL_SCROLL_INTENT_STATE: ScrollIntentState = {
  pinned: true,
  lastUserInputAtMs: Number.NEGATIVE_INFINITY,
  contentChangedSincePause: false
};

export type ScrollAction = "follow" | "release" | "none";

export interface ScrollIntentDecision {
  state: ScrollIntentState;
  /** `follow` = 把视口对齐到最新；`release` = 撤销挂起的程序化滚动；`none` = 什么都不做。 */
  action: ScrollAction;
}

export function reduceScrollIntent(
  state: ScrollIntentState,
  event: ScrollEvent,
  config: ScrollIntentConfig = DEFAULT_SCROLL_INTENT
): ScrollIntentDecision {
  const cfg = sanitizeConfig(config);

  switch (event.type) {
    case "user-gesture":
      // 物理输入永远优先：立刻释放跟随，语义上等价于"撤销挂起的程序化滚动"。
      return {
        state: {
          pinned: false,
          lastUserInputAtMs: toTimestamp(event.atMs),
          contentChangedSincePause: false
        },
        action: "release"
      };

    case "viewport-scrolled": {
      const pinned = isNearBottom(event.distanceToBottom, cfg.pinTolerancePx);
      return {
        state: {
          ...state,
          pinned,
          contentChangedSincePause: pinned ? false : state.contentChangedSincePause
        },
        action: "none"
      };
    }

    case "content-appended": {
      // 时间戳不可用（NaN / ±Infinity）时退化为 -Infinity：差值会成为 -Infinity，
      // 必然 <= 冷却窗口 ⇒ 判为"冷却中"⇒ 不移动视口。这是有意的 fail-closed：
      // 宁可不跟随，也不在一次可能正在发生的用户操作中抢夺视口。
      const inCooldown = toTimestamp(event.atMs) - state.lastUserInputAtMs <= cfg.inputCooldownMs;
      if (inCooldown) {
        return {
          state: { ...state, contentChangedSincePause: true },
          action: "none"
        };
      }

      if (state.pinned) {
        return { state, action: "follow" };
      }

      // 未 pinned：只有用户已经滚回贴底边缘才重新吸附并跟随。
      if (isNearBottom(event.distanceToBottom, cfg.pinTolerancePx)) {
        return {
          state: { ...state, pinned: true, contentChangedSincePause: false },
          action: "follow"
        };
      }

      return {
        state: { ...state, contentChangedSincePause: true },
        action: "none"
      };
    }

    case "idle-tick": {
      if (state.pinned) {
        return { state, action: "none" };
      }

      // 从未观测到物理输入（例如用户只用滚动条拖动，只产生 viewport-scrolled）时，
      // 我们无从得知他何时停手 —— 此时**宁可永不自动恢复**，也不能夺回视口。
      // 差值语义：lastUserInputAtMs 为 -Infinity 时 atMs - (-Inf) = +Inf，若不显式拦截
      // 会让首个 idle-tick 立刻判为"已静默超时"（这正是修复前的缺陷）。
      if (!Number.isFinite(state.lastUserInputAtMs)) {
        return { state, action: "none" };
      }

      const idle = toTimestamp(event.atMs) - state.lastUserInputAtMs;
      if (state.contentChangedSincePause && idle >= cfg.idleResumeMs) {
        return {
          state: { ...state, pinned: true, contentChangedSincePause: false },
          action: "follow"
        };
      }
      return { state, action: "none" };
    }

    default: {
      // 穷尽性兜底：新增事件类型若未在 switch 内处理，赋值给 never 会在编译期报错。
      // 运行期保持宽容（非法事件不改状态、不抛错），避免一条脏事件打断整个流。
      const unreachable: never = event;
      void unreachable;
      return { state, action: "none" };
    }
  }
}

function toTimestamp(value: number): number {
  return Number.isFinite(value) ? value : Number.NEGATIVE_INFINITY;
}

function isNearBottom(distanceToBottom: number, tolerance: number): boolean {
  if (!Number.isFinite(distanceToBottom)) return false;
  return distanceToBottom <= tolerance;
}

function sanitizeConfig(config: ScrollIntentConfig): ScrollIntentConfig {
  return {
    pinTolerancePx: Math.max(0, toFiniteOr(config.pinTolerancePx, DEFAULT_SCROLL_INTENT.pinTolerancePx)),
    inputCooldownMs: Math.max(0, toFiniteOr(config.inputCooldownMs, DEFAULT_SCROLL_INTENT.inputCooldownMs)),
    idleResumeMs: Math.max(0, toFiniteOr(config.idleResumeMs, DEFAULT_SCROLL_INTENT.idleResumeMs))
  };
}

function toFiniteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

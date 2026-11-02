/**
 * Pony Design System — Streaming Motion (PDS-SM)
 * L4 逻辑层 A：字符释放预算（Reveal Budget）
 *
 * 纯函数。不 import 任何框架、不读 DOM、不取时钟 —— 所有时间由调用方 tick 驱动。
 * 因此可在任意框架、任意测试环境下逐分支验证。
 *
 * 解决的问题：LLM 的到达速率不可控（几 tok/s 到数百 tok/s）。直接渲染 content
 * 会让文字以肉眼可见的块状"跳"出来；完全逐字又会滞后于真实到达。这里用一个
 * 有上界的追赶游标：每 tick 释放固定配额，积压过多时线性加速，停滞过久则一次性收敛。
 */

/** 释放预算参数。改动任一字段都要重跑 tests/streaming-motion-budget.spec.ts。 */
export interface RevealBudgetConfig {
  /** 每 tick 的基础释放字符数。 */
  charsPerTick: number;
  /** 积压达到该字符数后才开始加速。低于它时不加速，避免短视频闪烁。 */
  backlogThreshold: number;
  /** 加速系数除数：额外释放 floor(backlog / backlogDivisor)。分母越大加速越温和。 */
  backlogDivisor: number;
  /** 内容连续未增长的 tick 数达到该值即视为流已停滞，一次性释放全部。 */
  stallTicks: number;
}

export const DEFAULT_REVEAL_BUDGET: RevealBudgetConfig = {
  charsPerTick: 4,
  backlogThreshold: 120,
  backlogDivisor: 8,
  stallTicks: 34
};

/** 单条流式消息的释放游标。 */
export interface RevealState {
  /**
   * 已释放到第几个字符（含），即可见前缀长度。
   *
   * 不变量归属：`visible <= incomingLength` 由 `stepReveal` 的**输出**保证。
   * 宿主可以在同一结构里保存**高于**当前长度的"高水位"游标（例如内容被重置为空后
   * 不希望重新逐字打一遍），再原样喂回 `stepReveal` —— 函数会自行夹取，不会越界，
   * 也不会把高水位丢掉。直接按本字段裁剪文本的消费者仍须自己 clamp（用 `visiblePrefix`）。
   */
  visible: number;
  /** 上一 tick 观察到的入参长度，用于停滞检测。`-1` 表示尚未观测过。 */
  lastIncomingLength: number;
  /** 内容长度连续未增长的 tick 计数。 */
  stallTicks: number;
}

export const INITIAL_REVEAL_STATE: RevealState = {
  visible: 0,
  lastIncomingLength: -1,
  stallTicks: 0
};

export interface RevealTickInput {
  /** 本 tick 时该消息的完整 content 长度（不是增量）。 */
  incomingLength: number;
  /** 系统或应用级降级开关：为 true 时一步释放全部字符。 */
  reduceMotion?: boolean;
}

/**
 * 把任意数值安全化成长度：非有限值视为 0，负数夹到 0，小数向下取整。
 * 存在的意义是让上游（网络事件、JSON 解析、IPC）的畸形数据不会污染游标。
 */
function toLength(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.floor(value));
}

function sanitizeConfig(config: RevealBudgetConfig): RevealBudgetConfig {
  return {
    charsPerTick: Math.max(1, toLength(config.charsPerTick)),
    backlogThreshold: toLength(config.backlogThreshold),
    backlogDivisor: Math.max(1, toLength(config.backlogDivisor)),
    stallTicks: Math.max(1, toLength(config.stallTicks))
  };
}

/** 本 tick 的释放配额（含积压加速）。导出以便单测直接断言配额曲线。 */
export function revealQuota(backlog: number, config: RevealBudgetConfig = DEFAULT_REVEAL_BUDGET): number {
  const cfg = sanitizeConfig(config);
  const safeBacklog = toLength(backlog);
  if (safeBacklog <= 0) return 0;
  const bonus =
    safeBacklog > cfg.backlogThreshold ? Math.floor(safeBacklog / cfg.backlogDivisor) : 0;
  return cfg.charsPerTick + bonus;
}

/**
 * 推进一帧。
 *
 * 不变量（单测锁定）：
 * 1. `0 <= next.visible <= incomingLength`，且 `next.visible >= state.visible`
 *    除非 content **有限且变短**（此时夹到新长度，不抛错）。
 * 2. `incomingLength` 非有限（NaN / ±Infinity）视为「长度不可知」：整帧不推进，
 *    游标与停滞计数原样返回。**绝不把已显示正文回退为 0。**
 * 3. `reduceMotion` 为 true 时同步收敛到 `incomingLength`。
 * 4. 同一 `incomingLength` 连续传入超过 `stallTicks` 次 → 一步收敛（防无限落后）。
 */
export function stepReveal(
  state: RevealState,
  input: RevealTickInput,
  config: RevealBudgetConfig = DEFAULT_REVEAL_BUDGET
): RevealState {
  const cfg = sanitizeConfig(config);
  const visibleSafe = Math.max(0, Math.floor(Number.isFinite(state.visible) ? state.visible : 0));

  // 长度不可知或非法（NaN / ±Infinity / 负数，常见于上游序列化损坏、事件字段缺失）时
  // **不得回退游标**。若把非法长度当成 0，已显示的正文会整段消失再重打一遍 ——
  // 用户看到的是闪断，比"这一帧不推进"严重得多。故保持游标不动，等下一次合法长度到达。
  if (!Number.isFinite(input.incomingLength) || input.incomingLength < 0) {
    return {
      visible: visibleSafe,
      lastIncomingLength: state.lastIncomingLength,
      stallTicks: state.stallTicks
    };
  }

  const incoming = Math.max(0, Math.floor(input.incomingLength));

  // content 变短（重放 / reset / 服务端截断）：游标跟随收缩，不产生越界。
  const visible = Math.min(visibleSafe, incoming);

  if (input.reduceMotion === true) {
    return { visible: incoming, lastIncomingLength: incoming, stallTicks: 0 };
  }

  // 已追平：只更新观测值，不动游标。
  if (visible >= incoming) {
    return { visible, lastIncomingLength: incoming, stallTicks: 0 };
  }

  const grew = incoming !== state.lastIncomingLength;
  const stallTicks = grew ? 0 : toLength(state.stallTicks) + 1;

  if (stallTicks >= cfg.stallTicks) {
    return { visible: incoming, lastIncomingLength: incoming, stallTicks: 0 };
  }

  const backlog = incoming - visible;
  const next = visible + revealQuota(backlog, cfg);

  return {
    visible: Math.min(next, incoming),
    lastIncomingLength: incoming,
    stallTicks
  };
}

/** 便捷选择器：从完整 content 取出当前可见前缀。 */
export function visiblePrefix(content: string, state: RevealState): string {
  const limit = Math.min(toLength(state.visible), content.length);
  return content.slice(0, limit);
}

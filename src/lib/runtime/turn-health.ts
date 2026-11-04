/**
 * C2（P1-2 收尾）：轮次健康度文案（纯函数，不碰状态机、不发请求）。
 *
 * 数据来源：会话 trace 记录 `TurnTraceRecord.degraded` / `.toolFailures`（后端由 OTel 根 span
 * 同一计数器回填：`agent.turn.tool_failures` / `agent.turn.degraded`）。
 *
 * 语义（按优先级，最多一条，避免 UI 噪音）：
 * 1. `degraded === true` ⇒ 「降级」（本轮有工具失败但仍正常收尾）；
 * 2. 未降级但 `toolFailures > 0` ⇒ 「工具失败 N」；
 * 3. 其余（false / 0 / 缺失）⇒ `null`（不渲染任何徽标）。
 * 4. 畸形输入（`null`、字段类型错、负数）⇒ `null`，**绝不 throw**。
 */

/** 只读被测字段的窄视图（允许任意对象，字段类型在运行时校验）。 */
export type TurnHealthInput = {
  degraded?: unknown;
  toolFailures?: unknown;
};

/** 「降级」标签（含契约要求的「降级」字样）。 */
const DEGRADED_LABEL = "降级";

function readToolFailures(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return 0;
  }
  return Math.floor(value);
}

/**
 * 计算轮次健康度短标签。
 *
 * @param turn `TurnTraceRecord`（或任意形状的畸形对象）
 * @returns 中文短标签；无健康度问题时返回 `null`（调用方据此不渲染）。
 */
export function turnHealthLabel(turn: TurnHealthInput | null | undefined): string | null {
  if (turn === null || typeof turn !== "object" || Array.isArray(turn)) {
    return null;
  }
  const source = turn as TurnHealthInput;
  if (source.degraded === true) {
    return DEGRADED_LABEL;
  }
  const failures = readToolFailures(source.toolFailures);
  if (failures > 0) {
    return `工具失败 ${failures}`;
  }
  return null;
}

/**
 * 健康度徽标的悬浮说明（`title`）：给出**可行动**的解释，而不是重复标签文本。
 *
 * @param turn 同上
 * @returns 非空说明；无健康度问题时返回 `null`。
 */
export function turnHealthTitle(turn: TurnHealthInput | null | undefined): string | null {
  if (turn === null || typeof turn !== "object" || Array.isArray(turn)) {
    return null;
  }
  const source = turn as TurnHealthInput;
  const failures = readToolFailures(source.toolFailures);
  if (source.degraded === true) {
    return failures > 0
      ? `本轮有 ${failures} 次工具失败，属降级完成（答复已给出，但部分工具调用失败）`
      : "本轮属降级完成（部分工具调用失败，但答复已给出）";
  }
  if (failures > 0) {
    return `本轮出现 ${failures} 次工具失败`;
  }
  return null;
}

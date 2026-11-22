// D3 轮次执行与流式传输（20 键 / 5 getters / 14 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { TurnExecutionActions, TurnExecutionGetters } from "./types";

/** D3 轮次执行与流式传输 getters（键清单见 `types.ts` 的 `TurnExecutionGetters` 与事实图 §2）。 */
export const turnExecutionGetters = {} as TurnExecutionGetters;

/** D3 轮次执行与流式传输 actions（键清单见 `types.ts` 的 `TurnExecutionActions` 与事实图 §3）。 */
export const turnExecutionActions = {} as TurnExecutionActions;

// D6 历史/分支/检查点（13 键 / 2 getters / 4 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { HistoryActions, HistoryGetters } from "./types";

/** D6 历史/分支/检查点 getters（键清单见 `types.ts` 的 `HistoryGetters` 与事实图 §2）。 */
export const historyGetters = {} as HistoryGetters;

/** D6 历史/分支/检查点 actions（键清单见 `types.ts` 的 `HistoryActions` 与事实图 §3）。 */
export const historyActions = {} as HistoryActions;

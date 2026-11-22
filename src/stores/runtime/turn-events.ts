// turn 事件订阅聚合（0 键 / 0 getters / 3 actions，事实图 §5）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { TurnEventsActions, TurnEventsGetters } from "./types";

/** turn 事件订阅聚合 getters（键清单见 `types.ts` 的 `TurnEventsGetters` 与事实图 §2）。 */
export const turnEventsGetters = {} as TurnEventsGetters;

/** turn 事件订阅聚合 actions（键清单见 `types.ts` 的 `TurnEventsActions` 与事实图 §3）。 */
export const turnEventsActions = {} as TurnEventsActions;

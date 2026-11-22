// 跨域事务入口（0 键 / 0 getters / 21 actions，事实图 §4）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { OrchestrationActions, OrchestrationGetters } from "./types";

/** 跨域事务入口 getters（键清单见 `types.ts` 的 `OrchestrationGetters` 与事实图 §2）。 */
export const orchestrationGetters = {} as OrchestrationGetters;

/** 跨域事务入口 actions（键清单见 `types.ts` 的 `OrchestrationActions` 与事实图 §3）。 */
export const orchestrationActions = {} as OrchestrationActions;

// D2 工作区树（5 键 / 0 getters / 7 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { WorkspaceActions, WorkspaceGetters } from "./types";

/** D2 工作区树 getters（键清单见 `types.ts` 的 `WorkspaceGetters` 与事实图 §2）。 */
export const workspaceGetters = {} as WorkspaceGetters;

/** D2 工作区树 actions（键清单见 `types.ts` 的 `WorkspaceActions` 与事实图 §3）。 */
export const workspaceActions = {} as WorkspaceActions;

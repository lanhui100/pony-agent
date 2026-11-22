// D1 会话目录与生命周期（11 键 / 2 getters / 9 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { SessionCatalogActions, SessionCatalogGetters } from "./types";

/** D1 会话目录与生命周期 getters（键清单见 `types.ts` 的 `SessionCatalogGetters` 与事实图 §2）。 */
export const sessionCatalogGetters = {} as SessionCatalogGetters;

/** D1 会话目录与生命周期 actions（键清单见 `types.ts` 的 `SessionCatalogActions` 与事实图 §3）。 */
export const sessionCatalogActions = {} as SessionCatalogActions;

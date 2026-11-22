// D8 健康与能力目录（4 键 / 0 getters / 5 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { CatalogActions, CatalogGetters } from "./types";

/** D8 健康与能力目录 getters（键清单见 `types.ts` 的 `CatalogGetters` 与事实图 §2）。 */
export const catalogGetters = {} as CatalogGetters;

/** D8 健康与能力目录 actions（键清单见 `types.ts` 的 `CatalogActions` 与事实图 §3）。 */
export const catalogActions = {} as CatalogActions;

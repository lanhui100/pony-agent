// D5 遥测/指标/Provider 标注（18 键 / 9 getters / 13 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { TelemetryActions, TelemetryGetters } from "./types";

/** D5 遥测/指标/Provider 标注 getters（键清单见 `types.ts` 的 `TelemetryGetters` 与事实图 §2）。 */
export const telemetryGetters = {} as TelemetryGetters;

/** D5 遥测/指标/Provider 标注 actions（键清单见 `types.ts` 的 `TelemetryActions` 与事实图 §3）。 */
export const telemetryActions = {} as TelemetryActions;

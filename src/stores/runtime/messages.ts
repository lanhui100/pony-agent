// D4 消息与输入（10 键 / 0 getters / 14 actions）
//
// Wave 1 契约骨架：**只**导出类型化的空对象，供 `src/stores/runtime.ts` 装配壳 spread。
// 实现搬运（逐字、语义零改动）由对应域的 Executor 完成；禁止在此阶段写任何业务逻辑。
import type { MessagesActions, MessagesGetters } from "./types";

/** D4 消息与输入 getters（键清单见 `types.ts` 的 `MessagesGetters` 与事实图 §2）。 */
export const messagesGetters = {} as MessagesGetters;

/** D4 消息与输入 actions（键清单见 `types.ts` 的 `MessagesActions` 与事实图 §3）。 */
export const messagesActions = {} as MessagesActions;

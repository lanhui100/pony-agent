// runtime store 装配壳（Wave 1）。
//
// 职责边界（严格）：
//   1. 唯一的 `defineStore("runtime", …)` 调用点：state / getters / actions 全部来自
//      `./runtime/*` 各域模块的 spread；
//   2. 保留 store id `runtime`、导出名 `useRuntimeStore`、HMR dispose 钩子；
//   3. 不得在此文件写任何业务逻辑（实现归各域模块的 Executor）。
//
// 消费方契约：14 个消费方只 `import { useRuntimeStore }`，本文件的 store 门面类型由
// `./runtime/types.ts` 的 `RuntimeStore` 定义，键/签名冻结断言在 types.ts 内编译期校验。
import { defineStore } from "pinia";
import { debugLog } from "@/lib/runtime/utils";
import { createInitialState } from "./runtime/state";
import { catalogActions, catalogGetters } from "./runtime/catalog";
import { historyActions, historyGetters } from "./runtime/history";
import { messagesActions, messagesGetters } from "./runtime/messages";
import { orchestrationActions, orchestrationGetters } from "./runtime/orchestration";
import { queueActions, queueGetters } from "./runtime/queue";
import { sessionCatalogActions, sessionCatalogGetters } from "./runtime/session-catalog";
import { telemetryActions, telemetryGetters } from "./runtime/telemetry";
import { turnEventsActions, turnEventsGetters } from "./runtime/turn-events";
import { turnExecutionActions, turnExecutionGetters } from "./runtime/turn-execution";
import { workspaceActions, workspaceGetters } from "./runtime/workspace";

export const useRuntimeStore = defineStore("runtime", {
  state: createInitialState,
  getters: {
    ...sessionCatalogGetters,
    ...workspaceGetters,
    ...turnExecutionGetters,
    ...messagesGetters,
    ...telemetryGetters,
    ...historyGetters,
    ...queueGetters,
    ...catalogGetters,
    ...orchestrationGetters,
    ...turnEventsGetters
  },
  actions: {
    ...sessionCatalogActions,
    ...workspaceActions,
    ...turnExecutionActions,
    ...messagesActions,
    ...telemetryActions,
    ...historyActions,
    ...queueActions,
    ...catalogActions,
    ...orchestrationActions,
    ...turnEventsActions
  }
});

// F5b：HMR 重建本模块前释放已注册的 turn 事件监听器（否则热更后监听器成倍泄漏）。
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    try {
      void useRuntimeStore().disposeTurnEvents();
    } catch (error) {
      // 无 active pinia（单测/SSR 场景）：没有监听器需要清理。
      debugLog("events:hmr-dispose:skipped", { error: String(error) });
    }
  });
}

// 消费方依赖的类型再导出（保持原 `src/stores/runtime.ts` 的公开类型面）。
export type {
  ActiveStreamKind,
  TurnEventRegistry,
  RuntimeStore,
  RuntimeStoreActions,
  RuntimeStoreGetters,
  RuntimeStoreState,
  RuntimeStoreThis,
  TurnRetryProgress
} from "./runtime/types";
export type { AgentRuntimeStatus } from "@/lib/runtime/status";
export type { TurnRetryHint } from "@/lib/runtime/turn-retry";
export type { PendingAttachment, PendingAttachmentStatus } from "@/lib/runtime/file-attachments";

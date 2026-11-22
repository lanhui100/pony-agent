// D7 队列与授权策略（3 键 / 0 getters / 7 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**（唯一改动：import 路径按新层级调整、
// action 签名补 `this: RuntimeStoreThis` 形参），语义零改动。键清单与签名真源见 `./types.ts`。
import type { QueueDeliveryMode, QueuedMessageItem, ToolAuthorizationMode } from "@/types/runtime";
import type { QueueActions, QueueGetters, RuntimeStoreThis } from "./types";

/** D7 队列与授权策略 getters（本域 0 条，见事实图 §2）。 */
export const queueGetters = {} as QueueGetters;

/** D7 队列与授权策略 actions（键清单见 `types.ts` 的 `QueueActions` 与事实图 §3）。 */
export const queueActions = {
    // ── Queue & Steer (主会话排队与插队消息状态机) ──
    enqueueMessage(this: RuntimeStoreThis, content: string, mode?: QueueDeliveryMode) {
      const trimmed = content.trim();
      if (!trimmed) return;
      const targetMode = mode ?? this.queueDeliveryMode;
      const id = `queued-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      const item: QueuedMessageItem = {
        id,
        sessionId: this.sessionId,
        content: trimmed,
        mode: targetMode,
        createdAt: Date.now()
      };
      if (targetMode === "steer") {
        this.pendingQueuedMessages.unshift(item);
      } else {
        this.pendingQueuedMessages.push(item);
      }
    },
    promoteToSteer(this: RuntimeStoreThis, id: string) {
      const idx = this.pendingQueuedMessages.findIndex((m: QueuedMessageItem) => m.id === id);
      if (idx !== -1) {
        const item = this.pendingQueuedMessages[idx];
        item.mode = "steer";
        if (idx > 0) {
          this.pendingQueuedMessages.splice(idx, 1);
          this.pendingQueuedMessages.unshift(item);
        }
      }
    },
    removeQueuedMessage(this: RuntimeStoreThis, id: string) {
      const idx = this.pendingQueuedMessages.findIndex((m: QueuedMessageItem) => m.id === id);
      if (idx !== -1) {
        this.pendingQueuedMessages.splice(idx, 1);
      }
    },
    toggleQueueDeliveryMode(this: RuntimeStoreThis) {
      this.queueDeliveryMode = this.queueDeliveryMode === "queue" ? "steer" : "queue";
    },
    setQueueDeliveryMode(this: RuntimeStoreThis, mode: QueueDeliveryMode) {
      this.queueDeliveryMode = mode;
    },
    setToolAuthorizationMode(this: RuntimeStoreThis, mode: ToolAuthorizationMode) {
      this.toolAuthorizationMode = mode;
    },
    dequeueNextMessage(this: RuntimeStoreThis): QueuedMessageItem | null {
      return this.pendingQueuedMessages.shift() ?? null;
    },
} as QueueActions;

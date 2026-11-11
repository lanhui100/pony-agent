// cancelled-turn domain：取消回合哨兵句常量与展示层清洗工具（PA-118）。
//
// 后端在 turn:cancelled 事件的 text 字段下发哨兵句，并将其作为取消回合历史消息
// 内容持久化（历史分类依赖字符串相等的哨兵句）。此处仅做桌面端展示层过滤：
// 不改变 Rust 后端、不改变历史语义、不改变 trace/timeline 文本。
export const CANCELLED_TURN_MESSAGE = "用户终止，发送消息可继续。";

/** 去除首尾空白后若与取消回合哨兵句完全相等，则返回空串；否则原样返回。 */
export function stripCancelledTurnSentinel(content: string): string {
  if (content.trim() === CANCELLED_TURN_MESSAGE) {
    return "";
  }
  return content;
}

/** 后端中断检查点的文本标记（`session::interrupted_turn_checkpoint_text` 的固定首行）。 */
export const INTERRUPTED_CHECKPOINT_MARKER = "[中断轮次检查点]";

/** 取消终态的一行状态（面向用户；过程清单不进信息流）。 */
export const INTERRUPTED_CANCELLED_LINE = "上一轮已停止，发送消息可继续。";

/** 上游中断/崩溃收口的一行状态（面向用户；步骤明细由 trace 工具行承载）。 */
export const INTERRUPTED_FAILED_LINE = "上一轮中断，已保留已执行的步骤记录。";

/**
 * 合成中断记录的一行状态文案（L2，dsh 对齐）。
 *
 * 判定顺序：结构化 `kind` **优先**；旧会话无 kind ⇒ 回退 `content` 标记嗅探。
 * 返回 `null` 表示「这是模型真实正文」（取消哨兵句仍按 PA-118 口径清洗为空串）。
 */
export function interruptedRecordStatusLine(
  content: string,
  kind?: "cancelled" | "interrupted" | null
): string | null {
  if (kind === "cancelled") {
    return INTERRUPTED_CANCELLED_LINE;
  }
  if (kind === "interrupted") {
    return INTERRUPTED_FAILED_LINE;
  }
  if (content.trim().startsWith(INTERRUPTED_CHECKPOINT_MARKER)) {
    return INTERRUPTED_FAILED_LINE;
  }
  return null;
}
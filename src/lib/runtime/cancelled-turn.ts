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
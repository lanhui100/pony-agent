// L2（dsh 对齐）：合成中断记录（取消/失败轮）在信息流里只出一行状态，
// 绝不把「模型上下文载体」（正文 + 已执行步骤清单）当成 assistant 正文直出。
//
// 判定真值源：`src/lib/runtime/cancelled-turn.ts::interruptedRecordStatusLine`
// ——结构化 `kind` 优先，旧会话回退后端检查点标记。
import { describe, expect, it } from "vitest";

import {
  DEGRADED_TURN_LINE,
  INTERRUPTED_CANCELLED_LINE,
  INTERRUPTED_ERROR_LINE,
  INTERRUPTED_FAILED_LINE,
  interruptedRecordStatusLine
} from "@/lib/runtime/cancelled-turn";
import { hydrateMessagesFromHistory } from "@/lib/runtime/messages";

/** 后端 `session::interrupted_turn_checkpoint_text` 的真实形态。 */
const CHECKPOINT_TEXT = [
  "[中断轮次检查点]",
  "已执行的工具步骤：",
  "- Run 检查 social dev 三层健康状态",
  "- Run 检查端口监听与进程"
].join("\n");

describe("interruptedRecordStatusLine", () => {
  it("kind 优先：cancelled / interrupted 各给一句状态", () => {
    expect(interruptedRecordStatusLine("任意落库文本", "cancelled")).toBe(
      INTERRUPTED_CANCELLED_LINE
    );
    expect(interruptedRecordStatusLine("任意落库文本", "interrupted")).toBe(
      INTERRUPTED_FAILED_LINE
    );
  });

  it("旧会话回退标记嗅探；普通正文不得改写成状态行", () => {
    expect(interruptedRecordStatusLine(CHECKPOINT_TEXT, null)).toBe(
      INTERRUPTED_FAILED_LINE
    );
    expect(interruptedRecordStatusLine(CHECKPOINT_TEXT, undefined)).toBe(
      INTERRUPTED_FAILED_LINE
    );
    expect(interruptedRecordStatusLine("正常的模型回答", null)).toBeNull();
    expect(interruptedRecordStatusLine("正常的模型回答", "unknown" as never)).toBeNull();
  });

  it("含标记但前缀不同的正文不得误判（前缀判定，不是包含判定）", () => {
    expect(
      interruptedRecordStatusLine(`前置说明\n${CHECKPOINT_TEXT}`, null)
    ).toBeNull();
  });

  // B275：桌面端此前 kind 联合类型缺 "error"，同一份历史在 TUI 说「因错误终止」、
  // 在桌面端降级成「已中断」——同一条记录两种说法。四态必须逐一对齐 Rust 真源。
  it("四种终态各给一句不同的话（与 Rust errors.rs 逐字一致）", () => {
    expect(interruptedRecordStatusLine("任意落库文本", "cancelled")).toBe(
      INTERRUPTED_CANCELLED_LINE
    );
    expect(interruptedRecordStatusLine("任意落库文本", "interrupted")).toBe(
      INTERRUPTED_FAILED_LINE
    );
    expect(interruptedRecordStatusLine("任意落库文本", "error")).toBe(
      INTERRUPTED_ERROR_LINE
    );
    expect(interruptedRecordStatusLine("任意落库文本", "degraded")).toBe(
      DEGRADED_TURN_LINE
    );

    // 四句必须两两不同：任一撞词就说明两种终态又被合并了。
    const lines = [
      INTERRUPTED_CANCELLED_LINE,
      INTERRUPTED_FAILED_LINE,
      INTERRUPTED_ERROR_LINE,
      DEGRADED_TURN_LINE
    ];
    expect(new Set(lines).size).toBe(4);
  });

  // B274：降级轮是「跑完了、降级交付」，措辞不得自称中断/失败（否则用户会去重跑）。
  it("降级文案不自称中断或失败", () => {
    expect(DEGRADED_TURN_LINE).not.toContain("中断");
    expect(DEGRADED_TURN_LINE).not.toContain("失败");
  });
});

describe("hydrateMessagesFromHistory 对合成中断记录的处理", () => {
  it("kind 行只渲染一行状态，步骤清单不进入信息流", () => {
    const messages = hydrateMessagesFromHistory([
      { role: "user", content: "检查 social dev", turnId: "turn-1" },
      {
        role: "assistant",
        content: CHECKPOINT_TEXT,
        turnId: "turn-1",
        kind: "cancelled"
      }
    ]);

    const assistant = messages.find((message) => message.role === "assistant");
    expect(assistant?.content).toBe(INTERRUPTED_CANCELLED_LINE);
    expect(
      messages.some((message) => message.content.includes("Run 检查端口监听与进程"))
    ).toBe(false);
  });

  it("旧会话（无 kind）同样收敛为一行状态", () => {
    const messages = hydrateMessagesFromHistory([
      { role: "user", content: "检查 social dev", turnId: "turn-1" },
      { role: "assistant", content: CHECKPOINT_TEXT, turnId: "turn-1" }
    ]);

    const assistant = messages.find((message) => message.role === "assistant");
    expect(assistant?.content).toBe(INTERRUPTED_FAILED_LINE);
  });

  it("普通模型回答逐字保真（不得误伤）", () => {
    const messages = hydrateMessagesFromHistory([
      { role: "user", content: "你好", turnId: "turn-1" },
      { role: "assistant", content: "你好，有什么可以帮你？", turnId: "turn-1" }
    ]);

    expect(messages.find((message) => message.role === "assistant")?.content).toBe(
      "你好，有什么可以帮你？"
    );
  });
});

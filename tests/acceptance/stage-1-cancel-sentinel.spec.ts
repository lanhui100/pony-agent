import { describe, expect, it } from "vitest";
import { CANCELLED_TURN_MESSAGE, stripCancelledTurnSentinel } from "@/lib/runtime/cancelled-turn";
import { hydrateMessagesFromHistory } from "@/lib/runtime/messages";
import type { TurnHistoryMessage } from "@/types/runtime";

describe("Stage 1 - Cancel Sentinel Acceptance", () => {
  it("F1-1: exports CANCELLED_TURN_MESSAGE sentinel and stripCancelledTurnSentinel strips it properly", () => {
    expect(CANCELLED_TURN_MESSAGE).toBe("用户终止，发送消息可继续。");
    expect(stripCancelledTurnSentinel("用户终止，发送消息可继续。")).toBe("");
    expect(stripCancelledTurnSentinel("  用户终止，发送消息可继续。  ")).toBe("");
    expect(stripCancelledTurnSentinel("用户终止，发送消息可继续。\n")).toBe("");
    expect(stripCancelledTurnSentinel("正常内容")).toBe("正常内容");
    expect(stripCancelledTurnSentinel("")).toBe("");
  });

  it("F1-3: hydrateMessagesFromHistory cleans up cancelled turn sentinel content to empty string", () => {
    const history: TurnHistoryMessage[] = [
      {
        role: "user",
        content: "你好"
      },
      {
        role: "assistant",
        content: "用户终止，发送消息可继续。"
      }
    ];

    const hydrated = hydrateMessagesFromHistory(history);
    expect(hydrated).toHaveLength(2);
    expect(hydrated[0].content).toBe("你好");
    expect(hydrated[1].content).toBe("");
  });

  it("F1-3: hydrateMessagesFromHistory preserves regular assistant content", () => {
    const history: TurnHistoryMessage[] = [
      {
        role: "user",
        content: "你好"
      },
      {
        role: "assistant",
        content: "你好！有什么我可以帮你的？"
      }
    ];

    const hydrated = hydrateMessagesFromHistory(history);
    expect(hydrated).toHaveLength(2);
    expect(hydrated[1].content).toBe("你好！有什么我可以帮你的？");
  });
});

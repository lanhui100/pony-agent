import { describe, expect, it } from "vitest";
import { formatTurnSummary, clampText } from "@/components/chat/turn-dial-utils";
import type { TurnBucket } from "@/components/chat/WorkspaceTurnItem.vue";

describe("TurnDial Utils", () => {
  it("clampText truncates long strings with ellipsis", () => {
    expect(clampText("hello world", 5)).toBe("hello…");
    expect(clampText("hello", 5)).toBe("hello");
    expect(clampText("   hi   \n  there  ", 20)).toBe("hi there");
  });

  it("formatTurnSummary extracts prompt and response preview from a TurnBucket", () => {
    const turn: TurnBucket = {
      turnId: "turn-1",
      user: {
        id: "msg-1",
        role: "user",
        content: "What is the status of our deployment pipeline?",
        turnId: "turn-1"
      },
      assistant: {
        id: "msg-2",
        role: "assistant",
        content: "The deployment pipeline completed successfully without errors.",
        turnId: "turn-1"
      },
      tools: [],
      mergedTools: []
    };

    const summary = formatTurnSummary(turn, 1);
    expect(summary.turnNumber).toBe(1);
    expect(summary.prompt).toBe("What is the status of our deployment pipeline?");
    expect(summary.response).toBe("The deployment pipeline completed successfully without errors.");
  });

  it("formatTurnSummary handles empty or missing assistant gracefully", () => {
    const turn: TurnBucket = {
      turnId: "turn-2",
      user: {
        id: "msg-3",
        role: "user",
        content: "Hello assistant",
        turnId: "turn-2"
      },
      assistant: null,
      tools: [],
      mergedTools: []
    };

    const summary = formatTurnSummary(turn, 2);
    expect(summary.turnNumber).toBe(2);
    expect(summary.prompt).toBe("Hello assistant");
    expect(summary.response).toBe("");
  });
});

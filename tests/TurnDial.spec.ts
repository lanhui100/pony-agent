import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TurnDial from "@/components/chat/TurnDial.vue";
import { formatTurnSummary, clampText } from "@/components/chat/turn-dial-utils";
import type { TurnBucket } from "@/components/chat/WorkspaceTurnItem.vue";

describe("TurnDial Component", () => {
  const turns: TurnBucket[] = [
    {
      turnId: "turn-1",
      user: { id: "u1", role: "user", content: "Hi", turnId: "turn-1" },
      assistant: { id: "a1", role: "assistant", content: "Hello", turnId: "turn-1" },
      tools: [],
      mergedTools: []
    },
    {
      turnId: "turn-2",
      user: { id: "u2", role: "user", content: "How are you?", turnId: "turn-2" },
      assistant: { id: "a2", role: "assistant", content: "I am good", turnId: "turn-2" },
      tools: [],
      mergedTools: []
    }
  ];

  it("applies turn-dial-stick class when stickToSidebar prop is true", () => {
    const wrapper = mount(TurnDial, {
      props: {
        turns,
        activeTurnId: "turn-1",
        stickToSidebar: true
      }
    });

    const nav = wrapper.find("nav.turn-dial-root");
    expect(nav.exists()).toBe(true);
    expect(nav.classes()).toContain("turn-dial-stick");
  });

  it("omits turn-dial-stick class when stickToSidebar prop is false", () => {
    const wrapper = mount(TurnDial, {
      props: {
        turns,
        activeTurnId: "turn-1",
        stickToSidebar: false
      }
    });

    const nav = wrapper.find("nav.turn-dial-root");
    expect(nav.exists()).toBe(true);
    expect(nav.classes()).not.toContain("turn-dial-stick");
  });
});

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

import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import WorkspaceTurnItem, { type TurnBucket, type AgentTurnEvent } from "@/components/chat/WorkspaceTurnItem.vue";
import { createPinia, setActivePinia } from "pinia";

function createTurnBucket(): TurnBucket {
  return {
    turnId: "turn-test",
    user: null,
    assistant: {
      id: "assistant-1",
      turnId: "turn-test",
      role: "assistant",
      content: "测试回复正文",
      status: "done"
    },
    tools: [],
    mergedTools: []
  };
}

describe("WorkspaceTurnItem Layout and Alignment", () => {
  it("renders reasoning block with refined border-l guide line and removes isolated pl-5 indent", () => {
    const pinia = createPinia();
    setActivePinia(pinia);

    const events: AgentTurnEvent[] = [
      {
        kind: "reasoning",
        key: "reasoning-1",
        order: 1,
        assistant: createTurnBucket().assistant!,
        reasoningContent: "这是思考过程内容",
        streaming: false
      }
    ];

    const wrapper = mount(WorkspaceTurnItem, {
      props: {
        turn: createTurnBucket(),
        events,
        shouldShowAgentArticle: true,
        rollbackInFlight: false,
        confirmRollback: () => {},
        setUserMessageRef: () => {},
        setAgentMessageRef: () => {},
        handleMarkdownRenderComplete: () => {},
        shouldUseMarkdownAssistantRendering: () => false,
        shouldUseOptimizedAssistantStreaming: () => false,
        streamingReasoningStable: (r) => r,
        streamingReasoningFade: () => "",
        assistantDisplayedReasoningFadeStyle: () => undefined,
        assistantDisplayedReasoningFadeKey: () => 0
      },
      global: {
        plugins: [pinia],
        directives: {
          motion: () => {}
        }
      }
    });

    const reasoningPanel = wrapper.find(".conversation-reasoning-panel");
    expect(reasoningPanel.exists()).toBe(true);

    const reasoningBody = reasoningPanel.find(".conversation-reasoning-body");
    expect(reasoningBody.exists()).toBe(true);
    // 验证去除了孤立的 pl-5，改用了带有引导线的布局
    expect(reasoningBody.classes()).not.toContain("pl-5");
    expect(reasoningBody.classes()).toContain("border-l-2");
    expect(reasoningBody.classes()).toContain("pl-3");

    wrapper.unmount();
  });
});

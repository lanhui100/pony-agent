import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import WorkspaceTurnItem, {
  type AgentTurnEvent,
  type MergedToolCall,
  type TurnBucket
} from "@/components/chat/WorkspaceTurnItem.vue";

describe("Workspace Multi-Hop Flow and Discrete Sequencing", () => {
  it("renders distinct thinking blocks and individual tool cards strictly in sequence without merging", () => {
    const pinia = createPinia();
    setActivePinia(pinia);

    // 模拟 DeepSeek 多 hop 运行场景：
    // Hop 0: 思考过程 1 -> 连续调用两次同名工具 read_file
    // Hop 1: 思考过程 2 -> 调用工具 run_command
    // Hop 2: 最终正文回答
    const tools: MergedToolCall[] = [
      {
        id: "tool-turn-1-act-1",
        toolName: "read_file",
        canonicalToolName: "Read",
        displayNameZh: "读取文件",
        mergeKey: "read_file",
        description: "读取 package.json",
        status: "done",
        durationSeconds: 0.2,
        count: 1
      },
      {
        id: "tool-turn-1-act-2",
        toolName: "read_file",
        canonicalToolName: "Read",
        displayNameZh: "读取文件",
        mergeKey: "read_file",
        description: "读取 Cargo.toml",
        status: "done",
        durationSeconds: 0.3,
        count: 1
      },
      {
        id: "tool-turn-1-act-3",
        toolName: "run_command",
        canonicalToolName: "Run",
        displayNameZh: "执行命令",
        mergeKey: "run_command",
        description: "cargo check",
        status: "done",
        durationSeconds: 1.2,
        count: 1
      }
    ];

    const turnBucket: TurnBucket = {
      turnId: "turn-1",
      user: {
        id: "msg-user-1",
        turnId: "turn-1",
        role: "user",
        content: "检查依赖与编译"
      },
      assistant: {
        id: "msg-assistant-1",
        turnId: "turn-1",
        role: "assistant",
        content: "检查完毕，所有配置一致且编译正常。",
        status: "done"
      },
      tools: [],
      mergedTools: tools
    };

    // 事件流完全去除了合并，逐条按照时序排序
    const events: AgentTurnEvent[] = [
      {
        kind: "reasoning",
        key: "reasoning-msg-assistant-1",
        order: 1.8,
        assistant: turnBucket.assistant!,
        reasoningContent: "第一阶段：分析项目结构并读取文件清单...",
        streaming: false
      },
      {
        kind: "tools",
        key: "tools-tool-turn-1-act-1",
        order: 2.1,
        tools: [tools[0]!]
      },
      {
        kind: "tools",
        key: "tools-tool-turn-1-act-2",
        order: 2.2,
        tools: [tools[1]!]
      },
      {
        kind: "reasoning",
        key: "reasoning-msg-assistant-1-hop-1-model-2",
        order: 3.8,
        assistant: turnBucket.assistant!,
        reasoningContent: "第二阶段：文件已读取，准备进行编译检查...",
        streaming: false
      },
      {
        kind: "tools",
        key: "tools-tool-turn-1-act-3",
        order: 4.1,
        tools: [tools[2]!]
      },
      {
        kind: "content",
        key: "content-msg-assistant-1-hop-2-model-3",
        order: 5.0,
        assistant: turnBucket.assistant!,
        content: "检查完毕，所有配置一致且编译正常。",
        streaming: false
      }
    ];

    const wrapper = mount(WorkspaceTurnItem, {
      props: {
        turn: turnBucket,
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
        directives: { motion: () => {} }
      }
    });

    // 1. 验证思考过程没有被合并为一个，而是渲染了两个独立的思考展开卡片
    const reasoningPanels = wrapper.findAll(".conversation-reasoning-panel");
    expect(reasoningPanels).toHaveLength(2);
    expect(reasoningPanels[0]!.text()).toContain("第一阶段：分析项目结构并读取文件清单");
    expect(reasoningPanels[1]!.text()).toContain("第二阶段：文件已读取，准备进行编译检查");

    // 2. 验证工具调用没有被合并或折叠（没有出现 count > 1 如 '2x'），每个工具独立渲染
    const toolPanels = wrapper.findAll(".conversation-tool-panel");
    expect(toolPanels).toHaveLength(3);
    expect(toolPanels[0]!.text()).toContain("读取 package.json");
    expect(toolPanels[1]!.text()).toContain("读取 Cargo.toml");
    expect(toolPanels[2]!.text()).toContain("cargo check");
    expect(wrapper.text()).not.toContain("2x");

    // 3. 验证 DOM 节点的物理出现顺序严格遵循时间线交替时序
    const agentShell = wrapper.find(".conversation-agent-shell");
    expect(agentShell.exists()).toBe(true);

    const childElements = agentShell.element.children;
    // 排除最后的 agent-action-bar
    const eventTags = Array.from(childElements)
      .slice(0, 6)
      .map((el) => {
        if (el.classList.contains("conversation-reasoning-panel")) return "reasoning";
        if (el.classList.contains("conversation-tool-panel")) return "tools";
        if (el.classList.contains("assistant-response-panel")) return "content";
        return el.className;
      });

    expect(eventTags).toEqual([
      "reasoning",
      "tools",
      "tools",
      "reasoning",
      "tools",
      "content"
    ]);

    wrapper.unmount();
  });
});
